//! Small text/layout helpers shared by the transcript renderer and pickers.

use firment_core::ThinkingLevel;
use ratatui::layout::{Constraint, Layout, Rect};
use std::path::Path;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// The card's progress line: the phase, and the numbers only where the tool knows them.
///
/// A percentage the tool cannot know is worse than no percentage (see
/// [`firment_core::progress`]), so `current`/`total` render only when the event carried a real
/// total and the remaining time only when the tool could extrapolate one.
pub(crate) fn format_progress(event: &firment_core::progress::ProgressEvent) -> String {
    let mut text = event.phase.clone();
    if event.total > 0 && event.current <= event.total {
        text.push_str(&format!(" {}%", event.current * 100 / event.total));
    }
    if let Some(eta) = event.eta_ms {
        text.push_str(&format!(
            " · ~{} left",
            crate::step_time::format_step_duration(std::time::Duration::from_millis(eta))
        ));
    }
    text
}

/// Branch + working-tree change count shown in the status bar.
pub(crate) struct GitInfo {
    pub(crate) branch: String,
    pub(crate) changes: usize,
    /// The changed paths, repo-relative and `/`-separated, for the file rail to
    /// mark. Kept alongside the count rather than instead of it: the status bar
    /// wants a number, the rail wants names.
    pub(crate) changed: Vec<String>,
}

/// The paths `git status --porcelain` reports.
///
/// Two shapes to know about: a rename is `XY <old> -> <new>` -- the new name is
/// the one that exists on disk, so that is the one worth marking -- and git
/// quotes a path containing unusual characters, which is unquoted here without
/// trying to decode the escapes. A path the rail could not match would simply
/// go unmarked, which is the harmless direction for this to fail in.
pub(crate) fn parse_porcelain(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.len() > 3)
        .map(|line| line[3..].trim())
        .map(|path| path.rsplit(" -> ").next().unwrap_or(path))
        .map(|path| path.trim_matches('"').to_string())
        .collect()
}

pub(crate) fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for raw_line in text.split('\n') {
        if width == 0 {
            out.push(raw_line.to_string());
            continue;
        }
        let mut current = String::new();
        let mut current_w = 0;
        for ch in raw_line.chars() {
            let w = ch.width().unwrap_or(0);
            if current_w + w > width && !current.is_empty() {
                out.push(std::mem::take(&mut current));
                current_w = 0;
            }
            current.push(ch);
            current_w += w;
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

pub(crate) fn truncate_chars(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

/// Whether a stored/delivered tool output carries a unified diff (the edit
/// tools prepend a one-line "Edited …" header, so an `@@` header is the
/// signal). Non-diff bodies are not worth a card body — the summary already
/// says what happened.
pub(crate) fn is_diff_body(detail: &str) -> bool {
    // The rule lives in core: the CLI's `firm review last` and the self-review prompt
    // ask the same question, and a heuristic copied per surface is a heuristic that
    // drifts (2026-09-18).
    firment_core::review::self_review::looks_like_diff(detail)
}

/// A diff small enough to show without being asked (the breathing-LED case in
/// the TUI mockup is three changed lines: one `-` line plus two `+`). Larger
/// diffs collapse to the summary and wait for the expand key.
///
/// The `--- `/`+++ ` FILE HEADERS must not count: they are two extra lines on
/// every diff, and counting them made a one-line edit look like four changes.
pub(crate) fn diff_is_small(detail: &str) -> bool {
    detail
        .lines()
        .filter(|l| {
            (l.starts_with('-') && !l.starts_with("---"))
                || (l.starts_with('+') && !l.starts_with("+++"))
        })
        .count()
        <= 4
}

pub(crate) fn truncate_tail(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in text.chars().rev() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.insert(0, ch);
        w += cw;
    }
    format!("…{out}")
}

/// Human-readable in-progress hint for a running tool, e.g. "searching" or
/// "flashing target/out.elf…". Falls back to the raw tool name.
pub(crate) fn tool_activity(name: &str, args: &serde_json::Value) -> String {
    let label = match name {
        "grep" | "glob" | "symbols" | "list_dir" => "searching",
        "read_file" => "reading",
        "edit_file" | "write_file" => "editing",
        "build" => "building",
        "flash" => "flashing",
        "run" => "running target",
        "monitor" => "monitoring serial",
        "la" => "capturing logic",
        "verify" => "verifying",
        "shell" => "running shell command",
        other => other,
    };
    let target = ["file", "path", "pattern"]
        .iter()
        .find_map(|key| args.get(*key).and_then(|v| v.as_str()))
        .and_then(|s| {
            let base = s.rsplit(['/', '\\']).next().unwrap_or(s);
            if base.is_empty() {
                None
            } else {
                Some(base.to_string())
            }
        });
    match target {
        Some(target) => format!("{label} {target}…"),
        None => format!("{label}…"),
    }
}

pub(crate) fn next_thinking(level: ThinkingLevel) -> ThinkingLevel {
    match level {
        ThinkingLevel::Off => ThinkingLevel::Low,
        ThinkingLevel::Low => ThinkingLevel::Medium,
        ThinkingLevel::Medium => ThinkingLevel::High,
        ThinkingLevel::High => ThinkingLevel::XHigh,
        ThinkingLevel::XHigh => ThinkingLevel::Max,
        ThinkingLevel::Max => ThinkingLevel::Off,
    }
}

pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    let horizontal = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1]);
    horizontal[1]
}

pub(crate) fn format_ts(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| secs.to_string())
}

pub(crate) fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| anyhow::anyhow!("{e}"))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

/// Display width of `text` in terminal cells (CJK chars count as 2).
pub(crate) fn cell_width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Character index whose starting cell is at or after `cell` (0-based cells).
pub(crate) fn char_index_at_cell(text: &str, cell: usize) -> usize {
    let mut width = 0usize;
    for (idx, ch) in text.chars().enumerate() {
        if width >= cell {
            return idx;
        }
        width += ch.width().unwrap_or(0);
    }
    text.chars().count()
}

/// Find a subslice in a char slice (starting at `from`); returns the start
/// index.
pub(crate) fn find_subslice(haystack: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    let from = from.min(haystack.len());
    (from..=haystack.len().saturating_sub(needle.len()))
        .find(|&i| haystack[i..i + needle.len()] == *needle)
}

/// Branch + working-tree change count for the status bar. Returns None
/// outside a git repository or when git is unavailable — the bar just omits
/// the segment.
pub(crate) async fn git_info(cwd: &Path) -> Option<GitInfo> {
    // branch --show-current also resolves the unborn branch name of a fresh
    // repo (rev-parse fails before the first commit).
    let branch = tokio::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(cwd)
        .output()
        .await
        .ok()?;
    if !branch.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&branch.stdout).trim().to_string();
    let status = tokio::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(cwd)
        .output()
        .await
        .ok()?;
    let stdout = String::from_utf8_lossy(&status.stdout);
    let changed = parse_porcelain(&stdout);
    Some(GitInfo {
        branch,
        changes: changed.len(),
        changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_progress_line_shows_only_the_numbers_the_tool_knows() {
        // The phase half is always safe to print; the count only when the tool carried one.
        assert_eq!(
            format_progress(&firment_core::progress::ProgressEvent::phase("erasing")),
            "erasing"
        );
        let line = format_progress(&firment_core::progress::ProgressEvent::counted(
            "capturing",
            50,
            200,
            10_000,
        ));
        assert!(
            line.contains("capturing") && line.contains("25%") && line.contains("left"),
            "{line}"
        );
    }

    #[test]
    fn porcelain_paths_survive_the_two_column_prefix() {
        let text = " M src/main.c\n?? notes.txt\nA  include/pwm.h\n";
        assert_eq!(
            parse_porcelain(text),
            vec!["src/main.c", "notes.txt", "include/pwm.h"]
        );
    }

    #[test]
    fn a_rename_is_marked_under_the_name_that_exists() {
        // `R  old -> new`: only `new` is on disk, so only `new` can be shown.
        assert_eq!(
            parse_porcelain("R  src/old.c -> src/new.c"),
            vec!["src/new.c"]
        );
    }

    #[test]
    fn a_quoted_path_is_unquoted() {
        assert_eq!(parse_porcelain("?? \"a file.txt\""), vec!["a file.txt"]);
    }

    #[test]
    fn an_empty_status_is_no_paths() {
        assert!(parse_porcelain("").is_empty());
    }
}
