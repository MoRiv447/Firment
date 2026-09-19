//! Session export (plan §5, item 7): a self-contained record of what happened.
//!
//! Two formats, one shape: a header, the conversation, the changes with their diffs, and
//! the event log's timeline. Markdown for a report or an issue; HTML that opens from the
//! filesystem with no network and no external asset — because a shared session is usually
//! shared *because* something went wrong, and a page that needs a CDN is a page that will
//! not render where it is needed.
//!
//! Three rules, and the first is not negotiable:
//!
//! * **Everything from the session is escaped.** The export contains file contents, shell
//!   output and model text — any of which can contain `<script>`, and an exported session
//!   is opened in a browser. The HTML path escapes every interpolated string, and a test
//!   pins it rather than trusting the reading.
//! * **Recognisable secrets are masked.** A session can contain a key someone pasted into
//!   a prompt. This cannot be a guarantee — nothing can — so it recognises the shapes
//!   tokens actually have and says in the document that it did only that.
//! * **No external references.** No stylesheet link, no script tag, no image URL: the file
//!   is the whole document.

use crate::eventlog::LogRecord;
use crate::session::Session;
use crate::types::ChatMessage;

/// What the export is made of.
pub struct ExportInput<'a> {
    pub session: &'a Session,
    /// The session's event log, oldest first. May be empty for a session older than the log.
    pub events: &'a [LogRecord],
}

/// How many characters of one message to include before summarising it.
///
/// Not a redaction: the point is that an export of a long session stays a document someone
/// can read, and a 200 KB tool output is a document it cannot be. The omission is stated in
/// the text, so nothing looks complete that is not.
const MESSAGE_LIMIT: usize = 4000;

/// One session as Markdown.
pub fn markdown(input: &ExportInput<'_>) -> String {
    let session = input.session;
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", mask(&session.title())));
    out.push_str(&format!(
        "- session `{}`\n- provider `{}`, model `{}`\n- {} message(s), {} change(s)\n- exported by Firment\n\n",
        mask(&session.id),
        session.provider,
        session.model,
        session.messages.len(),
        session.changes_at(session.messages.len()).len(),
    ));

    if !input.events.is_empty() {
        out.push_str("## Timeline\n\n```\n");
        for (index, record) in input.events.iter().enumerate() {
            out.push_str(&format!("{:>4}  {}\n", index + 1, mask(&record.display())));
        }
        out.push_str("```\n\n");
    }

    out.push_str("## Conversation\n\n");
    for message in &session.messages {
        out.push_str(&section(message));
    }

    let changes = session.changes_at(session.messages.len());
    if !changes.is_empty() {
        out.push_str("## Changes\n\n");
        for (index, tool, diff) in &changes {
            out.push_str(&format!(
                "### `{}` (message {})\n\n```diff\n{}\n```\n\n",
                tool,
                index,
                mask(diff)
            ));
        }
    }

    out.push_str(&note(session.messages.len()));
    out
}

/// One session as a self-contained HTML document.
pub fn html(input: &ExportInput<'_>) -> String {
    let session = input.session;
    let mut body = String::new();
    body.push_str(&format!("<h1>{}</h1>", masked_escaped(&session.title())));
    body.push_str(&format!(
        "<p class=\"meta\">session <code>{}</code> · provider <code>{}</code> · model <code>{}</code> · \
         {} messages · {} changes</p>",
        masked_escaped(&session.id),
        masked_escaped(&session.provider),
        masked_escaped(&session.model),
        session.messages.len(),
        session.changes_at(session.messages.len()).len(),
    ));

    if !input.events.is_empty() {
        body.push_str("<h2>Timeline</h2><pre class=\"log\">");
        for (index, record) in input.events.iter().enumerate() {
            body.push_str(&format!(
                "<span class=\"n\">{:>4}</span>  {}\n",
                index + 1,
                masked_escaped(&record.display())
            ));
        }
        body.push_str("</pre>");
    }

    body.push_str("<h2>Conversation</h2>");
    for message in &session.messages {
        body.push_str(&html_section(message));
    }

    let changes = session.changes_at(session.messages.len());
    if !changes.is_empty() {
        body.push_str("<h2>Changes</h2>");
        for (index, tool, diff) in &changes {
            body.push_str(&format!(
                "<h3><code>{}</code> (message {})</h3><pre class=\"diff\">{}</pre>",
                masked_escaped(tool),
                index,
                masked_escaped(diff)
            ));
        }
    }

    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>{title}</title>\n<style>{css}</style>\n</head>\n<body>\n{body}\n\
         <footer>{note}</footer>\n</body>\n</html>\n",
        title = masked_escaped(&session.title()),
        css = CSS,
        body = body,
        note = masked_escaped(&note(session.messages.len())),
    )
}

/// The line every export ends with, so a reader knows what was done to it.
fn note(message_count: usize) -> String {
    format!(
        "\n---\n\nExported by Firment. Secrets that match a known token shape are masked; \
         this is a best effort, not a guarantee — read before sharing. Messages longer than \
         {MESSAGE_LIMIT} characters are summarised (not truncated silently: the omission is \
         stated where it happens). {message_count} message(s) in this session.\n"
    )
}

const CSS: &str = "\
body{font:15px/1.5 system-ui,sans-serif;max-width:60rem;margin:2rem auto;padding:0 1rem;color:#1a1a1a}\
h1{font-size:1.6rem}h2{font-size:1.2rem;margin-top:2rem;border-bottom:1px solid #ddd}\
.meta,.meta code{color:#555;font-size:.9rem}\
pre{background:#f6f8fa;padding:.75rem;overflow-x:auto;border-radius:4px;font-size:.85rem}\
pre.diff{background:#f6f8fa;border-left:3px solid #d0d7de}\
.log .n{color:#999}\
.msg{margin:.75rem 0;padding:.5rem .75rem;border-radius:4px}\
.msg.user{background:#eef6ff}.msg.assistant{background:#f6f8fa}\
.msg.tool{background:#fff8e6;font-size:.9rem}\
.msg.system{background:#f0f0f0;color:#555;font-size:.9rem}\
.role{font-weight:600;font-size:.8rem;text-transform:uppercase;letter-spacing:.03em;color:#666}\
footer{color:#666;font-size:.85rem;margin-top:2rem}";

fn section(message: &ChatMessage) -> String {
    let (role, body) = parts(message);
    format!(
        "### {role}\n\n{}\n\n",
        indent(&bounded(&mask(&body)), role == "diff")
    )
}

fn html_section(message: &ChatMessage) -> String {
    let (role, body) = parts(message);
    let class = match role.as_str() {
        "user" => "user",
        "assistant" => "assistant",
        "diff" => "tool",
        _ => "tool",
    };
    let content = if role == "diff" {
        format!("<pre>{}</pre>", masked_escaped(&mask(&body)))
    } else {
        masked_escaped(&mask(&bounded(&body)))
    };
    format!(
        "<div class=\"msg {class}\"><div class=\"role\">{role}</div>{content}</div>",
        role = masked_escaped(if role == "diff" { "change" } else { &role }),
    )
}

/// `(role, text)`, with a diff-carrying tool result labelled as a change.
fn parts(message: &ChatMessage) -> (String, String) {
    match message {
        ChatMessage::User { content } => ("user".to_string(), content.clone()),
        ChatMessage::Assistant { content, .. } => ("assistant".to_string(), content.clone()),
        ChatMessage::Tool { name, content, .. } => {
            if crate::review::self_review::looks_like_diff(content) {
                ("diff".to_string(), content.clone())
            } else {
                (name.clone(), content.clone())
            }
        }
        ChatMessage::System { content } => ("system".to_string(), content.clone()),
    }
}

/// A message longer than the limit, summarised in a way that cannot be mistaken for the
/// whole thing.
fn bounded(text: &str) -> String {
    if text.chars().count() <= MESSAGE_LIMIT {
        return text.to_string();
    }
    let head: String = text.chars().take(MESSAGE_LIMIT).collect();
    format!(
        "{head}\n\n… [{} more character(s) omitted from this export]",
        text.chars().count() - MESSAGE_LIMIT
    )
}

fn indent(text: &str, fenced: bool) -> String {
    if fenced {
        text.to_string()
    } else {
        text.lines()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Mask, then escape — the order every value from the session goes through.
///
/// One helper rather than two calls at each site, because the defect this fixes was exactly
/// a site that got only one of them: the title was escaped and **not** masked, and the title
/// is the first user message — the most likely place for a pasted key to be. A test pins it.
fn masked_escaped(text: &str) -> String {
    escape(&mask(text))
}

/// HTML-escape. The one function in this module that must never be skipped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Mask the token shapes we can recognise.
///
/// Deliberately not "mask long random-looking strings": build logs are full of hashes and
/// commit ids, and a redactor that eats those makes the export useless while still missing
/// the key that was pasted with a prefix nobody has seen before. Prefixes and lengths cover
/// what tokens actually look like.
pub fn mask(text: &str) -> String {
    const PREFIXES: [&str; 7] = [
        "sk-",
        "sk_live_",
        "sk-ant-",
        "ghp_",
        "github_pat_",
        "xoxb-",
        "AIza",
    ];
    let mut out = String::with_capacity(text.len());
    for word in split_keeping_whitespace(text) {
        let trimmed = word.trim_start_matches(|c: char| !c.is_alphanumeric() && c != '_');
        let candidate = trimmed.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '-');
        let looks_like_key = PREFIXES
            .iter()
            .any(|prefix| candidate.starts_with(prefix) && candidate.len() >= prefix.len() + 12);
        if looks_like_key {
            // Keep the prefix: a reader needs to know *which* key leaked.
            let prefix = PREFIXES
                .iter()
                .find(|prefix| candidate.starts_with(**prefix))
                .copied()
                .unwrap_or("");
            out.push_str(prefix);
            out.push_str("***masked***");
            let tail = &candidate[prefix.len()..];
            if let Some(rest) = word.rsplit_once(tail) {
                out.push_str(rest.1);
            }
        } else {
            out.push_str(word);
        }
    }
    out
}

/// Split into words while keeping the whitespace, so rejoining is lossless.
fn split_keeping_whitespace(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut in_space: Option<bool> = None;
    for (index, ch) in text.char_indices() {
        let is_space = ch.is_whitespace();
        match in_space {
            Some(previous) if previous != is_space => {
                parts.push(&text[start..index]);
                start = index;
                in_space = Some(is_space);
            }
            None => in_space = Some(is_space),
            _ => {}
        }
    }
    if start < text.len() {
        parts.push(&text[start..]);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChatMessage;

    fn session_with(messages: Vec<ChatMessage>) -> Session {
        let mut session = Session::new(std::path::PathBuf::from("."), "p", "m");
        for message in messages {
            session.push(message);
        }
        session
    }

    fn diff_message() -> ChatMessage {
        ChatMessage::Tool {
            tool_call_id: "c1".to_string(),
            name: "edit_file".to_string(),
            content: "Edited a.c (1 lines -> 2 lines)\n@@ -1 +1,2 @@\n-old\n+new\n".to_string(),
        }
    }

    #[test]
    fn a_script_tag_in_the_session_never_reaches_the_browser_unescaped() {
        // The export contains file contents, shell output and model text, and it is opened
        // in a browser. This is the test that makes the escaping a fact rather than a
        // reading of the code.
        let session = session_with(vec![ChatMessage::User {
            content: "<script>alert('x')</script> & \"quoted\"".to_string(),
        }]);
        let page = html(&ExportInput {
            session: &session,
            events: &[],
        });
        assert!(!page.contains("<script>"), "a raw tag reached the page");
        assert!(page.contains("&lt;script&gt;"), "the tag should be escaped");
        assert!(page.contains("&amp;"), "an ampersand should be escaped");
    }

    #[test]
    fn both_formats_carry_the_conversation_the_changes_and_the_timeline() {
        let session = session_with(vec![
            ChatMessage::User {
                content: "please blink".to_string(),
            },
            ChatMessage::Assistant {
                content: "done".to_string(),
                tool_calls: Vec::new(),
                thinking_blocks: Vec::new(),
            },
            diff_message(),
        ]);
        let events = vec![LogRecord {
            at: 1000,
            kind: "tool_end".to_string(),
            summary: "#1 edit_file ok — Edited a.c".to_string(),
        }];
        let input = ExportInput {
            session: &session,
            events: &events,
        };

        let md = markdown(&input);
        assert!(md.contains("# please blink"), "{md}");
        assert!(md.contains("## Timeline"), "{md}");
        assert!(md.contains("tool_end"), "{md}");
        assert!(md.contains("```diff"), "{md}");
        assert!(md.contains("+new"), "{md}");

        let page = html(&input);
        assert!(page.starts_with("<!DOCTYPE html>"), "{page}");
        // Self-contained: no stylesheet link, no script, no external URL.
        assert!(
            !page.contains("<link"),
            "the page must not reference a stylesheet"
        );
        assert!(
            !page.contains("http://"),
            "the page must not reference a URL"
        );
        assert!(
            !page.contains("https://"),
            "the page must not reference a URL"
        );
        assert!(page.contains("please blink"), "{page}");
        assert!(page.contains("@@ -1 +1,2 @@"), "{page}");
    }

    #[test]
    fn a_pasted_token_is_masked_with_its_prefix_left_visible() {
        let session = session_with(vec![ChatMessage::User {
            content: "my key is sk-abcdefghijklmnopqrstuvwxyz and my hash is 9f8e7d6c5b4a39281706"
                .to_string(),
        }]);
        let md = markdown(&ExportInput {
            session: &session,
            events: &[],
        });
        assert!(md.contains("sk-***masked***"), "{md}");
        assert!(!md.contains("sk-abcdefghijklmnopqrstuvwxyz"), "{md}");
        // A hex blob is a hash or a commit id far more often than it is a secret, and a
        // redactor that eats those makes the export useless.
        assert!(md.contains("9f8e7d6c5b4a39281706"), "{md}");
        // The reader is told what was done, and told it is not a guarantee.
        assert!(md.contains("not a guarantee"), "{md}");
    }

    #[test]
    fn the_title_is_masked_too_because_the_title_is_the_first_prompt() {
        // The defect this pins: the body was masked and the title was not, and the title is
        // the first user message — the most likely place for a key to have been pasted.
        let session = session_with(vec![ChatMessage::User {
            content: "help me with sk-abcdefghijklmnop please".to_string(),
        }]);
        let page = html(&ExportInput {
            session: &session,
            events: &[],
        });
        assert!(!page.contains("sk-abcdefghijklmnop"), "{page}");
        // Three places carry that prompt: the title tag, the heading, and the message
        // itself. The point of the assertion is the first two — the ones a reader sees
        // before scrolling — so it checks them by name rather than by counting.
        assert!(
            page.contains("<title>help me with sk-***masked*** please</title>"),
            "{page}"
        );
        assert!(
            page.contains("<h1>help me with sk-***masked*** please</h1>"),
            "{page}"
        );
        assert_eq!(page.matches("sk-***masked***").count(), 3, "{page}");
    }

    #[test]
    fn a_long_message_is_summarised_and_says_so() {
        let long = "x".repeat(MESSAGE_LIMIT + 50);
        let session = session_with(vec![ChatMessage::User { content: long }]);
        let page = html(&ExportInput {
            session: &session,
            events: &[],
        });
        assert!(
            page.contains("50 more character(s) omitted"),
            "the omission is stated"
        );
    }

    #[test]
    fn an_empty_session_still_produces_a_document() {
        // A session with no messages is a session someone might still export (it has a
        // timeline), and the renderers must not produce a broken page for it.
        let session = Session::new(std::path::PathBuf::from("."), "p", "m");
        let page = html(&ExportInput {
            session: &session,
            events: &[],
        });
        assert!(page.contains("</html>"), "{page}");
    }
}
