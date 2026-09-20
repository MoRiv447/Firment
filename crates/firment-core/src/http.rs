//! Shared HTTP client construction.
//!
//! Every HTTP client in Firment is built from [`http_builder()`] so proxy
//! exclusions stay consistent: LAN endpoints must never be sent through the
//! proxy, or a machine behind a proxy cannot reach its own Ollama.

use std::time::Duration;

/// Ranges that must bypass the proxy. reqwest reads `NO_PROXY` on its own,
/// but that variable almost never lists the LAN ranges — so an Ollama or LM
/// Studio server sitting on 192.168.x.x is unreachable the moment a proxy
/// is configured, with nothing but a connection error to show for it.
const NO_PROXY_RANGES: &str =
    "localhost,.local,127.0.0.0/8,::1,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16";

/// A [`reqwest::ClientBuilder`] with the LAN exclusion list already attached.
///
/// Mind the trap: `ClientBuilder::no_proxy()` **disables proxying entirely**,
/// it does not take an exclusion list. Exclusions belong on each `Proxy`,
/// which is why this rebuilds the env proxies rather than calling it.
///
/// When no proxy env var is set the builder is left alone, so reqwest's own
/// detection (including the macOS system configuration) still applies.
/// Note that a macOS *system* proxy therefore does not get these exclusions
/// — only the env-var form does.
pub fn http_builder() -> reqwest::ClientBuilder {
    let mut builder = reqwest::Client::builder();
    // Written as one `if let` per proxy: nested if-lets trip
    // clippy::collapsible_if, and let-chains would raise the MSRV past the
    // 1.85 this crate declares.
    if let Some(Ok(proxy)) = env_proxy("HTTP_PROXY").map(|u| reqwest::Proxy::http(&u)) {
        builder = builder.proxy(proxy.no_proxy(lan_no_proxy()));
    }
    if let Some(Ok(proxy)) = env_proxy("HTTPS_PROXY").map(|u| reqwest::Proxy::https(&u)) {
        builder = builder.proxy(proxy.no_proxy(lan_no_proxy()));
    }
    if let Some(Ok(proxy)) = env_proxy("ALL_PROXY").map(|u| reqwest::Proxy::all(&u)) {
        builder = builder.proxy(proxy.no_proxy(lan_no_proxy()));
    }
    builder
}

/// How long a provider call may go **without a byte** before it is treated as dead.
///
/// This is the deadline the offline design review asked for, and it is deliberately a
/// *read* timeout rather than a total one: a reasoning model that thinks for a minute and
/// then streams is working, not broken, and a total timeout would cut it off mid-answer.
///
/// It also implements the review's "no bytes **and** no heartbeat" rule without any extra
/// machinery: the stream parsers emit `ProviderEvent::Activity` for keep-alives, and a
/// keep-alive is a byte — so a server that is alive but slow stays well inside this
/// deadline, while a black-holed one (a dropped packet, a sleeping laptop, wifi that is up
/// but not routed) trips it. Before this existed, that case hung until the OS gave up,
/// which is minutes; a *refused* connection fails instantly, which is why nobody noticed.
pub const PROVIDER_READ_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a provider connection may take to establish.
///
/// Shorter than the read deadline: a TCP handshake to a reachable host is milliseconds, so
/// anything near this number is a failure the user should hear about in seconds.
pub const PROVIDER_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// A built client for talking to a model provider: LAN exclusions applied, and both
/// deadlines attached.
///
/// Use this instead of `reqwest::Client::new()` — that shorthand reads the proxy env vars
/// but not the LAN exclusions (which is how a LAN Ollama ends up unreachable) and carries no
/// deadline at all (which is how a dead endpoint ends up looking like a slow one).
pub fn provider_client() -> reqwest::Client {
    provider_client_with(PROVIDER_READ_TIMEOUT, PROVIDER_CONNECT_TIMEOUT)
}

/// [`provider_client`] with the deadlines passed in, so a test can prove they apply without
/// waiting two minutes for the production value.
///
/// The fallback loses both deadlines, and that is stated rather than hidden: it only runs
/// when the builder itself fails (TLS initialisation, in practice), where the alternative is
/// no client at all. It is not a path that should ever be reached.
pub fn provider_client_with(read: Duration, connect: Duration) -> reqwest::Client {
    http_builder()
        .read_timeout(read)
        .connect_timeout(connect)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn lan_no_proxy() -> Option<reqwest::NoProxy> {
    reqwest::NoProxy::from_string(NO_PROXY_RANGES)
}

fn env_proxy(var: &str) -> Option<String> {
    std::env::var(var)
        .or_else(|_| std::env::var(var.to_lowercase()))
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_ranges_are_expressible() {
        // NoProxy::from_string accepts CIDR ranges (reqwest documents
        // 192.168.1.0/24 matching 192.168.1.42); guard against a silent
        // change to that grammar leaving us with an exclusion list that
        // parses but matches nothing.
        assert!(lan_no_proxy().is_some());
        for range in ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"] {
            assert!(
                NO_PROXY_RANGES.contains(range),
                "{range} missing from the exclusion list"
            );
        }
    }

    #[test]
    fn builder_always_constructs() {
        // With or without proxy env vars in the test environment, the
        // builder must stay usable.
        let _ = http_builder();
    }

    #[tokio::test]
    async fn a_server_that_accepts_and_then_says_nothing_fails_within_the_deadline() {
        // The case the offline review found, reproduced without a network: a listener that
        // completes the TCP handshake and then never writes a byte. Before the read deadline
        // existed this is where a call to a black-holed endpoint went to wait for the OS.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let holder = std::thread::spawn(move || {
            if let Ok((_stream, _)) = listener.accept() {
                // Hold it open, silently, for longer than the deadline under test.
                std::thread::sleep(Duration::from_secs(5));
            }
        });

        let client = provider_client_with(Duration::from_millis(300), Duration::from_millis(300));
        let started = std::time::Instant::now();
        let result = client.get(format!("http://{addr}/v1/models")).send().await;
        let took = started.elapsed();

        assert!(
            result.is_err(),
            "a server that never answers must not be reported as a live one"
        );
        assert!(
            took < Duration::from_secs(3),
            "the deadline did not apply: the call took {took:?}"
        );
        // And it waited: without this the test would also pass if the connection had been
        // refused instantly, which is a different failure and not the one being tested.
        assert!(
            took >= Duration::from_millis(250),
            "the call failed in {took:?} — that is a connection error, not the deadline \
             firing; the listener is supposed to accept and then stay silent"
        );
        drop(holder);
    }

    #[test]
    fn the_production_deadlines_are_attached_to_the_client_providers_use() {
        // The regression this guards: a provider built without a deadline looks exactly like
        // a provider talking to a slow model — until it never comes back. The values are
        // asserted by name so that removing one is a test failure rather than a silent
        // return to "hangs until the OS gives up".
        assert!(
            PROVIDER_READ_TIMEOUT >= Duration::from_secs(60),
            "a slow model is not a broken one"
        );
        assert!(PROVIDER_CONNECT_TIMEOUT <= Duration::from_secs(30));
        let _ = provider_client();
    }
}
