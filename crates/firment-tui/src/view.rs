//! Transcript rendering: turns `App` state into ratatui widgets. Split from
//! `app.rs` so state transitions and drawing evolve independently; this impl
//! block only reads state (plus the selection highlighter used by the rows).

use crate::MAX_INPUT_HEIGHT;
use crate::app::{App, Item};
use crate::util::{
    GitInfo, cell_width, centered_rect, char_index_at_cell, format_ts, truncate_chars,
    truncate_tail, wrap_text,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

/// Spinner glyphs shared by the tool cards, the thinking row and the status
/// bar. The phase comes from `App::spinner_frame` (wall clock, 120ms/step).
const SPINNER: [char; 4] = ['◐', '◓', '◑', '◒'];

impl App {
    /// Constant-rate spinner phase, derived from wall clock: deriving it
    /// from the draw count made the rotation strobe during token bursts
    /// (one frame per delta) and crawl during silence.
    pub(crate) fn spinner_frame(&self) -> usize {
        (self.started.elapsed().as_millis() as usize / 120) % SPINNER.len()
    }

    /// Rect of the currently open modal, if any. Single source for both the
    /// scrim (dim everything OUTSIDE this rect) and the dialog itself so the
    /// two can never drift apart. Keep the percentages here only.
    fn modal_area(&self, frame: Rect) -> Option<Rect> {
        if self.question.is_some() {
            Some(centered_rect(68, 42, frame))
        } else if self.model_picker.is_some() {
            Some(centered_rect(60, 48, frame))
        } else if self.session_picker.is_some() {
            Some(centered_rect(76, 52, frame))
        } else {
            None
        }
    }

    /// Dim every cell outside `keep`: the transcript kept rendering at full
    /// brightness around a modal (concurrent tool waves guarantee events
    /// arrive behind an open ask_user), which read as corrupted output.
    fn apply_scrim(&self, frame: &mut Frame, keep: Rect) {
        let area = frame.area();
        for y in 0..area.height {
            for x in 0..area.width {
                let inside = x >= keep.x
                    && x < keep.x.saturating_add(keep.width)
                    && y >= keep.y
                    && y < keep.y.saturating_add(keep.height);
                if inside {
                    continue;
                }
                if let Some(cell) = frame.buffer_mut().cell_mut(Position { x, y }) {
                    cell.set_style(cell.style().add_modifier(Modifier::DIM));
                }
            }
        }
    }

    pub(crate) fn highlight_selection(&self, rows: &mut Vec<Line<'static>>) {
        let Some(selection) = self.selection else {
            return;
        };
        let ((r0, c0), (r1, c1)) = selection.normalized();
        for row_idx in r0..=r1 {
            let Some(row) = rows.get_mut(row_idx) else {
                break;
            };
            let (start, end) = if r0 == r1 {
                (c0, c1)
            } else if row_idx == r0 {
                (c0, usize::MAX)
            } else if row_idx == r1 {
                (0, c1)
            } else {
                (0, usize::MAX)
            };
            let mut col = 0usize;
            let mut new_spans = Vec::new();
            for span in std::mem::take(&mut row.spans) {
                let content: String = span.content.into_owned();
                let span_start = col;
                let span_end = col + cell_width(&content);
                let sel_start = span_start.max(start);
                let sel_end = span_end.min(end);
                if sel_start < sel_end {
                    let char_start = char_index_at_cell(&content, sel_start - span_start);
                    let char_end = char_index_at_cell(&content, sel_end - span_start);
                    let before: String = content.chars().take(char_start).collect();
                    let selected: String = content
                        .chars()
                        .skip(char_start)
                        .take(char_end.saturating_sub(char_start))
                        .collect();
                    let after: String = content.chars().skip(char_end).collect();
                    if !before.is_empty() {
                        new_spans.push(Span::styled(before, span.style));
                    }
                    new_spans.push(Span::styled(
                        selected,
                        span.style.add_modifier(Modifier::REVERSED),
                    ));
                    if !after.is_empty() {
                        new_spans.push(Span::styled(after, span.style));
                    }
                } else {
                    new_spans.push(Span::styled(content, span.style));
                }
                col = span_end;
            }
            row.spans = new_spans;
        }
    }

    /// Whether this item's wrapped rows change on their own and must be
    /// re-wrapped every frame (never cached).
    fn is_row_dynamic(&self, idx: usize, item: &Item) -> bool {
        match item {
            // The running spinner glyph is baked into the wrapped line.
            Item::Tool { running: true, .. } => true,
            // The streaming assistant message grows on every batch.
            Item::Assistant(_) => idx + 1 == self.items.len(),
            _ => false,
        }
    }

    pub(crate) fn render_rows(&mut self, width: usize) -> Vec<Line<'static>> {
        // Wrap cache: keyed by (mutation version, width, item count). The
        // old code re-wrapped the ENTIRE transcript character-by-character
        // on every draw — including idle animation frames — so long
        // sessions spent most of their frame budget in wrap_text.
        let cache_hit = self
            .row_cache
            .as_ref()
            .is_some_and(|(version, cached_width, rows)| {
                *version == self.row_version
                    && *cached_width == width
                    && rows.len() == self.items.len()
            });
        if !cache_hit {
            self.row_cache = Some((self.row_version, width, vec![None; self.items.len()]));
        }
        let mut rows = Vec::new();
        for (idx, item) in self.items.iter().enumerate() {
            let dynamic = self.is_row_dynamic(idx, item);
            if !dynamic
                && let Some(Some(cached)) = self.row_cache.as_ref().and_then(|c| c.2.get(idx))
            {
                rows.extend(cached.iter().cloned());
                continue;
            }
            let wrapped = self.render_item(item, width);
            if !dynamic
                && let Some((version, cached_width, cache)) = self.row_cache.as_mut()
                && *version == self.row_version
                && *cached_width == width
            {
                cache[idx] = Some(wrapped.clone());
            }
            rows.extend(wrapped);
        }
        rows
    }

    fn render_item(&self, item: &Item, width: usize) -> Vec<Line<'static>> {
        let mut rows = Vec::new();
        match item {
            Item::User(text) => {
                let wrapped = wrap_text(text, width.saturating_sub(2));
                for (idx, seg) in wrapped.iter().enumerate() {
                    if idx == 0 {
                        rows.push(Line::from(vec![
                            Span::styled(
                                "❯ ",
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                seg.clone(),
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                    } else {
                        rows.push(Line::from(Span::styled(
                            seg.clone(),
                            Style::default().fg(Color::Cyan),
                        )));
                    }
                }
                rows.push(Line::from(""));
            }
            Item::Assistant(text) => {
                for seg in wrap_text(text, width.saturating_sub(1)) {
                    rows.push(Line::from(Span::styled(
                        seg,
                        Style::default().fg(Color::LightGreen),
                    )));
                }
                rows.push(Line::from(""));
            }
            Item::Tool {
                name,
                seq: _,
                running,
                ok,
                summary,
            } => {
                // Finished cards dim into the background: the eye should go
                // to what is RUNNING, not to a wall of bright history.
                let dim = !running;
                let (symbol, color) = if *running {
                    (SPINNER[self.spinner_frame()], Color::Yellow)
                } else if *ok {
                    ('✓', Color::Green)
                } else {
                    ('✗', Color::Red)
                };
                let style = if dim {
                    Style::default().fg(color).add_modifier(Modifier::DIM)
                } else {
                    Style::default().fg(color)
                };
                let line = format!("{symbol} {name}  {}", truncate_chars(summary, 140));
                for seg in wrap_text(&line, width.saturating_sub(1)) {
                    rows.push(Line::from(Span::styled(seg, style)));
                }
            }
            Item::Permission { tool, reason } => {
                rows.push(Line::from(Span::styled(
                    "⚠ Permission required",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                for seg in wrap_text(&format!("Tool: {tool}"), width.saturating_sub(1)) {
                    rows.push(Line::from(Span::styled(
                        seg,
                        Style::default().fg(Color::Yellow),
                    )));
                }
                for seg in wrap_text(&format!("Reason: {reason}"), width.saturating_sub(1)) {
                    rows.push(Line::from(Span::styled(
                        seg,
                        Style::default().fg(Color::White),
                    )));
                }
                rows.push(Line::from(Span::styled(
                    "[y] allow    [a] always allow for this session    [n] / Esc deny",
                    Style::default().fg(Color::Green),
                )));
                rows.push(Line::from(""));
            }
            Item::System(text) => {
                for seg in wrap_text(text, width.saturating_sub(1)) {
                    rows.push(Line::from(Span::styled(
                        seg,
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }
            Item::Error(text) => {
                for seg in wrap_text(&format!("⚠ {text}"), width.saturating_sub(1)) {
                    rows.push(Line::from(Span::styled(
                        seg,
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    )));
                }
            }
        }
        rows
    }

    pub(crate) fn render(&mut self, frame: &mut Frame) {
        let frame_width = frame.area().width.saturating_sub(2) as usize;
        let (input_lines, line_starts, cursor_line, cursor_col) = if self.input.is_empty() {
            (Vec::<String>::new(), Vec::new(), 0, 0)
        } else {
            self.input_layout(frame_width.max(1))
        };
        let input_height = (input_lines.len() + 2).clamp(3, MAX_INPUT_HEIGHT) as u16;
        let [transcript_area, status_area, input_area] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(input_height),
        ])
        .areas(frame.area());
        self.input_width = frame_width;
        self.input_rect = input_area;

        let content_width = transcript_area.width.saturating_sub(2) as usize;
        self.transcript_rect = transcript_area;
        self.content_width = content_width.max(1);
        let mut rows = self.render_rows(content_width.max(1));
        if self.ai_thinking {
            let ch = SPINNER[self.spinner_frame()];
            // Elapsed seconds make a long reasoning phase feel observed
            // instead of hung (reasoning can legitimately run for minutes).
            let secs = self
                .thinking_since
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0);
            rows.push(Line::from(Span::styled(
                format!(" {ch} thinking… {secs}s"),
                Style::default().fg(Color::Yellow),
            )));
        }
        let height = transcript_area.height.saturating_sub(2) as usize;
        let max_offset = rows.len().saturating_sub(height);
        self.max_offset = max_offset;
        self.scroll = self.scroll.min(max_offset);
        if self.scroll == 0 {
            self.follow = true;
        }
        let offset = if self.follow {
            max_offset
        } else {
            max_offset.saturating_sub(self.scroll)
        };
        self.highlight_selection(&mut rows);
        let title = if self.follow {
            " Firment ".to_string()
        } else {
            format!(" Firment · ↑ {} ", self.scroll)
        };
        let paragraph = Paragraph::new(rows).scroll((offset as u16, 0)).block(
            Block::bordered()
                .title(Span::styled(title, Style::default().fg(Color::Cyan)))
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        frame.render_widget(paragraph, transcript_area);

        let spinner = if self.busy {
            const SPINNER: [char; 4] = ['◐', '◓', '◑', '◒'];
            SPINNER[self.spinner_frame()].to_string()
        } else {
            "•".to_string()
        };
        let state = self.status_text();
        let mut cwd_str = self.cwd.display().to_string();
        if cwd_str.width() > 36 {
            cwd_str = format!("…{}", truncate_tail(&cwd_str, 35));
        }
        let git_str = match &self.git {
            Some(GitInfo { branch, changes }) if *changes > 0 => {
                format!(" git: {branch} · {changes}")
            }
            Some(GitInfo { branch, .. }) => format!(" git: {branch}"),
            None => String::new(),
        };
        let left = format!(
            " {} {}/{} · T:{} · {}{}  ",
            self.mode.label().to_uppercase(),
            self.provider,
            self.model,
            self.thinking.label(),
            cwd_str,
            git_str
        );
        let right = if self.interrupt_armed_at.is_some() {
            format!(" {} · {state} · ⏸ Esc again to interrupt ", spinner)
        } else if self.busy && !self.interrupting {
            format!(" {} · {state} · Esc×2 interrupt ", spinner)
        } else {
            format!(" {} · {state} ", spinner)
        };
        let pad = (status_area.width as usize).saturating_sub(left.width() + right.width());
        let status_line = Line::from(vec![
            Span::styled(left, Style::default().fg(Color::Cyan)),
            Span::raw(" ".repeat(pad)),
            Span::styled(right, Style::default().fg(Color::DarkGray)),
        ]);
        frame.render_widget(Paragraph::new(status_line), status_area);

        let visible_text_height = input_area.height.saturating_sub(2) as usize;
        let hidden_lines = input_lines.len().saturating_sub(visible_text_height.max(1));
        let collapsed_lines: usize = self
            .paste_blocks
            .iter()
            .map(|b| b.text.lines().count().max(1))
            .sum();
        let title = if hidden_lines > 0 {
            format!(" input · ↑{hidden_lines} lines hidden (Enter sends everything) ")
        } else if collapsed_lines > 0 {
            format!(" input · {collapsed_lines} lines collapsed (Enter sends full text) ")
        } else {
            " input ".to_string()
        };
        let block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .title(Span::styled(title, Style::default().fg(Color::Cyan)))
            .border_style(Style::default().fg(Color::DarkGray));
        let content = if self.input.is_empty() {
            self.input_scroll = 0;
            Paragraph::new(Line::from(Span::styled(
                "Type a message (or /help for commands & keys)",
                Style::default().fg(Color::DarkGray),
            )))
            .block(block)
        } else {
            let max_scroll = input_lines.len().saturating_sub(visible_text_height.max(1));
            if cursor_line < self.input_scroll {
                self.input_scroll = cursor_line;
            }
            if visible_text_height > 0 && cursor_line >= self.input_scroll + visible_text_height {
                self.input_scroll = cursor_line + 1 - visible_text_height;
            }
            self.input_scroll = self.input_scroll.min(max_scroll);
            let shown = input_lines
                .iter()
                .skip(self.input_scroll)
                .take(visible_text_height.max(1))
                .enumerate()
                .map(|(shown_idx, line)| {
                    let abs_line = self.input_scroll + shown_idx;
                    let line_start = line_starts.get(abs_line).copied().unwrap_or(0);
                    let line_len = line.chars().count();
                    let (sel_min, sel_max) = self
                        .input_sel
                        .map(|(a, b)| if a <= b { (a, b) } else { (b, a) })
                        .unwrap_or((0, 0));
                    let seg_start = sel_min.saturating_sub(line_start);
                    let seg_end = sel_max.saturating_sub(line_start).min(line_len);
                    if seg_start < seg_end {
                        let before: String = line.chars().take(seg_start).collect();
                        let selected: String = line
                            .chars()
                            .skip(seg_start)
                            .take(seg_end - seg_start)
                            .collect();
                        let after: String = line.chars().skip(seg_end).collect();
                        Line::from(vec![
                            Span::styled(before, Style::default().fg(Color::White)),
                            Span::styled(
                                selected,
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::REVERSED),
                            ),
                            Span::styled(after, Style::default().fg(Color::White)),
                        ])
                    } else {
                        Line::from(Span::styled(
                            line.clone(),
                            Style::default().fg(Color::White),
                        ))
                    }
                })
                .collect::<Vec<_>>();
            Paragraph::new(shown).block(block)
        };
        frame.render_widget(content, input_area);
        // Permission cards are inline now, so the input always keeps the
        // cursor; even with empty input, pin it to the input start so IME/first
        // chars are not drawn outside the box.
        let modal_open =
            self.model_picker.is_some() || self.session_picker.is_some() || self.question.is_some();
        if !modal_open {
            let cursor_x =
                (input_area.x + 1 + cursor_col as u16).min(input_area.right().saturating_sub(1));
            let cursor_y = input_area.y + 1 + cursor_line.saturating_sub(self.input_scroll) as u16;
            frame.set_cursor_position((cursor_x, cursor_y));
        }

        // Modal scrim (see modal_area/apply_scrim): dim the live transcript
        // around the dialog BEFORE drawing it — the dialog's Clear only
        // covers its own rect, and the transcript otherwise kept rendering
        // at full brightness on both sides.
        if let Some(keep) = self.modal_area(frame.area()) {
            self.apply_scrim(frame, keep);
        }

        // Modals render regardless of the (inline, non-modal) permission
        // card: the old `permission.is_none()` gate made the question dialog
        // VANISH whenever an approval was pending while its keys still
        // routed there — an invisible dialog swallowing keypresses.
        {
            if let Some(picker) = &self.model_picker {
                let area = centered_rect(60, 48, frame.area());
                frame.render_widget(Clear, area);
                let block = Block::bordered()
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .title(Span::styled(
                        " Model picker ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .border_style(Style::default().fg(Color::Cyan));
                frame.render_widget(block, area);
                let inner = area.inner(Margin {
                    horizontal: 2,
                    vertical: 1,
                });
                let mut lines = Vec::new();
                let query: String = picker.query.iter().collect();
                lines.push(Line::from(Span::styled(
                    format!("Filter: {query} (Enter select · Esc close)"),
                    Style::default().fg(Color::DarkGray),
                )));
                if picker.models.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "Fetching model list…",
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    let filtered = picker.filtered();
                    for (idx, model) in filtered.iter().take(12).enumerate() {
                        let (marker, style) = if idx == picker.selected {
                            (
                                "❯ ",
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            )
                        } else {
                            ("  ", Style::default().fg(Color::White))
                        };
                        lines.push(Line::from(Span::styled(format!("{marker}{model}"), style)));
                    }
                    if filtered.len() > 12 {
                        lines.push(Line::from(Span::styled(
                            format!("… {} more", filtered.len() - 12),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                }
                frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
            }

            if let Some(picker) = &self.session_picker {
                let area = centered_rect(76, 52, frame.area());
                frame.render_widget(Clear, area);
                let block = Block::bordered()
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .title(Span::styled(
                        " Session picker ",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .border_style(Style::default().fg(Color::Magenta));
                frame.render_widget(block, area);
                let inner = area.inner(Margin {
                    horizontal: 2,
                    vertical: 1,
                });
                let mut lines = Vec::new();
                let query: String = picker.query.iter().collect();
                lines.push(Line::from(Span::styled(
                    format!("Filter: {query} (↑/↓ select · Enter open · Esc close)"),
                    Style::default().fg(Color::DarkGray),
                )));
                if picker.sessions.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "Loading session list…",
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    let filtered = picker.filtered();
                    let start = picker.selected.saturating_sub(4);
                    for (idx, session) in filtered.iter().enumerate().skip(start).take(6) {
                        let (marker, row_style) = if idx == picker.selected {
                            (
                                "❯ ",
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            )
                        } else {
                            ("  ", Style::default().fg(Color::White))
                        };
                        let preview = truncate_chars(&session.preview, 42);
                        // Main line: timestamp + model + preview; full id on the
                        // next line in dim grey so it is easy to read and select.
                        lines.push(Line::from(Span::styled(
                            format!(
                                "{marker}{}  {:<22}  {}",
                                format_ts(session.updated_at),
                                session.model,
                                preview
                            ),
                            row_style,
                        )));
                        lines.push(Line::from(Span::styled(
                            format!("    id: {}", session.id),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                    if filtered.len() > 6 {
                        lines.push(Line::from(Span::styled(
                            format!("… {} more", filtered.len() - 6),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                    lines.push(Line::from(Span::styled(
                        "Keys: ↑/↓ select · Enter open · c copy id · d delete · Esc close",
                        Style::default().fg(Color::DarkGray),
                    )));
                }
                frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
            }

            if let Some(question) = &self.question {
                let area = centered_rect(68, 42, frame.area());
                frame.render_widget(Clear, area);
                let block = Block::bordered()
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .title(Span::styled(
                        " Question ",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .border_style(Style::default().fg(Color::Yellow));
                frame.render_widget(block, area);
                let inner = area.inner(Margin {
                    horizontal: 2,
                    vertical: 1,
                });
                let mut lines = Vec::new();
                for line in wrap_text(&question.question, inner.width as usize) {
                    lines.push(Line::from(Span::styled(
                        line,
                        Style::default().fg(Color::White),
                    )));
                }
                if !question.options.is_empty() {
                    lines.push(Line::default());
                    for (idx, option) in question.options.iter().enumerate() {
                        lines.push(Line::from(Span::styled(
                            format!("  {}  {option}", idx + 1),
                            Style::default().fg(Color::Cyan),
                        )));
                    }
                }
                lines.push(Line::default());
                let typed: String = self.question_input.iter().collect();
                lines.push(Line::from(vec![
                    Span::styled("Answer: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(typed, Style::default().fg(Color::White)),
                ]));
                lines.push(Line::from(Span::styled(
                    "1-9 pick an option (before typing) · type + Enter free answer · Esc dismiss",
                    Style::default().fg(Color::DarkGray),
                )));
                frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
            }
        }
    }

    /// Short status text shown in the status bar while the agent is running.
    pub(crate) fn status_text(&self) -> String {
        if self.permission.is_some() {
            "waiting for approval".to_string()
        } else if self.question.is_some() {
            "question".to_string()
        } else if self.interrupt_armed_at.is_some() {
            "Esc again to interrupt".to_string()
        } else if self.interrupting {
            "interrupting…".to_string()
        } else if self.ai_thinking {
            "thinking".to_string()
        } else if self.busy {
            if let Some((_, label)) = self.active_tools.last() {
                let count = if self.active_tools.len() > 1 {
                    format!("{}× ", self.active_tools.len())
                } else {
                    String::new()
                };
                format!("working · {count}{label}")
            } else {
                "working".to_string()
            }
        } else {
            "ready".to_string()
        }
    }
}
