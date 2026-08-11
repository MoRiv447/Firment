use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

/// Turn-level cooperative cancellation signal.
///
/// `cancel()` flips an atomic flag and wakes every waiter; `cancelled()`
/// resolves as soon as the flag is set. Cloning is cheap — all clones share
/// one flag. `reset()` clears the flag between turns so a new turn starts
/// un-cancelled.
#[derive(Clone, Debug, Default)]
pub struct Cancellable {
    inner: Arc<CancellableInner>,
}

#[derive(Debug, Default)]
struct CancellableInner {
    flag: AtomicBool,
    notify: Notify,
}

impl Cancellable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the cancellation flag and wake all waiters.
    pub fn cancel(&self) {
        if !self.inner.flag.swap(true, Ordering::SeqCst) {
            self.inner.notify.notify_waiters();
        }
    }

    /// Clear the cancellation flag (used between turns, never mid-turn).
    pub fn reset(&self) {
        self.inner.flag.store(false, Ordering::Release);
    }

    /// Whether cancellation was requested. Safe to call from sync code.
    pub fn is_cancelled(&self) -> bool {
        self.inner.flag.load(Ordering::Acquire)
    }

    /// Resolve when cancellation is requested. Returns immediately if the
    /// flag is already set. The wait is race-free: the future is registered
    /// before the flag is re-checked, so a `cancel()` racing with this call
    /// can never be missed.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let notified = self.inner.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_wakes_waiters() {
        let cancel = Cancellable::new();
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            c.cancel();
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), cancel.cancelled())
            .await
            .expect("cancelled() must resolve after cancel()");
        assert!(cancel.is_cancelled());
    }

    #[tokio::test]
    async fn cancelled_returns_immediately_when_already_set() {
        let cancel = Cancellable::new();
        cancel.cancel();
        tokio::time::timeout(std::time::Duration::from_millis(50), cancel.cancelled())
            .await
            .expect("must not wait when already cancelled");
    }

    #[tokio::test]
    async fn reset_clears_the_flag() {
        let cancel = Cancellable::new();
        cancel.cancel();
        cancel.reset();
        assert!(!cancel.is_cancelled());
        // A subsequent cancel still wakes late waiters.
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            c.cancel();
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), cancel.cancelled())
            .await
            .expect("re-cancel must wake new waiters");
    }

    #[test]
    fn clones_share_the_flag() {
        let cancel = Cancellable::new();
        let clone = cancel.clone();
        clone.cancel();
        assert!(cancel.is_cancelled());
    }
}
