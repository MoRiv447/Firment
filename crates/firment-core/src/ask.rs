use async_trait::async_trait;
use tokio::sync::oneshot;

/// A question forwarded to the UI (TUI modal or CLI stdin); the reply carries
/// the user's answer, or `None` when the user dismissed it.
#[derive(Debug)]
pub struct QuestionRequest {
    pub question: String,
    pub options: Vec<String>,
    pub reply: oneshot::Sender<Option<String>>,
}

/// Interactive user front-end used by the `ask_user` tool. The TUI shows a
/// modal with the question and answer options; the CLI reads a line from stdin.
#[async_trait]
pub trait Asker: Send + Sync {
    async fn ask(&self, question: &str, options: &[String]) -> Result<String, String>;
}
