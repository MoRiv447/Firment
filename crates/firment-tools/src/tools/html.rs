/// Minimal HTML-to-text helpers for the web tools: strip script/style blocks,
/// tags, and decode common entities.
pub(crate) fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut skip_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut lower = Vec::new();
    let mut chars = html.chars().peekable();
    let mut last_char = '\0';
    while let Some(ch) = chars.next() {
        if in_script || in_style {
            if ch == '<' {
                lower.clear();
                let mut closing = String::new();
                for c in chars.by_ref() {
                    if c == '>' {
                        break;
                    }
                    closing.push(c.to_ascii_lowercase());
                }
                if closing.contains("/script") {
                    in_script = false;
                } else if closing.contains("/style") {
                    in_style = false;
                }
            }
            continue;
        }
        match ch {
            '<' => {
                skip_tag = true;
                lower.clear();
            }
            '>' if skip_tag => {
                skip_tag = false;
                let tag: String = lower.iter().collect();
                let first = tag.split_whitespace().next().unwrap_or("");
                if first == "script" {
                    in_script = true;
                } else if first == "style" {
                    in_style = true;
                } else if matches!(
                    first,
                    "p" | "div" | "br" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "pre"
                ) && last_char != '\n'
                {
                    out.push('\n');
                }
            }
            '>' => {
                // stray '>'
                if last_char != ' ' && !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            ch if skip_tag => {
                lower.push(ch.to_ascii_lowercase());
            }
            ch => {
                out.push(ch);
                last_char = ch;
            }
        }
    }
    decode_entities(&out)
}

/// Decode the common HTML entities into plain text.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let end = tail.find(';').map(|i| i + 1).unwrap_or(tail.len());
        let entity = &tail[..end];
        let decoded = match entity {
            "&amp;" => "&".to_string(),
            "&lt;" => "<".to_string(),
            "&gt;" => ">".to_string(),
            "&quot;" => "\"".to_string(),
            "&#39;" | "&apos;" => "'".to_string(),
            "&nbsp;" => " ".to_string(),
            "&hellip;" => "…".to_string(),
            "&mdash;" => "—".to_string(),
            "&ndash;" => "–".to_string(),
            other => {
                if let Some(stripped) = other.strip_prefix("&#").and_then(|s| s.strip_suffix(';')) {
                    let code = if stripped.starts_with('x') || stripped.starts_with('X') {
                        u32::from_str_radix(&stripped[1..], 16).ok()
                    } else {
                        stripped.parse::<u32>().ok()
                    };
                    code.and_then(char::from_u32)
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| other.to_string())
                } else {
                    other.to_string()
                }
            }
        };
        out.push_str(&decoded);
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags_and_scripts() {
        let html = "<html><head><script>var x = 1;</script></head><body><h1>Title</h1><p>Hello <b>world</b></p></body></html>";
        let text = strip_html(html);
        assert!(text.contains("Title"), "got: {text}");
        assert!(text.contains("Hello world"), "got: {text}");
        assert!(!text.contains("script"), "got: {text}");
        assert!(!text.contains("var x"), "got: {text}");
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(
            decode_entities("a &amp; b &lt;c&gt; &quot;q&quot;"),
            "a & b <c> \"q\""
        );
        assert_eq!(decode_entities("&#65;&#x42;"), "AB");
    }

    #[test]
    fn bare_amp_hash_does_not_panic() {
        assert_eq!(decode_entities("a &# b &#x c"), "a &# b &#x c");
        assert_eq!(decode_entities("&#"), "&#");
    }
}
