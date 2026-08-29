//! The TUI application state: transcript items, input buffer, pickers,
//! permission/question modals and every state transition the event loop
//! triggers. Rendering lives in the same impl — the methods take `&mut self`
//! (or `&self`) and draw into the current frame.

use crate::adapters::PermissionRequest;
use crate::commands::AgentCmd;
use crate::paste::{EnterAction, PasteBlock, PasteBurst, PasteOut};
use crate::pickers::{ModelPicker, Selection, SessionPicker};
use crate::util::{
    GitInfo, cell_width, char_index_at_cell, copy_to_clipboard, find_subslice, next_thinking,
    tool_activity,
};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use firment_core::{AgentEvent, ChatMessage, QuestionRequest, SessionMode, ThinkingLevel};
use ratatui::layout::Rect;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use unicode_width::UnicodeWidthChar;

pub(crate) struct App {
    pub(crate) items: Vec<Item>,
    pub(crate) input: Vec<char>,
    pub(crate) cursor: usize,
    /// Selection inside the input box (anchor char index, current char index)
    pub(crate) input_sel: Option<(usize, usize)>,
    /// Collapsed paste blocks: placeholder text + original text
    pub(crate) paste_blocks: Vec<PasteBlock>,
    pub(crate) paste_burst: PasteBurst,
    pub(crate) history: Vec<String>,
    pub(crate) history_pos: Option<usize>,
    pub(crate) busy: bool,
    pub(crate) ai_thinking: bool,
    /// Tools currently running (raw name, activity label) for status hints.
    pub(crate) active_tools: Vec<(String, String)>,
    /// While busy, the first Esc arms an interrupt confirmation window (5s);
    /// a second Esc inside it actually cancels the turn.
    pub(crate) interrupt_armed_at: Option<Instant>,
    /// Pre-extracted cancellation handles, wired in by `run` only, so the
    /// Esc interrupt fires them directly without going through the (possibly
    /// blocked) command channel.
    pub(crate) cancel_tx: Option<watch::Sender<bool>>,
    pub(crate) cancel_signal: Option<firment_core::Cancellable>,
    /// Lazily-refreshed git state for the status bar (None outside a repo).
    pub(crate) git: Option<GitInfo>,
    pub(crate) permission: Option<PermissionRequest>,
    /// Permission requests that arrived while one was already on screen
    /// (concurrent tools/subagents in a wave). They are shown one at a time:
    /// the user decides the current request before the next appears, instead
    /// of the older one being denied implicitly.
    pub(crate) permission_queue: VecDeque<PermissionRequest>,
    /// Pending `ask_user` question shown as a modal; the agent is blocked until
    /// the user answers or dismisses it.
    pub(crate) question: Option<QuestionRequest>,
    /// Free-form answer being typed into the question modal.
    pub(crate) question_input: Vec<char>,
    pub(crate) interrupting: bool,
    pub(crate) scroll: usize,
    pub(crate) max_offset: usize,
    pub(crate) follow: bool,
    pub(crate) input_scroll: usize,
    pub(crate) quit: bool,
    pub(crate) model: String,
    pub(crate) provider: String,
    pub(crate) thinking: ThinkingLevel,
    pub(crate) mode: SessionMode,
    /// Set by `/new`: the transcript was cleared locally; events from the old
    /// turn are suppressed until `SessionLoaded` for the fresh session arrives.
    pub(crate) pending_new_session: bool,
    /// Items index captured by `/new`; messages added after it (e.g. a message
    /// typed and sent while the fresh session is still loading) survive the
    /// transcript clear in `SessionLoaded`.
    pub(crate) pending_new_baseline: usize,
    pub(crate) model_picker: Option<ModelPicker>,
    pub(crate) session_picker: Option<SessionPicker>,
    /// The user closed the picker with Esc. A late data event (the async
    /// model/session list arriving after dismissal) must not re-open it.
    pub(crate) model_picker_dismissed: bool,
    pub(crate) session_picker_dismissed: bool,
    pub(crate) transcript_rect: Rect,
    pub(crate) input_rect: Rect,
    pub(crate) content_width: usize,
    pub(crate) input_width: usize,
    pub(crate) selection: Option<Selection>,
    pub(crate) cwd: PathBuf,
    pub(crate) config_path: PathBuf,
    pub(crate) cmd_tx: mpsc::Sender<AgentCmd>,
    pub(crate) always: Arc<Mutex<HashSet<String>>>,
    pub(crate) frame: u64,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        cmd_tx: mpsc::Sender<AgentCmd>,
        always: Arc<Mutex<HashSet<String>>>,
        model: String,
        cwd: PathBuf,
        provider: String,
        thinking: ThinkingLevel,
        mode: SessionMode,
        config_path: PathBuf,
        startup_hint: Option<String>,
        initial_messages: Vec<ChatMessage>,
    ) -> Self {
        let mut app = Self {
            items: Vec::new(),
            input: Vec::new(),
            cursor: 0,
            input_sel: None,
            paste_blocks: Vec::new(),
            paste_burst: PasteBurst::default(),
            history: Vec::new(),
            history_pos: None,
            busy: false,
            ai_thinking: false,
            active_tools: Vec::new(),
            permission: None,
            permission_queue: VecDeque::new(),
            question: None,
            question_input: Vec::new(),
            interrupting: false,
            interrupt_armed_at: None,
            cancel_tx: None,
            cancel_signal: None,
            git: None,
            scroll: 0,
            max_offset: 0,
            follow: true,
            input_scroll: 0,
            quit: false,
            model,
            provider,
            thinking,
            mode,
            pending_new_session: false,
            pending_new_baseline: 0,
            model_picker: None,
            session_picker: None,
            model_picker_dismissed: false,
            session_picker_dismissed: false,
            transcript_rect: Rect::default(),
            input_rect: Rect::default(),
            content_width: 0,
            input_width: 80,
            selection: None,
            cwd,
            config_path,
            cmd_tx,
            always,
            frame: 0,
        };
        if let Some(hint) = startup_hint {
            app.items.push(Item::System(hint));
        }
        app.push_messages(&initial_messages);
        app
    }

    pub(crate) fn push_messages(&mut self, messages: &[ChatMessage]) {
        for message in messages {
            match message {
                ChatMessage::User { content } => {
                    self.items.push(Item::User(content.clone()));
                }
                ChatMessage::Assistant { content, .. } => {
                    self.items.push(Item::Assistant(content.clone()));
                }
                ChatMessage::Tool { name, content, .. } => {
                    let ok = !content.starts_with("Permission denied")
                        && !content.starts_with("unknown tool")
                        && !content.starts_with("[Permission] Dangerous command");
                    self.items.push(Item::Tool {
                        name: name.clone(),
                        seq: u64::MAX,
                        running: false,
                        ok,
                        summary: content.clone(),
                    });
                }
                ChatMessage::System { content } => {
                    self.items.push(Item::System(content.clone()));
                }
            }
        }
    }

    pub(crate) fn on_agent(&mut self, event: AgentEvent) {
        // While `/new` is in flight, ignore events from the old turn (stream
        // deltas, tool cards, interrupt/rollback messages) so they cannot leak
        // into the fresh conversation.
        if self.pending_new_session && !matches!(&event, AgentEvent::SessionLoaded(_)) {
            return;
        }
        match event {
            AgentEvent::TurnStart => {
                self.busy = true;
                self.ai_thinking = true;
            }
            AgentEvent::TextDelta(text) => match self.items.last_mut() {
                Some(Item::Assistant(buffer)) => {
                    self.ai_thinking = false;
                    buffer.push_str(&text);
                }
                _ => {
                    self.ai_thinking = false;
                    self.items.push(Item::Assistant(text));
                }
            },
            // Real reasoning deltas from extended thinking: keep the
            // "thinking…" indicator up (it previously flipped off only when
            // text started, which made enabled-thinking look identical to
            // off while the model was actually reasoning).
            AgentEvent::Thinking(_) => {
                self.ai_thinking = true;
            }
            AgentEvent::ToolStart { name, args, seq } => {
                self.ai_thinking = false;
                self.active_tools
                    .push((name.clone(), tool_activity(&name, &args)));
                self.items.push(Item::Tool {
                    name,
                    seq,
                    running: true,
                    ok: false,
                    summary: args.to_string(),
                });
            }
            AgentEvent::ToolEnd {
                name,
                ok,
                summary,
                seq,
            } => {
                if let Some(pos) = self.active_tools.iter().position(|(n, _)| n == &name) {
                    self.active_tools.remove(pos);
                }
                for item in self.items.iter_mut().rev() {
                    if let Item::Tool {
                        name: n,
                        seq: item_seq,
                        running,
                        ok: current_ok,
                        summary: current_summary,
                    } = item
                        && n == &name
                        && *item_seq == seq
                    {
                        *running = false;
                        *current_ok = ok;
                        *current_summary = summary;
                        break;
                    }
                }
            }
            AgentEvent::TurnEnd { .. } => {
                self.busy = false;
                self.ai_thinking = false;
                self.interrupting = false;
                self.interrupt_armed_at = None;
            }
            AgentEvent::Info(message) => self.items.push(Item::System(message)),
            AgentEvent::Settings {
                provider,
                model,
                thinking,
                mode,
            } => {
                if let Some(provider) = provider {
                    self.provider = provider;
                }
                if let Some(model) = model {
                    self.model = model;
                }
                if let Some(thinking) = thinking {
                    self.thinking = thinking;
                }
                if let Some(mode) = mode {
                    self.mode = mode;
                }
            }
            AgentEvent::Models(models) => {
                if let Some(picker) = &mut self.model_picker {
                    picker.models = models;
                    picker.clamp();
                } else if !self.model_picker_dismissed {
                    self.model_picker = Some(ModelPicker::new(models));
                }
            }
            AgentEvent::Sessions(sessions) => {
                if let Some(picker) = &mut self.session_picker {
                    picker.sessions = sessions;
                    picker.clamp();
                } else if !self.session_picker_dismissed {
                    self.session_picker = Some(SessionPicker::new(sessions));
                }
            }
            AgentEvent::SessionLoaded(session) => {
                let was_new = self.pending_new_session;
                self.pending_new_session = false;
                // Keep anything the user added after `/new` (e.g. a message
                // typed and sent while the fresh session was loading).
                let keep = if was_new {
                    self.items.split_off(self.pending_new_baseline)
                } else {
                    Vec::new()
                };
                self.items.clear();
                self.provider = session.provider.clone();
                self.model = session.model.clone();
                self.thinking = session.thinking;
                self.mode = session.mode;
                self.cwd = session.cwd.clone();
                self.busy = false;
                self.ai_thinking = false;
                // A session swap invalidates any pending prompts/pickers.
                self.deny_all_permissions();
                if let Some(previous) = self.question.take() {
                    let _ = previous.reply.send(None);
                }
                self.model_picker = None;
                self.session_picker = None;
                self.model_picker_dismissed = true;
                self.session_picker_dismissed = true;
                self.follow = true;
                self.scroll = 0;
                self.max_offset = 0;
                self.input_scroll = 0;
                self.input_sel = None;
                self.paste_blocks.clear();
                self.interrupting = false;
                self.interrupt_armed_at = None;
                if was_new {
                    self.items
                        .push(Item::System("New conversation started".to_string()));
                }
                self.items.extend(keep);
                self.push_messages(&session.messages);
            }
            AgentEvent::Error(message) => {
                self.items.push(Item::Error(message));
                self.busy = false;
                self.ai_thinking = false;
                self.interrupting = false;
                self.interrupt_armed_at = None;
            }
        }
    }

    pub(crate) fn on_permission(&mut self, request: PermissionRequest) {
        // Requests from a tool wave can arrive while one is already on
        // screen. Queue them instead of replacing (and implicitly denying)
        // the pending one: the user decides the current request first, and
        // the next one pops up only after an answer is given.
        if self.permission.is_some() {
            self.permission_queue.push_back(request);
            return;
        }
        self.show_permission(request);
    }

    /// Display a permission request (or the next queued one) on screen.
    pub(crate) fn show_permission(&mut self, request: PermissionRequest) {
        self.items.push(Item::Permission {
            tool: request.tool.clone(),
            reason: request.reason.clone(),
        });
        // The inline card must be visible; force the view back to the bottom.
        self.follow = true;
        self.scroll = 0;
        self.permission = Some(request);
    }

    /// Pop the next queued permission request, if any, and show it.
    pub(crate) fn pop_permission(&mut self) {
        if self.permission.is_none()
            && let Some(request) = self.permission_queue.pop_front()
        {
            self.show_permission(request);
        }
    }

    /// Deny every pending and queued permission request (session swap, /new).
    pub(crate) fn deny_all_permissions(&mut self) {
        if let Some(previous) = self.permission.take() {
            let _ = previous.reply.send(false);
        }
        while let Some(queued) = self.permission_queue.pop_front() {
            let _ = queued.reply.send(false);
        }
    }

    pub(crate) fn on_question(&mut self, request: QuestionRequest) {
        // Same story as permissions: dismiss any pending question so its
        // caller does not hang waiting for an answer that can never come.
        if let Some(previous) = self.question.take() {
            let _ = previous.reply.send(None);
        }
        self.question_input.clear();
        // The modal IS the question display — no transcript duplicate. (The
        // old `Item::System("❓ …")` row landed mid-tool-list and, never
        // removed after answering, stayed in the transcript forever.)
        // The question modal must be visible; force the view back to the bottom.
        self.follow = true;
        self.scroll = 0;
        self.question = Some(request);
    }

    pub(crate) fn on_question_key(&mut self, key: KeyEvent) -> bool {
        let Some(question) = self.question.take() else {
            return false;
        };
        let answer = match key.code {
            KeyCode::Char(d)
                if d.is_ascii_digit() && d != '0' && self.question_input.is_empty() =>
            {
                let idx = (d as usize) - ('1' as usize);
                match question.options.get(idx).cloned() {
                    Some(option) => Some(option),
                    // Out-of-range digit: a stray keypress must not answer
                    // (decline) on the user's behalf — put the modal back.
                    None => {
                        self.question = Some(question);
                        return false;
                    }
                }
            }
            KeyCode::Enter => {
                let typed: String = self.question_input.iter().collect();
                let typed = typed.trim().to_string();
                if typed.is_empty() {
                    // Empty Enter is a no-op, not a decline.
                    self.question = Some(question);
                    return false;
                }
                Some(typed)
            }
            KeyCode::Backspace => {
                self.question_input.pop();
                self.question = Some(question);
                return false;
            }
            KeyCode::Esc => None,
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.question_input.push(ch);
                self.question = Some(question);
                return false;
            }
            _ => {
                self.question = Some(question);
                return false;
            }
        };
        self.question_input.clear();
        // Transcript echo of what was answered — the question itself only
        // ever lived in the modal.
        let echo = match &answer {
            Some(a) => format!("question answered: {a}"),
            None => "question dismissed".to_string(),
        };
        self.items.push(Item::System(echo));
        let _ = question.reply.send(answer);
        false
    }

    pub(crate) fn on_ui(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key_with_burst(key),
            Event::Paste(text) => {
                self.paste_burst.clear();
                self.apply_burst_outputs();
                self.insert_text_at_cursor(&text, true);
                false
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.scroll_up(3);
                    false
                }
                MouseEventKind::ScrollDown => {
                    self.scroll_down(3);
                    false
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(idx) = self.cell_to_input(mouse.column, mouse.row) {
                        self.cursor = idx;
                        self.input_sel = Some((idx, idx));
                        self.selection = None;
                    } else {
                        self.input_sel = None;
                        self.selection =
                            self.cell_to_content(mouse.column, mouse.row)
                                .map(|(row, col)| Selection {
                                    anchor_row: row,
                                    anchor_col: col,
                                    row,
                                    col,
                                });
                    }
                    false
                }
                MouseEventKind::Drag(MouseButton::Left) => {
                    if let Some((anchor, _)) = self.input_sel {
                        if let Some(idx) = self.cell_to_input(mouse.column, mouse.row) {
                            self.input_sel = Some((anchor, idx));
                            self.cursor = idx;
                        }
                    } else if let Some((row, col)) = self.cell_to_content(mouse.column, mouse.row)
                        && let Some(selection) = &mut self.selection
                    {
                        selection.row = row;
                        selection.col = col;
                    }
                    false
                }
                MouseEventKind::Up(MouseButton::Left) => false,
                MouseEventKind::Down(MouseButton::Right) => {
                    if self.input_sel.is_some() && self.input_selection_text().is_some() {
                        self.copy_input_selection();
                    } else if self.selection.is_some() {
                        self.copy_selection();
                    } else {
                        self.paste_clipboard();
                    }
                    false
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Key entry point: runs paste-burst detection first, then falls back to
    /// the original key handling.
    pub(crate) fn on_key_with_burst(&mut self, key: KeyEvent) -> bool {
        if self.permission.is_some()
            || self.question.is_some()
            || self.model_picker.is_some()
            || self.session_picker.is_some()
        {
            return self.on_key(key);
        }
        self.on_key_burst(key, Instant::now())
    }

    /// Key handling with paste-burst detection; `now` lets tests inject time.
    pub(crate) fn on_key_burst(&mut self, key: KeyEvent, now: Instant) -> bool {
        self.paste_burst.flush_if_due(now);
        match key.code {
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::SUPER) =>
            {
                self.paste_burst.on_plain_char(ch, now, self.cursor);
                self.apply_burst_outputs_at(now);
                false
            }
            KeyCode::Enter
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                let action = if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.paste_burst.on_shift_enter(now)
                } else {
                    self.paste_burst.on_enter(now)
                };
                match action {
                    EnterAction::Submit => {
                        self.paste_burst.clear();
                        self.apply_burst_outputs_at(now);
                        self.submit();
                    }
                    EnterAction::Newline => {
                        self.apply_burst_outputs_at(now);
                        self.insert_char('\n');
                    }
                    EnterAction::BufferNewline => {}
                }
                false
            }
            _ => {
                self.paste_burst.clear();
                self.apply_burst_outputs_at(now);
                self.on_key(key)
            }
        }
    }

    /// Apply outputs queued by the paste burst; returns whether anything was
    /// applied.
    pub(crate) fn apply_burst_outputs_at(&mut self, now: Instant) -> bool {
        self.paste_burst.flush_if_due(now);
        let mut applied = false;
        for out in self.paste_burst.drain_outputs() {
            match out {
                PasteOut::InsertChar(c) => self.insert_char(c),
                PasteOut::RemoveAt(pos, expected) => {
                    if pos < self.input.len() && self.input[pos] == expected {
                        self.input.remove(pos);
                        if self.cursor > pos {
                            self.cursor -= 1;
                        }
                    }
                }
                PasteOut::HandlePaste(text) => self.insert_text_at_cursor(&text, true),
            }
            applied = true;
        }
        applied
    }

    pub(crate) fn apply_burst_outputs(&mut self) -> bool {
        self.apply_burst_outputs_at(Instant::now())
    }

    pub(crate) fn on_key(&mut self, key: KeyEvent) -> bool {
        // Global shortcuts (Ctrl+Q quit, Ctrl+C / Ctrl+Shift+C copy, Ctrl+V
        // paste) stay live even while a permission/question/picker modal is
        // up — a long approval queue must never trap the user in the TUI.
        if let Some(handled) = self.global_shortcut(key) {
            return handled;
        }
        if self.permission.is_some() {
            return self.on_permission_key(key);
        }
        if self.question.is_some() {
            return self.on_question_key(key);
        }
        if self.model_picker.is_some() {
            return self.on_picker_key(key);
        }
        if self.session_picker.is_some() {
            return self.on_session_picker_key(key);
        }
        match key.code {
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_model_picker();
                false
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = 0;
                self.input_sel = None;
                false
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = self.input.len();
                self.input_sel = None;
                false
            }
            KeyCode::Char(ch) => {
                self.insert_char(ch);
                false
            }
            KeyCode::Backspace => {
                self.backspace();
                false
            }
            KeyCode::Delete => {
                self.delete_char();
                false
            }
            KeyCode::Left => {
                self.move_cursor_left();
                false
            }
            KeyCode::Right => {
                self.move_cursor_right();
                false
            }
            KeyCode::Home => {
                self.cursor = 0;
                self.input_sel = None;
                false
            }
            KeyCode::End => {
                self.cursor = self.input.len();
                self.input_sel = None;
                false
            }
            KeyCode::Up => {
                if self.history_pos.is_some() || (self.input.is_empty() && !self.history.is_empty())
                {
                    self.history_up();
                } else if !self.input.is_empty() && self.input_line_count() > 1 {
                    self.move_input_cursor(-1);
                } else {
                    self.scroll_up(1);
                }
                false
            }
            KeyCode::Down => {
                if self.history_pos.is_some() {
                    self.history_down();
                } else if !self.input.is_empty() && self.input_line_count() > 1 {
                    self.move_input_cursor(1);
                } else {
                    self.scroll_down(1);
                }
                false
            }
            KeyCode::PageUp => {
                self.scroll_up(10);
                false
            }
            KeyCode::PageDown => {
                self.scroll_down(10);
                false
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.insert_char('\n');
                false
            }
            KeyCode::Enter => {
                self.submit();
                false
            }
            KeyCode::Esc => {
                if self.busy {
                    // Double-Esc confirmation: the first Esc arms a short
                    // window, the second Esc inside it actually cancels the
                    // turn. A stale arm is just a hint.
                    const ESC_CONFIRM_WINDOW: Duration = Duration::from_secs(5);
                    let armed = self
                        .interrupt_armed_at
                        .is_some_and(|t| t.elapsed() < ESC_CONFIRM_WINDOW);
                    if armed {
                        self.interrupt_armed_at = None;
                        self.request_interrupt();
                    } else {
                        self.interrupt_armed_at = Some(Instant::now());
                        self.items.push(Item::System(
                            "⏸ Press Esc again within 5s to interrupt…".to_string(),
                        ));
                    }
                } else {
                    self.interrupt_armed_at = None;
                    self.input.clear();
                    self.cursor = 0;
                    self.input_sel = None;
                    self.paste_blocks.clear();
                }
                false
            }
            _ => false,
        }
    }

    /// Quit / copy / paste shortcuts that must work regardless of what modal
    /// is on screen. Returns `Some(handled)` when the key was consumed.
    pub(crate) fn global_shortcut(&mut self, key: KeyEvent) -> Option<bool> {
        match key.code {
            KeyCode::Char('c')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.copy_last_output();
                Some(false)
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.copy_primary_selection();
                Some(false)
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.paste_clipboard();
                Some(false)
            }
            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.quit = true;
                Some(true)
            }
            _ => None,
        }
    }

    /// Queue a command for the agent task. If the channel is full, surface
    /// the loss instead of silently dropping the user's action.
    pub(crate) fn send_cmd(&mut self, cmd: AgentCmd) {
        if self.cmd_tx.try_send(cmd).is_err() {
            self.items.push(Item::Error(
                "command channel is full; please retry".to_string(),
            ));
        }
    }

    pub(crate) fn request_interrupt(&mut self) {
        if self.interrupting {
            return;
        }
        self.interrupting = true;
        // Cancel directly instead of queueing AgentCmd::Cancel: while a turn
        // holds the agent lock, the command loop can be blocked on a queued
        // command's lock wait (e.g. /model), which would otherwise stall the
        // channel and make Esc unable to interrupt a long turn.
        if let Some(tx) = &self.cancel_tx {
            let _ = tx.send(true);
        }
        if let Some(signal) = &self.cancel_signal {
            signal.cancel();
        }
        self.items
            .push(Item::System("⏹ Interrupt request sent…".to_string()));
    }

    /// Soft-wrap the input to the display width; returns (lines, line start
    /// char indexes, cursor line, cursor column). Cursor positions are char
    /// indexes and account for CJK wide chars.
    pub(crate) fn input_layout(&self, width: usize) -> (Vec<String>, Vec<usize>, usize, usize) {
        let chars = &self.input;
        let mut lines: Vec<String> = Vec::new();
        let mut line_starts: Vec<usize> = Vec::new();
        let mut line_start = 0usize;
        let mut current = String::new();
        let mut current_w = 0usize;
        for (pos, &ch) in chars.iter().enumerate() {
            if ch == '\n' {
                lines.push(std::mem::take(&mut current));
                line_starts.push(line_start);
                current_w = 0;
                line_start = pos + 1;
                continue;
            }
            let w = ch.width().unwrap_or(0);
            if current_w + w > width && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                line_starts.push(line_start);
                line_start = pos;
                current_w = 0;
            }
            current.push(ch);
            current_w += w;
        }
        if !current.is_empty() || lines.is_empty() {
            lines.push(current);
            line_starts.push(line_start);
        }
        let cursor = self.cursor.min(chars.len());
        let cursor_line = line_starts
            .iter()
            .rposition(|&start| cursor >= start)
            .unwrap_or(0);
        let cursor_col: usize = chars[line_starts[cursor_line]..cursor]
            .iter()
            .map(|c| c.width().unwrap_or(0))
            .sum();
        (lines, line_starts, cursor_line, cursor_col)
    }

    pub(crate) fn on_permission_key(&mut self, key: KeyEvent) -> bool {
        let Some(prompt) = self.permission.take() else {
            return false;
        };
        let (allowed, always) = match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => (true, false),
            KeyCode::Char('a') | KeyCode::Char('A') => (true, true),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => (false, false),
            _ => {
                self.permission = Some(prompt);
                return false;
            }
        };
        if always {
            self.always
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(prompt.tool.clone());
        }
        let _ = prompt.reply.send(allowed);
        if let Some(idx) = self
            .items
            .iter()
            .rposition(|item| matches!(item, Item::Permission { .. }))
        {
            self.items.remove(idx);
        }
        self.items.push(Item::System(format!(
            "{}: {}",
            if allowed { "✓ Allowed" } else { "✗ Denied" },
            prompt.tool
        )));
        self.pop_permission();
        false
    }

    pub(crate) fn insert_char(&mut self, ch: char) {
        self.history_pos = None;
        self.input_sel = None;
        let cursor = self.snap_cursor(self.cursor);
        self.cursor = cursor;
        if cursor >= self.input.len() {
            self.input.push(ch);
        } else {
            self.input.insert(cursor, ch);
        }
        self.cursor += 1;
    }

    pub(crate) fn backspace(&mut self) {
        self.input_sel = None;
        if self.cursor == 0 {
            return;
        }
        if let Some((start, end, idx)) = self.placeholder_range_with_end(self.cursor) {
            self.input.drain(start..end);
            self.paste_blocks.remove(idx);
            self.cursor = start;
            return;
        }
        self.history_pos = None;
        self.input.remove(self.cursor - 1);
        self.cursor -= 1;
    }

    pub(crate) fn delete_char(&mut self) {
        self.input_sel = None;
        if let Some((start, end, idx)) = self.placeholder_range_with_start(self.cursor) {
            self.input.drain(start..end);
            self.paste_blocks.remove(idx);
            return;
        }
        if self.cursor < self.input.len() {
            self.history_pos = None;
            self.input.remove(self.cursor);
        }
    }

    /// Move the cursor left; a collapsed placeholder is treated as one unit
    /// (jump from its tail to its head).
    pub(crate) fn move_cursor_left(&mut self) {
        self.input_sel = None;
        if let Some((start, _end, _)) = self.placeholder_range_with_end(self.cursor) {
            self.cursor = start;
            return;
        }
        self.cursor = self.cursor.saturating_sub(1);
        self.cursor = self.snap_cursor(self.cursor);
    }

    /// Move the cursor right; a collapsed placeholder is treated as one unit
    /// (jump from its head to its tail).
    pub(crate) fn move_cursor_right(&mut self) {
        self.input_sel = None;
        if let Some((_start, end, _)) = self.placeholder_range_with_start(self.cursor) {
            self.cursor = end;
            return;
        }
        self.cursor = (self.cursor + 1).min(self.input.len());
        self.cursor = self.snap_cursor(self.cursor);
    }

    /// Move the cursor up/down inside the input (by display line), keeping the
    /// column position.
    pub(crate) fn move_input_cursor(&mut self, delta: isize) {
        if self.input.is_empty() {
            return;
        }
        let (lines, line_starts, cursor_line, cursor_col) =
            self.input_layout(self.input_width.max(1));
        let target_line = if delta < 0 {
            cursor_line.saturating_sub(1)
        } else {
            (cursor_line + 1).min(lines.len().saturating_sub(1))
        };
        if target_line == cursor_line {
            return;
        }
        let line_text = &lines[target_line];
        let col = cursor_col.min(cell_width(line_text));
        let char_in_line = char_index_at_cell(line_text, col);
        let target = line_starts[target_line] + char_in_line;
        self.cursor = self.snap_cursor(target);
        self.history_pos = None;
        self.input_sel = None;
    }

    pub(crate) fn input_line_count(&self) -> usize {
        if self.input.is_empty() {
            return 0;
        }
        self.input_layout(self.input_width.max(1)).0.len()
    }

    /// Positions of all collapsed placeholders in the current input (in input
    /// order).
    pub(crate) fn placeholder_ranges(&self) -> Vec<(usize, usize, usize)> {
        let mut ranges = Vec::new();
        let mut search_from = 0usize;
        for (idx, block) in self.paste_blocks.iter().enumerate() {
            let needle: Vec<char> = block.placeholder.chars().collect();
            if let Some(pos) = find_subslice(&self.input, &needle, search_from) {
                ranges.push((pos, pos + needle.len(), idx));
                search_from = pos + needle.len();
            }
        }
        ranges
    }

    pub(crate) fn placeholder_range_with_end(
        &self,
        cursor: usize,
    ) -> Option<(usize, usize, usize)> {
        self.placeholder_ranges()
            .into_iter()
            .find(|(_, end, _)| *end == cursor)
    }

    pub(crate) fn placeholder_range_with_start(
        &self,
        cursor: usize,
    ) -> Option<(usize, usize, usize)> {
        self.placeholder_ranges()
            .into_iter()
            .find(|(start, _, _)| *start == cursor)
    }

    /// If the cursor lands inside a collapsed placeholder, jump to its end.
    pub(crate) fn snap_cursor(&self, cursor: usize) -> usize {
        for (start, end, _) in self.placeholder_ranges() {
            if start < cursor && cursor < end {
                return end;
            }
        }
        cursor
    }

    /// Expand the input to full text: placeholders are replaced by their
    /// original pasted content.
    pub(crate) fn expand_input(&self) -> String {
        let chars = &self.input;
        let mut out = String::new();
        let mut last = 0usize;
        for (start, end, idx) in self.placeholder_ranges() {
            out.extend(chars[last..start].iter());
            out.push_str(&self.paste_blocks[idx].text);
            last = end;
        }
        out.extend(chars[last..].iter());
        out
    }

    /// Full text of the input selection (with collapsed blocks expanded).
    pub(crate) fn input_selection_text(&self) -> Option<String> {
        let (a, b) = self.input_sel?;
        let (s0, s1) = if a <= b { (a, b) } else { (b, a) };
        if s0 == s1 {
            return None;
        }
        let chars = &self.input;
        let mut out = String::new();
        let mut last = s0;
        for (start, end, idx) in self.placeholder_ranges() {
            if start >= s1 {
                break;
            }
            if end <= s0 {
                continue;
            }
            let seg_start = start.max(s0);
            let seg_end = end.min(s1);
            if last < seg_start {
                out.extend(chars[last..seg_start].iter());
            }
            out.push_str(&self.paste_blocks[idx].text);
            last = seg_end;
        }
        if last < s1 {
            out.extend(chars[last..s1].iter());
        }
        if out.is_empty() { None } else { Some(out) }
    }

    /// Insert text at the cursor; with `collapse` true, large text is folded
    /// into a placeholder.
    pub(crate) fn insert_text_at_cursor(&mut self, text: &str, collapse: bool) {
        let text: String = text.chars().filter(|c| *c != '\r').collect();
        if text.is_empty() {
            return;
        }
        self.history_pos = None;
        self.input_sel = None;
        let start = self.snap_cursor(self.cursor);
        self.cursor = start;
        let insert: Vec<char> = if collapse && Self::needs_collapse(&text) {
            let placeholder = Self::collapse_label(&text, &self.paste_blocks);
            self.paste_blocks.push(PasteBlock {
                placeholder: placeholder.clone(),
                text: text.clone(),
            });
            placeholder.chars().collect()
        } else {
            text.chars().collect()
        };
        if start >= self.input.len() {
            self.input.extend(insert.iter().copied());
        } else {
            self.input.splice(start..start, insert.iter().copied());
        }
        self.cursor = start + insert.len();
    }

    pub(crate) fn needs_collapse(text: &str) -> bool {
        text.lines().count() > 2 || text.chars().count() > 150
    }

    pub(crate) fn collapse_label(text: &str, existing: &[PasteBlock]) -> String {
        let line_count = text.lines().count().max(1);
        let mut label = if line_count > 1 {
            format!("【line 1-{line_count}】")
        } else {
            format!("【collapsed {} chars】", text.chars().count())
        };
        let mut id = 1usize;
        while existing.iter().any(|b| b.placeholder == label) {
            id += 1;
            label = if line_count > 1 {
                format!("【line 1-{line_count}#{id}】")
            } else {
                format!("【collapsed {} chars#{id}】", text.chars().count())
            };
        }
        label
    }

    pub(crate) fn copy_input_selection(&mut self) {
        let Some(text) = self.input_selection_text() else {
            return;
        };
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        match copy_to_clipboard(text) {
            Ok(()) => self.items.push(Item::System(format!(
                "Copied input selection ({} chars)",
                text.chars().count()
            ))),
            Err(e) => self.items.push(Item::System(format!("Copy failed: {e}"))),
        }
    }

    /// Ctrl+C priority: input selection > transcript selection > last reply.
    pub(crate) fn copy_primary_selection(&mut self) {
        if self.input_sel.is_some() && self.input_selection_text().is_some() {
            self.copy_input_selection();
        } else if self.selection.is_some() {
            self.copy_selection();
        } else if let Some(text) = self.last_output_text() {
            match copy_to_clipboard(&text) {
                Ok(()) => self.items.push(Item::System(format!(
                    "Copied the last reply ({} chars)",
                    text.chars().count()
                ))),
                Err(e) => self.items.push(Item::System(format!("Copy failed: {e}"))),
            }
        }
    }

    pub(crate) fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next = match self.history_pos {
            Some(pos) if pos > 0 => pos - 1,
            _ => self.history.len() - 1,
        };
        self.history_pos = Some(next);
        self.input = self.history[next].chars().collect();
        self.cursor = self.input.len();
        self.paste_blocks.clear();
        self.input_sel = None;
    }

    pub(crate) fn history_down(&mut self) {
        let Some(pos) = self.history_pos else {
            return;
        };
        if pos + 1 < self.history.len() {
            let next = pos + 1;
            self.history_pos = Some(next);
            self.input = self.history[next].chars().collect();
            self.cursor = self.input.len();
            self.paste_blocks.clear();
            self.input_sel = None;
        } else {
            self.history_pos = None;
            self.input.clear();
            self.cursor = 0;
            self.paste_blocks.clear();
            self.input_sel = None;
        }
    }

    pub(crate) fn open_model_picker(&mut self) {
        if self.model_picker.is_some() {
            return;
        }
        self.model_picker_dismissed = false;
        self.model_picker = Some(ModelPicker::new(Vec::new()));
        self.send_cmd(AgentCmd::OpenModelPicker);
    }

    pub(crate) fn on_picker_key(&mut self, key: KeyEvent) -> bool {
        let Some(picker) = self.model_picker.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => {
                self.model_picker = None;
                self.model_picker_dismissed = true;
            }
            KeyCode::Up => {
                if picker.selected > 0 {
                    picker.selected -= 1;
                }
            }
            KeyCode::Down => {
                let count = picker.filtered().len();
                if count > 0 && picker.selected + 1 < count {
                    picker.selected += 1;
                }
            }
            KeyCode::Home => picker.selected = 0,
            KeyCode::End => {
                let count = picker.filtered().len();
                if count > 0 {
                    picker.selected = count - 1;
                }
            }
            KeyCode::Enter => {
                if let Some(model) = picker.selected_model() {
                    self.model = model.clone();
                    self.items
                        .push(Item::System(format!("model -> {model} (switching…)")));
                    self.send_cmd(AgentCmd::SetModel(model));
                }
                self.model_picker = None;
            }
            KeyCode::Backspace => {
                picker.query.pop();
                picker.clamp();
            }
            KeyCode::Char(ch) => {
                picker.query.push(ch);
                picker.clamp();
            }
            _ => {}
        }
        false
    }

    pub(crate) fn open_session_picker(&mut self) {
        if self.session_picker.is_some() {
            return;
        }
        self.session_picker_dismissed = false;
        self.session_picker = Some(SessionPicker::new(Vec::new()));
        self.send_cmd(AgentCmd::OpenSessionPicker);
    }

    pub(crate) fn on_session_picker_key(&mut self, key: KeyEvent) -> bool {
        let Some(picker) = self.session_picker.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => {
                self.session_picker = None;
                self.session_picker_dismissed = true;
            }
            KeyCode::Up => {
                if picker.selected > 0 {
                    picker.selected -= 1;
                }
            }
            KeyCode::Down => {
                let count = picker.filtered().len();
                if count > 0 && picker.selected + 1 < count {
                    picker.selected += 1;
                }
            }
            KeyCode::Home => picker.selected = 0,
            KeyCode::End => {
                let count = picker.filtered().len();
                if count > 0 {
                    picker.selected = count - 1;
                }
            }
            KeyCode::Enter => {
                if let Some(session) = picker.filtered().get(picker.selected) {
                    let id = session.id.clone();
                    self.items
                        .push(Item::System(format!("Loading session {id}…")));
                    self.send_cmd(AgentCmd::LoadSession(id));
                }
                self.session_picker = None;
            }
            KeyCode::Backspace => {
                picker.query.pop();
                picker.clamp();
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                if let Some(session) = picker.filtered().get(picker.selected).cloned() {
                    match copy_to_clipboard(&session.id) {
                        Ok(()) => self.items.push(Item::System(format!(
                            "copied session id to clipboard: {}",
                            session.id
                        ))),
                        Err(e) => self.items.push(Item::System(format!("copy failed: {e}"))),
                    }
                }
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                if let Some(session) = picker.filtered().get(picker.selected).cloned() {
                    let id = session.id.clone();
                    self.send_cmd(AgentCmd::DeleteSession(id.clone()));
                    self.items
                        .push(Item::System(format!("deleting session {id}…")));
                }
            }
            KeyCode::Char(ch) => {
                picker.query.push(ch);
                picker.clamp();
            }
            _ => {}
        }
        false
    }

    pub(crate) fn cell_to_content(&self, column: u16, row: u16) -> Option<(usize, usize)> {
        let area = self.transcript_rect;
        if area.width == 0 || area.height == 0 {
            return None;
        }
        if row <= area.y || row >= area.y + area.height - 1 {
            return None;
        }
        if column <= area.x || column >= area.x + area.width - 1 {
            return None;
        }
        let visible = (row - area.y - 1) as usize;
        let content_row = self.offset().saturating_add(visible);
        Some((content_row, (column - area.x - 1) as usize))
    }

    /// Terminal cell → input char index (for click/drag selection in the input
    /// box).
    pub(crate) fn cell_to_input(&self, column: u16, row: u16) -> Option<usize> {
        let area = self.input_rect;
        if area.width == 0 || area.height == 0 {
            return None;
        }
        if row <= area.y || row >= area.y + area.height - 1 {
            return None;
        }
        if column <= area.x || column >= area.x + area.width - 1 {
            return None;
        }
        if self.input.is_empty() {
            return Some(0);
        }
        let (lines, line_starts, _, _) = self.input_layout(self.input_width.max(1));
        let line = (row - area.y - 1) as usize + self.input_scroll;
        let line_text = lines.get(line)?;
        let col = (column - area.x - 1) as usize;
        let char_in_line = char_index_at_cell(line_text, col.min(cell_width(line_text)));
        let idx = line_starts[line] + char_in_line;
        Some(self.snap_cursor(idx))
    }

    pub(crate) fn offset(&self) -> usize {
        if self.follow {
            self.max_offset
        } else {
            self.max_offset.saturating_sub(self.scroll)
        }
    }

    pub(crate) fn selection_text(&self, selection: Selection) -> String {
        let width = self.content_width.max(1);
        let rows = self.render_rows(width);
        let ((r0, c0), (r1, c1)) = selection.normalized();
        let mut out = Vec::new();
        for row_idx in r0..=r1 {
            let Some(row) = rows.get(row_idx) else {
                break;
            };
            let text: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
            let (start, end) = if r0 == r1 {
                (c0, c1)
            } else if row_idx == r0 {
                (c0, usize::MAX)
            } else if row_idx == r1 {
                (0, c1)
            } else {
                (0, usize::MAX)
            };
            let total_cells = cell_width(&text);
            let start = start.min(total_cells);
            let end = end.min(total_cells);
            let char_start = char_index_at_cell(&text, start);
            let char_end = char_index_at_cell(&text, end);
            out.push(
                text.chars()
                    .skip(char_start)
                    .take(char_end.saturating_sub(char_start))
                    .collect::<String>(),
            );
        }
        out.join("\n")
    }

    pub(crate) fn copy_selection(&mut self) {
        let Some(selection) = self.selection.take() else {
            return;
        };
        let text = self.selection_text(selection);
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        match copy_to_clipboard(text) {
            Ok(()) => self.items.push(Item::System(format!(
                "Copied selection ({} chars)",
                text.chars().count()
            ))),
            Err(e) => self.items.push(Item::System(format!("Copy failed: {e}"))),
        }
    }

    pub(crate) fn paste_clipboard(&mut self) {
        let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) else {
            return;
        };
        self.insert_text_at_cursor(&text, true);
    }

    pub(crate) fn last_output_text(&self) -> Option<String> {
        self.items.iter().rev().find_map(|item| match item {
            Item::Assistant(text) if !text.trim().is_empty() => Some(text.clone()),
            _ => None,
        })
    }

    pub(crate) fn copy_last_output(&mut self) {
        match self.last_output_text() {
            Some(text) => match copy_to_clipboard(&text) {
                Ok(()) => self.items.push(Item::System(format!(
                    "Copied the last reply ({} chars)",
                    text.chars().count()
                ))),
                Err(e) => self.items.push(Item::System(format!("Copy failed: {e}"))),
            },
            None => self
                .items
                .push(Item::System("No reply to copy yet".to_string())),
        }
    }

    pub(crate) fn scroll_up(&mut self, amount: usize) {
        if self.max_offset == 0 {
            return;
        }
        self.follow = false;
        self.scroll = (self.scroll + amount).min(self.max_offset);
    }

    pub(crate) fn scroll_down(&mut self, amount: usize) {
        if self.follow {
            return;
        }
        self.scroll = self.scroll.saturating_sub(amount);
        if self.scroll == 0 {
            self.follow = true;
        }
    }

    pub(crate) fn submit(&mut self) {
        let text = self.expand_input();
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        if let Some(command) = text.strip_prefix('/') {
            self.run_command(command);
            return;
        }
        // Check busy BEFORE clearing the input so a stale Enter does not
        // wipe a draft the user is still composing.
        if self.busy {
            self.items.push(Item::System(
                "Agent is busy; wait for it to finish.".to_string(),
            ));
            return;
        }
        if self.history.last().map(String::as_str) != Some(text.as_str()) {
            self.history.push(text.clone());
            if self.history.len() > 200 {
                self.history.remove(0);
            }
        }
        self.history_pos = None;
        self.input.clear();
        self.cursor = 0;
        self.input_sel = None;
        self.paste_blocks.clear();
        self.items.push(Item::User(text.clone()));
        self.busy = true;
        self.ai_thinking = true;
        self.follow = true;
        self.scroll = 0;
        self.send_cmd(AgentCmd::User(text));
    }

    pub(crate) fn run_command(&mut self, command: &str) {
        let (name, arg) = command
            .split_once(char::is_whitespace)
            .map(|(n, a)| (n, a.trim()))
            .unwrap_or((command, ""));
        match name {
            "help" => self.items.push(Item::System(
                "Commands: /new  /plan [on|off]  /agent  /models  /model <id>  /sessions (use ↑/↓ to select)  /session <id>  /delete <id>  /undo  /ledger  /pin <path>  /unpin <path>  /copy  /provider <name>  /add-provider <name> <openai|anthropic> <base_url> <model>  /apikey [provider] <key>  /thinking [off|low|medium|high|xhigh|max]  /budget <chars>  /output <tokens>  /context  /config  /clear  /help  /quit\nKeys: ↑/↓ browse history when input is empty, move the input cursor on multi-line input, scroll the transcript on single-line input · Shift+Enter manual newline · PgUp/PgDn/wheel scroll · Ctrl+P model picker · inside /sessions: c copies the selected id to clipboard, d deletes it (drag-select and right-click are disabled because the TUI captures mouse events; press Esc to dismiss the picker, then your terminal's native selection works in the scrollback) · Ctrl+C copies the selection (copies the last reply when there is none) · Ctrl+V paste · Ctrl+Shift+C copy last reply · ←/→ move the input cursor · y/n/a permission answers · Esc interrupts AI output (Esc twice while working; clears input when idle) · Ctrl+Q quit\nInput box: auto-wraps and grows to up to 5 lines; taller content scrolls, large pastes collapse into 【line x-y】, and the title shows hidden/collapsed line counts before sending"
                    .to_string(),
            )),
            "new" => {
                self.paste_burst.clear();
                self.apply_burst_outputs();
                self.input.clear();
                self.cursor = 0;
                self.input_sel = None;
                self.paste_blocks.clear();
                // Clear the transcript immediately and stop any running turn,
                // so the previous conversation cannot linger on screen while
                // the agent task processes the fresh session.
                let was_busy = self.busy;
                self.items.clear();
                self.busy = false;
                self.ai_thinking = false;
                self.interrupting = false;
                self.interrupt_armed_at = None;
                self.deny_all_permissions();
                self.follow = true;
                self.scroll = 0;
                self.pending_new_session = true;
                if was_busy {
                    self.send_cmd(AgentCmd::Cancel);
                }
                self.send_cmd(AgentCmd::NewSession);
                self.items
                    .push(Item::System("Starting a new conversation…".to_string()));
                self.pending_new_baseline = self.items.len();
            }
            "plan" => {
                let mode = match arg {
                    "on" => SessionMode::Plan,
                    "off" => SessionMode::Agent,
                    _ if self.mode == SessionMode::Plan => SessionMode::Agent,
                    _ => SessionMode::Plan,
                };
                self.mode = mode;
                self.send_cmd(AgentCmd::SetMode(mode));
                let queued = if self.busy { " (takes effect after the current turn)" } else { "" };
                self.items.push(Item::System(format!(
                    "mode -> {}{queued}",
                    mode.label()
                )));
            }
            "agent" => {
                self.mode = SessionMode::Agent;
                self.send_cmd(AgentCmd::SetMode(SessionMode::Agent));
                let queued = if self.busy { " (takes effect after the current turn)" } else { "" };
                self.items
                    .push(Item::System(format!("mode -> agent{queued}")));
            }
            "thinking" => {
                let level = if arg.is_empty() {
                    next_thinking(self.thinking)
                } else {
                    match arg.parse::<ThinkingLevel>() {
                        Ok(level) => level,
                        Err(_) => {
                            self.items.push(Item::System(
                                "invalid level; use: off / low / medium / high / xhigh / max"
                                    .to_string(),
                            ));
                            return;
                        }
                    }
                };
                self.thinking = level;
                self.send_cmd(AgentCmd::SetThinking(level));
                self.items
                    .push(Item::System(format!("thinking -> {}", level.label())));
            }
            "budget" => {
                if arg.is_empty() {
                    self.items.push(Item::System(
                        "usage: /budget <chars>  (e.g. 256k, 131072)".to_string(),
                    ));
                    return;
                }
                match firment_core::config::parse_size(arg) {
                    Ok(chars) => {
                        self.send_cmd(AgentCmd::SetContextBudget(chars));
                        self.items
                            .push(Item::System(format!("context budget -> {chars} chars…")));
                    }
                    Err(e) => {
                        self.items.push(Item::System(format!("invalid budget: {e}")));
                    }
                }
            }
            "output" => {
                if arg.is_empty() {
                    self.items.push(Item::System(
                        "usage: /output <tokens>  (e.g. 32k, 16384)".to_string(),
                    ));
                    return;
                }
                match firment_core::config::parse_size(arg) {
                    Ok(tokens) => {
                        let tokens = tokens.min(u32::MAX as usize) as u32;
                        self.send_cmd(AgentCmd::SetMaxOutputTokens(tokens));
                        self.items
                            .push(Item::System(format!("max output tokens -> {tokens}…")));
                    }
                    Err(e) => {
                        self.items.push(Item::System(format!("invalid output cap: {e}")));
                    }
                }
            }
            "context" => {
                self.send_cmd(AgentCmd::ShowContext);
                self.items.push(Item::System("context usage:".to_string()));
            }
            "delete" => {
                if arg.is_empty() {
                    self.items.push(Item::System(
                        "usage: /delete <session-id>  (deletes the transcript and its undo/spill/ledger; /sessions lists ids)".to_string(),
                    ));
                    return;
                }
                self.send_cmd(AgentCmd::DeleteSession(arg.to_string()));
                self.items
                    .push(Item::System(format!("deleting session {arg}…")));
            }
            "provider" if !arg.is_empty() => {
                self.send_cmd(AgentCmd::SetProvider(arg.to_string()));
                self.items
                    .push(Item::System(format!("Switching to provider {arg}…")));
            }
            "model" if !arg.is_empty() => {
                self.model = arg.to_string();
                self.send_cmd(AgentCmd::SetModel(arg.to_string()));
                self.items
                    .push(Item::System(format!("model -> {arg}")));
            }
            "model" => {
                self.open_model_picker();
            }
            "models" => {
                self.send_cmd(AgentCmd::ListModels);
                self.items.push(Item::System(format!(
                    "Fetching model list for {}…",
                    self.provider
                )));
            }
            "sessions" => {
                self.open_session_picker();
            }
            "session" if !arg.is_empty() => {
                self.send_cmd(AgentCmd::LoadSession(arg.to_string()));
                self.items
                    .push(Item::System(format!("Loading session {arg}…")));
            }
            "session" => {
                self.open_session_picker();
            }
            "undo" => {
                self.send_cmd(AgentCmd::Undo);
                self.items.push(Item::System(
                    "Undoing the last committed edit…".to_string(),
                ));
            }
            "ledger" => {
                self.send_cmd(AgentCmd::Ledger);
                self.items.push(Item::System("Reading the change ledger…".to_string()));
            }
            "pin" if !arg.is_empty() => {
                self.send_cmd(AgentCmd::Pin { path: arg.to_string() });
                self.items
                    .push(Item::System(format!("Pinning {arg}…")));
            }
            "pin" => {
                self.items.push(Item::System(
                    "Usage: /pin <path> (keeps the file's full content during compaction)"
                        .to_string(),
                ));
            }
            "unpin" if !arg.is_empty() => {
                self.send_cmd(AgentCmd::Unpin { path: arg.to_string() });
                self.items
                    .push(Item::System(format!("Unpinning {arg}…")));
            }
            "unpin" => {
                self.items.push(Item::System("Usage: /unpin <path>".to_string()));
            }
            "copy" => self.copy_last_output(),
            "apikey" | "key" if !arg.is_empty() => {
                let (provider, key) = match arg.split_once(char::is_whitespace) {
                    Some((p, k)) => (Some(p.to_string()), k.to_string()),
                    None => (None, arg.to_string()),
                };
                self.send_cmd(AgentCmd::SetApiKey { provider, key });
                self.items
                    .push(Item::System("Saving API key…".to_string()));
            }
            "apikey" | "key" => {
                self.items.push(Item::System(
                    "Usage: /apikey <key> (current provider) or /apikey <provider> <key>; saved \
                     to auth.json so you won't need to configure it again"
                        .to_string(),
                ));
            }
            "add-provider" | "addprovider" => {
                let parts: Vec<&str> = arg.split_whitespace().collect();
                if parts.len() != 4 {
                    self.items.push(Item::System(
                        "Usage: /add-provider <name> <openai|anthropic> <base_url> <model>\n\
                         Example: /add-provider deepseek openai https://api.deepseek.com/v1 \
                         deepseek-v4-flash"
                            .to_string(),
                    ));
                    return;
                }
                let (name, r#type, base_url, model) = (parts[0], parts[1], parts[2], parts[3]);
                self.send_cmd(AgentCmd::AddProvider {
                    name: name.to_string(),
                    r#type: r#type.to_string(),
                    base_url: base_url.to_string(),
                    model: model.to_string(),
                });
                self.items
                    .push(Item::System(format!("Saving provider {name}…")));
            }
            "config" => {
                self.items.push(Item::System(format!(
                    "provider: {} · model: {} · thinking: {} · cwd: {}\nconfig: {}\nauth: {}",
                    self.provider,
                    self.model,
                    self.thinking.label(),
                    self.cwd.display(),
                    self.config_path.display(),
                    firment_core::auth_path().display(),
                )));
            }
            "clear" => {
                self.items.clear();
                self.follow = true;
                self.scroll = 0;
            }
            "quit" | "exit" => self.quit = true,
            other => self
                .items
                .push(Item::System(format!("unknown command: /{other}"))),
        }
    }
}

pub(crate) enum Item {
    User(String),
    Assistant(String),
    Tool {
        name: String,
        /// Event-pairing id from AgentEvent::ToolStart/ToolEnd; parallel
        /// same-name tool calls each get their own card.
        seq: u64,
        running: bool,
        ok: bool,
        summary: String,
    },
    /// Permission confirmations render as inline cards in the transcript
    /// instead of popups covering the context.
    Permission {
        tool: String,
        reason: String,
    },
    System(String),
    Error(String),
}
