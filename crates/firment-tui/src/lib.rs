use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    read,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use firment_core::{
    AgentEvent, Asker, Config, PermissionChecker, PlanModePermission, QuestionRequest, Session,
    SessionStore,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::collections::HashSet;
use std::io::{self, Stdout};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc;

mod adapters;
mod app;
mod commands;
mod paste;
mod pickers;
mod util;

use adapters::{ChannelSink, PermissionRequest, TuiAsker, TuiPermission};
use app::App;
use commands::spawn_agent_task;
use util::{GitInfo, git_info};

pub async fn run(
    config: Config,
    config_path: std::path::PathBuf,
    session: Session,
) -> anyhow::Result<()> {
    // Keep the user-level config untouched so `/model` & co. only ever write
    // the user's own settings to the global file — project `.firment.toml`
    // overrides (build_command, default_chip, …) must not leak out of the
    // project's scope.
    let base_config = config;
    let config = base_config.clone().merged_for(&session.cwd);
    let store = SessionStore::default();
    let default_registry = firment_tools::default_registry();
    let plan_registry = firment_tools::plan_registry();

    let (event_tx, event_rx) = mpsc::channel(256);
    let (perm_tx, perm_rx) = mpsc::channel(16);
    let (ask_tx, ask_rx) = mpsc::channel(16);
    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let always: Arc<Mutex<HashSet<String>>> =
        Arc::new(Mutex::new(config.auto_approve.iter().cloned().collect()));

    let sink = Arc::new(ChannelSink { tx: event_tx });
    let tui_permission: Arc<dyn PermissionChecker> = Arc::new(TuiPermission {
        req_tx: perm_tx,
        always: always.clone(),
    });
    let plan_permission: Arc<dyn PermissionChecker> =
        Arc::new(PlanModePermission::new(tui_permission.clone()));
    let asker: Arc<dyn Asker> = Arc::new(TuiAsker { req_tx: ask_tx });

    // Interactive TUI: the permission popup is the decision point, so
    // dangerous shell commands are allowed to reach it (and are labeled ⚠).
    // The TUI must start even without an API key, so a provider failure only
    // becomes a startup hint — the user can run /apikey or /provider inside.
    let mut assembly = firment_tools::assembly::assemble_agent(
        &config,
        session,
        store.clone(),
        sink,
        tui_permission.clone(),
        Some(asker),
        true,
    );
    let startup_hint = assembly.provider_error.take().map(|e| {
        format!("⚠ {e} (run /apikey sk-xxx inside the TUI to configure it without exiting)")
    });

    let session_mode = assembly.agent.session().mode;
    let initial_messages = assembly.agent.session().messages.clone();
    let model = assembly.agent.session().model.clone();
    let cwd = assembly.agent.session().cwd.clone();
    let provider_name = assembly.agent.session().provider.clone();
    let thinking = assembly.agent.session().thinking;
    let task_config = base_config;
    let task_config_path = config_path.clone();
    // Cancel handles were extracted by the assembly BEFORE the agent moves
    // into its lock: cancel must be actionable WHILE a turn is running, so it
    // cannot go through the agent lock.
    //
    // Keep a copy for the app so Esc can fire cancel directly (bypassing the
    // command channel); the originals are moved into the agent task.
    let app_cancel_tx = assembly.cancel_tx.clone();
    let app_cancel_signal = assembly.cancel_signal.clone();
    let agent = Arc::new(tokio::sync::Mutex::new(assembly.agent));
    // Serializes turns: a queued message waits for the running turn instead
    // of running two agent loops against the same session.
    let turn_lock = Arc::new(tokio::sync::Mutex::new(()));
    // The command loop is extracted so tests can drive these exact semantics:
    // turns run on their own task (so the loop stays responsive) and `Cancel`
    // fires the pre-extracted handles directly instead of going through the
    // agent lock — together that is what makes Esc interrupt a running turn.

    let agent_task = spawn_agent_task(
        cmd_rx,
        agent,
        assembly.cancel_tx,
        assembly.cancel_signal,
        turn_lock,
        store.clone(),
        task_config,
        task_config_path,
        plan_registry.clone(),
        default_registry.clone(),
        plan_permission.clone(),
        tui_permission.clone(),
    );

    let (ui_tx, ui_rx) = mpsc::channel(128);
    thread::spawn(move || {
        while let Ok(event) = read() {
            if ui_tx.blocking_send(event).is_err() {
                break;
            }
        }
    });

    let mut terminal = init_terminal()?;
    let mut app = App::new(
        cmd_tx,
        always,
        model,
        cwd,
        provider_name,
        thinking,
        session_mode,
        config_path,
        startup_hint,
        initial_messages,
    );
    // Wire the pre-extracted cancel handles into the app so Esc can fire them
    // directly (bypassing the command channel that may be blocked on the
    // agent lock while a turn runs).
    app.cancel_tx = Some(app_cancel_tx);
    app.cancel_signal = Some(app_cancel_signal);
    let result = run_loop(&mut terminal, &mut app, event_rx, perm_rx, ask_rx, ui_rx).await;
    restore_terminal(&mut terminal)?;
    agent_task.abort();
    result
}

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Maximum input box height (borders included): 2 border rows + up to 5 text rows.
pub(crate) const MAX_INPUT_HEIGHT: usize = 7;

fn init_terminal() -> anyhow::Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Tui) -> anyhow::Result<()> {
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    disable_raw_mode()?;
    Ok(())
}

async fn run_loop(
    terminal: &mut Tui,
    app: &mut App,
    mut event_rx: mpsc::Receiver<AgentEvent>,
    mut perm_rx: mpsc::Receiver<PermissionRequest>,
    mut ask_rx: mpsc::Receiver<QuestionRequest>,
    mut ui_rx: mpsc::Receiver<Event>,
) -> anyhow::Result<()> {
    // A 25ms tick lands paste-burst buffers on time without slowing animations.
    let mut ticker = tokio::time::interval(Duration::from_millis(25));
    // Refresh the git status bar every few seconds. The refresh runs on its
    // own task: `git status` can block for seconds on huge repos or network
    // drives, and awaiting it inline in this select would freeze key/event
    // handling for the whole UI (no Esc interrupt, no Ctrl+Q).
    let mut git_ticker = tokio::time::interval(Duration::from_secs(4));
    let (git_tx, mut git_rx) = mpsc::channel::<GitInfo>(1);
    let mut git_in_flight = false;
    let mut dirty = true;
    loop {
        let mut spinner_tick = false;
        tokio::select! {
            _ = git_ticker.tick() => {
                if !git_in_flight {
                    git_in_flight = true;
                    let tx = git_tx.clone();
                    let cwd = app.cwd.clone();
                    tokio::spawn(async move {
                        if let Some(info) = git_info(&cwd).await {
                            let _ = tx.send(info).await;
                        }
                    });
                }
            }
            Some(info) = git_rx.recv() => {
                git_in_flight = false;
                app.git = Some(info);
                dirty = true;
            }
            event = event_rx.recv() => {
                if let Some(event) = event {
                    app.on_agent(event);
                    dirty = true;
                }
            }
            request = perm_rx.recv() => {
                if let Some(request) = request {
                    app.on_permission(request);
                    dirty = true;
                }
            }
            question = ask_rx.recv() => {
                if let Some(question) = question {
                    app.on_question(question);
                    dirty = true;
                }
            }
            ui_event = ui_rx.recv() => {
                if let Some(ui_event) = ui_event {
                    let quit = app.on_ui(ui_event);
                    dirty = true;
                    if quit {
                        break;
                    }
                }
            }
            _ = ticker.tick() => {
                spinner_tick = true;
                // Held/buffered chars land here when due (e.g. after a paste
                // stream ends with no further keys).
                if app.apply_burst_outputs() {
                    dirty = true;
                }
                // A stale Esc confirmation arm expires on its own.
                if app
                    .interrupt_armed_at
                    .is_some_and(|t| t.elapsed() >= Duration::from_secs(5))
                {
                    app.interrupt_armed_at = None;
                    dirty = true;
                }
            }
        }
        // No animation plays while waiting for approval; stop the 25ms
        // redraws to avoid flicker.
        let animate =
            (app.busy || app.ai_thinking) && app.permission.is_none() && app.question.is_none();
        if dirty || (animate && spinner_tick) {
            terminal.draw(|frame| app.render(frame))?;
            dirty = false;
        }
        if app.quit {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::Item;
    use async_trait::async_trait;
    use commands::AgentCmd;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use firment_core::{Agent, ChatMessage, SessionMode, ThinkingLevel, ToolRegistry};
    use paste::PasteBlock;
    use pickers::Selection;
    use ratatui::layout::Rect;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;
    use tokio::sync::oneshot;
    use util::tool_activity;

    fn test_app() -> App {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        // Keep the receiver alive so queued commands with a full/dropped
        // channel signal errors rather than silently failing.
        std::thread::spawn(move || while cmd_rx.blocking_recv().is_some() {});
        App::new(
            cmd_tx,
            Arc::new(Mutex::new(HashSet::new())),
            "test-model".to_string(),
            PathBuf::from("."),
            "default".to_string(),
            ThinkingLevel::Off,
            SessionMode::Agent,
            PathBuf::from("config.toml"),
            None,
            Vec::new(),
        )
    }

    #[tokio::test]
    async fn git_info_reports_branch_changes_and_none_outside_repo() {
        use tokio::process::Command;
        async fn run_git(root: &Path, args: &[&str]) -> std::process::Output {
            Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .await
                .unwrap()
        }
        async fn run_commit(root: &Path, args: &[&str]) -> std::process::Output {
            Command::new("git")
                .args(args)
                .current_dir(root)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .await
                .unwrap()
        }
        // tui has no dev-dependencies; use a unique temp dir and clean up.
        let dir = std::env::temp_dir().join(format!(
            "firment-git-test-{}-{:.6}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_micros()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cleanup = dir.clone();

        // Not a git repo yet -> None.
        assert!(git_info(&dir).await.is_none());

        let init = run_git(&dir, &["init", "-b", "main"]).await;
        assert!(init.status.success(), "git init failed");

        // Dirty work tree -> branch + changes >= 1.
        std::fs::write(dir.join("a.txt"), "hi").unwrap();
        let info = git_info(&dir).await.expect("repo should yield info");
        assert_eq!(info.branch, "main");
        assert!(
            info.changes >= 1,
            "expected dirty changes, got {}",
            info.changes
        );

        // Clean after commit -> changes == 0.
        let add = run_git(&dir, &["add", "a.txt"]).await;
        assert!(add.status.success());
        let commit = run_commit(&dir, &["commit", "-m", "t"]).await;
        assert!(commit.status.success(), "commit failed");
        let clean = git_info(&dir).await.expect("repo still yields info");
        assert_eq!(clean.branch, "main");
        assert_eq!(
            clean.changes, 0,
            "expected clean tree, got {}",
            clean.changes
        );

        let _ = std::fs::remove_dir_all(&cleanup);
    }

    #[test]
    fn input_history_navigates_when_empty_and_scrolls_when_typing() {
        let mut app = test_app();
        app.max_offset = 10;

        app.input = "first".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        app.busy = false; // simulate the turn finishing
        app.input = "second".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert_eq!(app.history, vec!["first", "second"]);

        app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.input.iter().collect::<String>(), "second");
        app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.input.iter().collect::<String>(), "first");
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.input.iter().collect::<String>(), "second");
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert!(app.input.is_empty());

        // Non-empty input: Up/Down keep scrolling the transcript.
        app.scroll = 0;
        app.follow = true;
        app.input = "abc".chars().collect();
        app.cursor = app.input.len();
        app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.scroll, 1);
        assert_eq!(app.input.iter().collect::<String>(), "abc");
    }

    #[test]
    fn esc_while_busy_requires_double_press_and_keeps_input() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let mut app = App::new(
            cmd_tx,
            Arc::new(Mutex::new(HashSet::new())),
            "test-model".to_string(),
            PathBuf::from("."),
            "default".to_string(),
            ThinkingLevel::Off,
            SessionMode::Agent,
            PathBuf::from("config.toml"),
            None,
            Vec::new(),
        );
        app.busy = true;
        app.input = "draft".chars().collect();
        app.cursor = app.input.len();

        // First Esc only arms the confirmation window: no Cancel is sent.
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.interrupting);
        assert!(app.interrupt_armed_at.is_some());
        assert!(cmd_rx.try_recv().is_err());

        // Second Esc inside the window cancels for real. The cancel now fires
        // the pre-extracted handles directly (wired by `run`; not present in
        // this harness) instead of queueing an AgentCmd — so no command is
        // enqueued, but the interrupt is flagged and the draft is kept.
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.interrupting);
        assert!(app.interrupt_armed_at.is_none());
        assert_eq!(app.input.iter().collect::<String>(), "draft");
        assert!(cmd_rx.try_recv().is_err());

        // The interrupt request can only be sent once.
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(cmd_rx.try_recv().is_err());
    }

    #[test]
    fn esc_confirmation_window_expires_without_cancel() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let mut app = App::new(
            cmd_tx,
            Arc::new(Mutex::new(HashSet::new())),
            "test-model".to_string(),
            PathBuf::from("."),
            "default".to_string(),
            ThinkingLevel::Off,
            SessionMode::Agent,
            PathBuf::from("config.toml"),
            None,
            Vec::new(),
        );
        app.busy = true;

        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.interrupt_armed_at.is_some());

        // Simulate the 5s window lapsing; a second Esc re-arms instead of
        // sending Cancel.
        app.interrupt_armed_at = Some(Instant::now() - Duration::from_secs(6));
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.interrupting);
        assert!(app.interrupt_armed_at.is_some());
        assert!(cmd_rx.try_recv().is_err());

        // And the ticker clears a stale arm.
        app.interrupt_armed_at = Some(Instant::now() - Duration::from_secs(6));
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.interrupt_armed_at.is_some());
    }

    #[test]
    fn esc_when_idle_clears_input() {
        let mut app = test_app();
        app.input = "abc".chars().collect();
        app.cursor = 1;
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.input.is_empty());
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn permission_card_is_inline_and_answer_removes_it() {
        let mut app = test_app();
        let (tx, mut rx) = oneshot::channel();
        app.on_permission(PermissionRequest {
            tool: "write_file".to_string(),
            reason: "need to write a file".to_string(),
            reply: tx,
        });
        assert!(matches!(
            app.items.last(),
            Some(Item::Permission { tool, .. }) if tool == "write_file"
        ));

        let rows = app.render_rows(80);
        let text: String = rows
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains("[y] allow"));

        app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(rx.try_recv(), Ok(true));
        assert!(
            !app.items
                .iter()
                .any(|item| matches!(item, Item::Permission { .. }))
        );
        assert!(matches!(
            app.items.last(),
            Some(Item::System(text)) if text.starts_with("✓ Allowed")
        ));
    }

    #[test]
    fn permission_esc_denies_inline_card() {
        let mut app = test_app();
        let (tx, mut rx) = oneshot::channel();
        app.on_permission(PermissionRequest {
            tool: "shell".to_string(),
            reason: "test".to_string(),
            reply: tx,
        });
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(rx.try_recv(), Ok(false));
        assert!(
            !app.items
                .iter()
                .any(|item| matches!(item, Item::Permission { .. }))
        );
    }

    #[test]
    fn concurrent_permissions_are_queued_not_denied() {
        let mut app = test_app();
        let (tx1, mut rx1) = oneshot::channel();
        let (tx2, mut rx2) = oneshot::channel();
        app.on_permission(PermissionRequest {
            tool: "write_file".to_string(),
            reason: "first".to_string(),
            reply: tx1,
        });
        // A second request arrives while the first is still on screen.
        app.on_permission(PermissionRequest {
            tool: "shell".to_string(),
            reason: "second".to_string(),
            reply: tx2,
        });
        assert_eq!(
            rx1.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty),
            "first request must not be denied by the second"
        );
        assert_eq!(app.permission_queue.len(), 1);
        // The second request must not be visible yet.
        assert!(
            !app.items
                .iter()
                .any(|item| matches!(item, Item::Permission { tool, .. } if tool == "shell")),
            "queued permission must not be shown before the first is answered"
        );

        // Answer the first: allow it.
        app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(rx1.try_recv(), Ok(true));
        // The queued request now pops up, awaiting the user.
        assert_eq!(
            rx2.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty),
            "queued request must wait for its own answer"
        );
        assert!(app.permission_queue.is_empty());
        assert!(matches!(
            app.items.last(),
            Some(Item::Permission { tool, .. }) if tool == "shell"
        ));
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(rx2.try_recv(), Ok(false));
        assert!(
            !app.items
                .iter()
                .any(|item| matches!(item, Item::Permission { .. }))
        );
    }

    #[test]
    fn question_modal_answers_by_option_key() {
        let mut app = test_app();
        let (tx, mut rx) = oneshot::channel();
        app.on_question(QuestionRequest {
            question: "which chip?".to_string(),
            options: vec!["stm32f407".to_string(), "stm32g0".to_string()],
            reply: tx,
        });
        assert!(matches!(
            app.items.last(),
            Some(Item::System(text)) if text.contains("which chip?")
        ));
        assert!(app.question.is_some());

        app.on_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        assert_eq!(rx.try_recv(), Ok(Some("stm32g0".to_string())));
        assert!(app.question.is_none());
    }

    #[test]
    fn question_modal_digits_inside_typed_answer_do_not_pick_options() {
        let mut app = test_app();
        let (tx, mut rx) = oneshot::channel();
        app.on_question(QuestionRequest {
            question: "which board?".to_string(),
            options: vec!["f103".to_string(), "f407".to_string(), "g431".to_string()],
            reply: tx,
        });
        for ch in ['n', 'u', 'c', 'l', 'e', 'o', ' ', 'g', '4', '3', '1'] {
            app.on_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert!(
            app.question.is_some(),
            "typing an answer with a digit in it must not pick an option"
        );
        assert_eq!(app.question_input.iter().collect::<String>(), "nucleo g431");
        assert!(
            rx.try_recv().is_err(),
            "typing a digit into the answer must not answer yet"
        );
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(rx.try_recv(), Ok(Some("nucleo g431".to_string())));
        assert!(app.question.is_none());
    }

    #[test]
    fn question_modal_accepts_free_form_answer_and_esc_dismisses() {
        let mut app = test_app();
        let (tx, mut rx) = oneshot::channel();
        app.on_question(QuestionRequest {
            question: "which toolchain?".to_string(),
            options: Vec::new(),
            reply: tx,
        });
        for ch in ['g', 'n', 'u', '-', 'r', 'm'] {
            app.on_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(app.question_input.iter().collect::<String>(), "gnu-rm");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(rx.try_recv(), Ok(Some("gnu-rm".to_string())));
        assert!(app.question.is_none());

        let (tx, mut rx) = oneshot::channel();
        app.on_question(QuestionRequest {
            question: "still there?".to_string(),
            options: Vec::new(),
            reply: tx,
        });
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(rx.try_recv(), Ok(None));
        assert!(app.question.is_none());
    }

    #[test]
    fn input_layout_wraps_long_text_and_tracks_cursor() {
        let mut app = test_app();
        // 12 ASCII + 2 CJK chars: width = 12 + 4 = 16, so a 4-column wrap puts
        // 4 chars per line.
        app.input = "abcdefghijkl你好".chars().collect();
        app.cursor = app.input.len();
        let (lines, line_starts, cursor_line, cursor_col) = app.input_layout(4);
        assert_eq!(lines, vec!["abcd", "efgh", "ijkl", "你好"]);
        assert_eq!(line_starts, vec![0, 4, 8, 12]);
        assert_eq!(cursor_line, 3);
        assert_eq!(cursor_col, 4);

        // Cursor in the middle: column counts cell widths.
        app.input = "你abc".chars().collect();
        app.cursor = 3; // CJK char 你(width 2) + a(1) + b(1) → col 4
        let (_, _, cursor_line, cursor_col) = app.input_layout(10);
        assert_eq!(cursor_line, 0);
        assert_eq!(cursor_col, 4);
    }

    #[test]
    fn move_input_cursor_moves_between_wrapped_lines() {
        let mut app = test_app();
        app.input_width = 4;
        app.input = "abcdefghijkl".chars().collect();
        app.cursor = app.input.len(); // lines: abcd / efgh / ijkl, cursor at end of line 2

        app.move_input_cursor(-1);
        assert_eq!(app.cursor, 8); // end of previous line (efgh)
        app.move_input_cursor(-1);
        assert_eq!(app.cursor, 4); // end of the line before that (abcd)
        app.move_input_cursor(1);
        assert_eq!(app.cursor, 8); // back to the end of efgh
    }

    #[test]
    fn paste_collapses_long_text_into_placeholder() {
        let mut app = test_app();
        let text = "a\nb\nc\nd";
        app.insert_text_at_cursor(text, true);
        assert_eq!(app.input.iter().collect::<String>(), "【line 1-4】");
        assert_eq!(app.expand_input(), text);
        assert_eq!(app.cursor, "【line 1-4】".chars().count());

        // Moving left from the placeholder tail skips the whole placeholder;
        // the same applies to the right.
        app.move_cursor_left();
        assert_eq!(app.cursor, 0);
        app.move_cursor_right();
        assert_eq!(app.cursor, "【line 1-4】".chars().count());

        // Backspace at the placeholder tail deletes it as a whole.
        app.backspace();
        assert!(app.input.is_empty());
        assert!(app.paste_blocks.is_empty());
    }

    #[test]
    fn paste_small_text_stays_inline() {
        let mut app = test_app();
        app.insert_text_at_cursor("hello", true);
        assert_eq!(app.input.iter().collect::<String>(), "hello");
        assert!(app.paste_blocks.is_empty());
    }

    #[test]
    fn paste_burst_multi_line_flow_is_collapsed_not_submitted() {
        let mut app = test_app();
        let t0 = Instant::now() - Duration::from_millis(1000);
        let ch = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);

        // Simulate Windows Terminal injecting a multi-line paste as a fast
        // keystroke stream (ending with Enter).
        app.on_key_burst(ch('a'), t0);
        app.on_key_burst(ch('b'), t0 + Duration::from_millis(10));
        app.on_key_burst(enter, t0 + Duration::from_millis(20));
        app.on_key_burst(ch('c'), t0 + Duration::from_millis(30));
        app.on_key_burst(enter, t0 + Duration::from_millis(40));
        app.on_key_burst(ch('d'), t0 + Duration::from_millis(50));

        // Before the buffer flushes: nothing is submitted and no partial text
        // appears in the input.
        assert!(app.items.is_empty());
        assert!(app.input.is_empty());

        // After the idle timeout, the whole buffer goes through collapsed
        // paste.
        app.apply_burst_outputs_at(t0 + Duration::from_millis(300));
        assert_eq!(app.input.iter().collect::<String>(), "【line 1-3】");
        assert_eq!(app.expand_input(), "ab\nc\nd");
        assert!(app.items.is_empty());
    }

    #[test]
    fn paste_burst_single_char_enter_does_not_submit() {
        let mut app = test_app();
        let t0 = Instant::now() - Duration::from_millis(1000);
        app.on_key_burst(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE), t0);
        app.on_key_burst(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            t0 + Duration::from_millis(10),
        );
        assert!(app.items.is_empty());
        assert_eq!(app.input.iter().collect::<String>(), "x\n");
    }

    #[test]
    fn paste_burst_slow_typing_inserts_normally() {
        let mut app = test_app();
        let t0 = Instant::now() - Duration::from_millis(1000);
        let ch = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);

        app.on_key_burst(ch('a'), t0);
        assert!(app.input.is_empty()); // first ASCII char is held
        assert!(app.apply_burst_outputs_at(t0 + Duration::from_millis(50)));
        assert_eq!(app.input.iter().collect::<String>(), "a");

        app.on_key_burst(ch('b'), t0 + Duration::from_millis(100));
        app.on_key_burst(ch('c'), t0 + Duration::from_millis(200));
        assert_eq!(app.input.iter().collect::<String>(), "ab");
        assert!(app.apply_burst_outputs_at(t0 + Duration::from_millis(250)));
        assert_eq!(app.input.iter().collect::<String>(), "abc");
        assert!(app.paste_blocks.is_empty());
    }

    #[test]
    fn paste_burst_non_ascii_retro_captures_inserted_prefix() {
        let mut app = test_app();
        let t0 = Instant::now() - Duration::from_millis(1000);
        app.input = "AB".chars().collect();
        app.cursor = 1;

        app.on_key_burst(KeyEvent::new(KeyCode::Char('你'), KeyModifiers::NONE), t0);
        assert_eq!(app.input.iter().collect::<String>(), "A你B");

        // A second char arriving quickly reclaims the inserted first char and
        // moves both into the buffer.
        app.on_key_burst(
            KeyEvent::new(KeyCode::Char('好'), KeyModifiers::NONE),
            t0 + Duration::from_millis(10),
        );
        assert_eq!(app.input.iter().collect::<String>(), "AB");

        app.apply_burst_outputs_at(t0 + Duration::from_millis(300));
        assert_eq!(app.input.iter().collect::<String>(), "A你好B");
        assert!(app.items.is_empty());
    }

    #[test]
    fn paste_burst_enter_after_flush_is_newline_within_window() {
        let mut app = test_app();
        let t0 = Instant::now() - Duration::from_millis(1000);
        let ch = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);

        app.on_key_burst(ch('a'), t0);
        app.on_key_burst(ch('b'), t0 + Duration::from_millis(10));
        assert!(app.apply_burst_outputs_at(t0 + Duration::from_millis(200)));
        assert_eq!(app.input.iter().collect::<String>(), "ab");

        // Enter within 100ms after flush: treated as a newline, not sent.
        app.on_key_burst(enter, t0 + Duration::from_millis(250));
        assert!(app.items.is_empty());
        assert_eq!(app.input.iter().collect::<String>(), "ab\n");

        // After the protection window, Enter submits normally.
        app.on_key_burst(enter, t0 + Duration::from_millis(500));
        assert!(app.input.is_empty());
        assert!(matches!(
            app.items.last(),
            Some(Item::User(text)) if text == "ab"
        ));
    }

    #[test]
    fn paste_burst_modified_keys_pass_through_and_clear_state() {
        let mut app = test_app();
        let t0 = Instant::now() - Duration::from_millis(1000);
        app.on_key_burst(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), t0);
        // Ctrl+P bypasses the burst and flushes the held char.
        let quit = app.on_key_burst(
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
            t0 + Duration::from_millis(5),
        );
        assert!(!quit);
        assert!(app.model_picker.is_some());
        assert_eq!(app.input.iter().collect::<String>(), "a");
    }

    #[test]
    fn paste_burst_held_char_is_not_lost_on_other_keys() {
        let mut app = test_app();
        let t0 = Instant::now() - Duration::from_millis(1000);
        app.on_key_burst(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), t0);
        app.on_key_burst(
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            t0 + Duration::from_millis(5),
        );
        assert_eq!(app.input.iter().collect::<String>(), "a");
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn new_command_clears_input_and_starts_fresh_session() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let mut app = App::new(
            cmd_tx,
            Arc::new(Mutex::new(HashSet::new())),
            "test-model".to_string(),
            PathBuf::from("."),
            "default".to_string(),
            ThinkingLevel::Off,
            SessionMode::Agent,
            PathBuf::from("config.toml"),
            None,
            Vec::new(),
        );
        app.input = "old draft".chars().collect();
        app.cursor = app.input.len();
        app.items.push(Item::User("old message".to_string()));
        app.busy = true;

        app.run_command("new");
        assert!(app.input.is_empty());
        assert_eq!(app.cursor, 0);
        assert!(app.paste_blocks.is_empty());
        assert!(app.items.iter().any(|i| matches!(
            i,
            Item::System(text) if text == "Starting a new conversation…"
        )));
        assert!(!app.busy);
        assert!(app.pending_new_session);
        // A running turn is cancelled first so the fresh session is processed
        // without waiting for the old turn to finish.
        match cmd_rx.try_recv().unwrap() {
            AgentCmd::Cancel => {}
            _ => panic!("expected Cancel"),
        }
        match cmd_rx.try_recv().unwrap() {
            AgentCmd::NewSession => {}
            _ => panic!("expected NewSession"),
        }
    }

    #[test]
    fn new_command_suppresses_old_turn_events_until_session_loaded() {
        let (cmd_tx, _cmd_rx) = mpsc::channel(16);
        let mut app = App::new(
            cmd_tx,
            Arc::new(Mutex::new(HashSet::new())),
            "test-model".to_string(),
            PathBuf::from("."),
            "default".to_string(),
            ThinkingLevel::Off,
            SessionMode::Agent,
            PathBuf::from("config.toml"),
            None,
            Vec::new(),
        );
        app.items.push(Item::User("old blue message".to_string()));
        app.items.push(Item::Assistant("old reply".to_string()));

        app.run_command("new");
        // Old-turn events arriving before the fresh session must not leak back
        // into the cleared transcript.
        app.on_agent(AgentEvent::TextDelta("stale delta".to_string()));
        app.on_agent(AgentEvent::Info("stale info".to_string()));
        app.on_agent(AgentEvent::ToolStart {
            name: "read_file".to_string(),
            args: serde_json::json!({}),
            seq: 1,
        });
        assert_eq!(app.items.len(), 1); // only the "Starting…" hint
        assert!(matches!(
            app.items.last(),
            Some(Item::System(text)) if text == "Starting a new conversation…"
        ));

        // A message typed and sent while the fresh session is loading must
        // survive the transcript clear.
        app.input = "new message".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert!(matches!(app.items.last(), Some(Item::User(text)) if text == "new message"));

        let fresh = Session::new(PathBuf::from("."), "default", "m");
        app.on_agent(AgentEvent::SessionLoaded(fresh));
        assert!(!app.pending_new_session);
        assert_eq!(app.items.len(), 2); // confirmation + the user's new message
        assert!(matches!(
            app.items[0],
            Item::System(ref text) if text == "New conversation started"
        ));
        assert!(matches!(
            app.items[1],
            Item::User(ref text) if text == "new message"
        ));
    }

    #[test]
    fn shift_enter_inserts_newline_instead_of_sending() {
        let mut app = test_app();
        app.input = "abc".chars().collect();
        app.cursor = app.input.len();
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(app.input.iter().collect::<String>(), "abc\n");
        assert!(!app.busy);
        assert!(app.items.is_empty());
    }

    #[test]
    fn input_selection_expands_collapsed_blocks() {
        let mut app = test_app();
        app.input = "A【line 1-2】B".chars().collect();
        app.paste_blocks.push(PasteBlock {
            placeholder: "【line 1-2】".to_string(),
            text: "x\ny".to_string(),
        });
        app.input_sel = Some((0, app.input.len()));
        assert_eq!(app.input_selection_text().unwrap(), "Ax\nyB");
    }

    #[test]
    fn model_picker_filters_and_selects() {
        let mut app = test_app();
        app.on_agent(AgentEvent::Models(vec![
            "deepseek-v4-flash".to_string(),
            "deepseek-v4-pro".to_string(),
            "gpt-xhigh".to_string(),
        ]));
        assert!(app.model_picker.is_some());
        assert_eq!(
            app.model_picker.as_ref().unwrap().selected_model().unwrap(),
            "deepseek-v4-flash"
        );

        app.on_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(
            app.model_picker.as_ref().unwrap().filtered(),
            vec!["gpt-xhigh"]
        );
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.model_picker.is_none());
        assert_eq!(app.model, "gpt-xhigh");
    }

    #[test]
    fn mouse_selection_extracts_rendered_text() {
        let mut app = test_app();
        app.items.push(Item::Assistant("hello world".to_string()));
        app.items.push(Item::Assistant("second line".to_string()));
        app.transcript_rect = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 6,
        };
        app.content_width = 28;
        app.max_offset = 0;
        app.follow = true;

        let across_rows = Selection {
            anchor_row: 0,
            anchor_col: 0,
            row: 2,
            col: 2,
        };
        assert_eq!(app.selection_text(across_rows), "hello world\n\nse");

        let within_row = Selection {
            anchor_row: 0,
            anchor_col: 6,
            row: 0,
            col: 11,
        };
        assert_eq!(app.selection_text(within_row), "world");
    }

    #[test]
    fn selection_uses_cell_widths_for_cjk() {
        let mut app = test_app();
        app.items.push(Item::Assistant("你好世界 ok".to_string()));
        app.transcript_rect = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 5,
        };
        app.content_width = 38;
        app.max_offset = 0;
        app.follow = true;

        // Cells: 你(0-2) 好(2-4) 世(4-6) 界(6-8) space(8) o(9) k(10)
        let first_four = Selection {
            anchor_row: 0,
            anchor_col: 0,
            row: 0,
            col: 8,
        };
        assert_eq!(app.selection_text(first_four), "你好世界");

        let mixed = Selection {
            anchor_row: 0,
            anchor_col: 4,
            row: 0,
            col: 9,
        };
        assert_eq!(app.selection_text(mixed), "世界 ");
    }

    #[test]
    fn session_picker_navigates_and_loads_selected() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let mut app = App::new(
            cmd_tx,
            Arc::new(Mutex::new(HashSet::new())),
            "test-model".to_string(),
            PathBuf::from("."),
            "default".to_string(),
            ThinkingLevel::Off,
            SessionMode::Agent,
            PathBuf::from("config.toml"),
            None,
            Vec::new(),
        );
        app.on_agent(AgentEvent::Sessions(vec![
            firment_core::SessionSummary {
                id: "11111111-aaaa".to_string(),
                updated_at: 1,
                model: "m1".to_string(),
                cwd: PathBuf::from("."),
                preview: "first".to_string(),
            },
            firment_core::SessionSummary {
                id: "22222222-bbbb".to_string(),
                updated_at: 2,
                model: "m2".to_string(),
                cwd: PathBuf::from("."),
                preview: "second".to_string(),
            },
        ]));
        assert!(app.session_picker.is_some());

        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.session_picker.is_none());
        match cmd_rx.try_recv().unwrap() {
            AgentCmd::LoadSession(id) => assert_eq!(id, "22222222-bbbb"),
            _ => panic!("expected LoadSession"),
        }
    }

    #[test]
    fn last_output_text_returns_most_recent_assistant_message() {
        let mut app = test_app();
        assert!(app.last_output_text().is_none());
        app.items.push(Item::Assistant("first".to_string()));
        app.items.push(Item::System("note".to_string()));
        app.items.push(Item::Assistant("second".to_string()));
        assert_eq!(app.last_output_text().unwrap(), "second");
    }

    #[test]
    fn session_loaded_repopulates_transcript() {
        let mut app = test_app();
        let mut session = Session::new(PathBuf::from("."), "default", "m");
        session.push(ChatMessage::User {
            content: "hello".to_string(),
        });
        session.push(ChatMessage::Assistant {
            content: "reply".to_string(),
            tool_calls: Vec::new(),
        });
        session.push(ChatMessage::Tool {
            tool_call_id: "c1".to_string(),
            name: "read_file".to_string(),
            content: "ok".to_string(),
        });

        app.on_agent(AgentEvent::SessionLoaded(session));
        assert!(matches!(&app.items[0], Item::User(t) if t == "hello"));
        assert!(matches!(&app.items[1], Item::Assistant(t) if t == "reply"));
        assert!(matches!(
            &app.items[2],
            Item::Tool {
                name,
                running: false,
                ok: true,
                ..
            } if name == "read_file"
        ));
    }

    #[test]
    fn plan_mode_toggle_updates_status_through_settings() {
        let mut app = test_app();
        assert_eq!(app.mode, SessionMode::Agent);
        app.on_agent(AgentEvent::Settings {
            provider: None,
            model: None,
            thinking: None,
            mode: Some(SessionMode::Plan),
        });
        assert_eq!(app.mode, SessionMode::Plan);
    }

    #[test]
    fn plan_command_toggles_both_ways_and_agent_is_explicit() {
        let mut app = test_app();
        app.input = "/plan".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert_eq!(app.mode, SessionMode::Plan);

        app.input = "/plan".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert_eq!(app.mode, SessionMode::Agent);

        app.input = "/plan on".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert_eq!(app.mode, SessionMode::Plan);

        app.input = "/plan off".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert_eq!(app.mode, SessionMode::Agent);

        app.input = "/plan on".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert_eq!(app.mode, SessionMode::Plan);

        app.input = "/agent".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert_eq!(app.mode, SessionMode::Agent);
    }

    #[test]
    fn scroll_counts_rows_away_from_bottom_and_clamps_at_top() {
        let mut app = test_app();
        app.max_offset = 80;
        assert!(app.follow);

        app.scroll_up(3);
        assert!(!app.follow);
        assert_eq!(app.scroll, 3);

        app.scroll_up(5);
        assert_eq!(app.scroll, 8);

        // cannot scroll past the top of the transcript
        app.scroll_up(1000);
        assert_eq!(app.scroll, 80);
        assert!(!app.follow);

        app.scroll_down(80);
        assert_eq!(app.scroll, 0);
        assert!(app.follow);

        // nothing to scroll when the whole transcript fits on screen
        app.max_offset = 0;
        app.scroll_up(10);
        assert!(app.follow);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn tool_activity_names_common_tools_and_targets() {
        assert_eq!(
            tool_activity("grep", &serde_json::json!({ "pattern": "fn main" })),
            "searching fn main…"
        );
        assert_eq!(
            tool_activity("glob", &serde_json::json!({ "path": "src/main.rs" })),
            "searching main.rs…"
        );
        assert_eq!(
            tool_activity(
                "flash",
                &serde_json::json!({ "file": "target/thumbv7em/debug/app.elf" })
            ),
            "flashing app.elf…"
        );
        assert_eq!(
            tool_activity("monitor", &serde_json::json!({ "port": "COM3" })),
            "monitoring serial…"
        );
        assert_eq!(
            tool_activity("read_file", &serde_json::json!({ "path": "" })),
            "reading…"
        );
        // Unknown tools fall back to their raw name.
        assert_eq!(tool_activity("deploy", &serde_json::json!({})), "deploy…");
    }

    #[test]
    fn status_tracks_active_tools_with_count() {
        let mut app = test_app();
        assert_eq!(app.status_text(), "ready");

        app.on_agent(AgentEvent::TurnStart);
        assert_eq!(app.status_text(), "thinking");

        app.on_agent(AgentEvent::ToolStart {
            name: "grep".to_string(),
            args: serde_json::json!({ "pattern": "fn main" }),
            seq: 1,
        });
        assert_eq!(app.status_text(), "working · searching fn main…");

        app.on_agent(AgentEvent::ToolStart {
            name: "flash".to_string(),
            args: serde_json::json!({ "file": "app.elf" }),
            seq: 2,
        });
        assert_eq!(app.status_text(), "working · 2× flashing app.elf…");

        app.on_agent(AgentEvent::ToolEnd {
            name: "flash".to_string(),
            ok: true,
            summary: String::new(),
            seq: 2,
        });
        assert_eq!(app.status_text(), "working · searching fn main…");

        app.on_agent(AgentEvent::ToolEnd {
            name: "grep".to_string(),
            ok: true,
            summary: String::new(),
            seq: 1,
        });
        assert_eq!(app.status_text(), "working");
    }

    /// Provider whose stream never ends on its own: run_turn blocks inside its
    /// per-event select until the turn-level cancel/watch fires — exactly the
    /// stale spot where Esc used to be swallowed by the serial command loop.
    struct StallProvider {
        calls: Arc<AtomicUsize>,
        started: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl firment_core::Provider for StallProvider {
        async fn stream(
            &self,
            _request: firment_core::ChatRequest,
        ) -> Result<firment_core::ProviderStream, firment_core::ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            // A real provider awaits the network before its stream becomes
            // ready. Without this await, the `select!` in run_turn would
            // always prefer the (immediately-ready) stream branch and never
            // expose a premature cancel saying "changed".
            tokio::time::sleep(Duration::from_millis(50)).await;
            let stream = futures::stream::unfold((), |()| async move {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Some((Err(firment_core::ProviderError::StreamEnded), ()))
            });
            Ok(Box::pin(stream))
        }

        fn model(&self) -> &str {
            "stall"
        }
    }

    /// Sends a prompt, waits until `provider.stream` is entered `expected`
    /// times, then asserts the turn is genuinely still streaming: the first
    /// event must be `TurnStart` and no second event may arrive within 150ms.
    /// This catches "cancelled" state leaks that would end the turn right at
    /// its start (e.g. a stale watch version making `changed()` fire early).
    async fn start_turn_and_expect_live_stream(
        cmd_tx: &mpsc::Sender<AgentCmd>,
        calls: &Arc<AtomicUsize>,
        expected: usize,
        event_rx: &mut mpsc::Receiver<AgentEvent>,
    ) {
        cmd_tx.send(AgentCmd::User("go".to_string())).await.unwrap();
        for _ in 0..500 {
            if calls.load(Ordering::SeqCst) >= expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            calls.load(Ordering::SeqCst) >= expected,
            "provider.stream must be entered ({expected} times)"
        );
        match tokio::time::timeout(Duration::from_secs(1), event_rx.recv()).await {
            Ok(Some(AgentEvent::TurnStart)) => {}
            Ok(Some(other)) => panic!("first event must be TurnStart, got {other:?}"),
            Ok(None) => panic!("event stream closed"),
            Err(_) => panic!("no TurnStart within 1s"),
        }
        let premature = tokio::time::timeout(Duration::from_millis(150), event_rx.recv()).await;
        assert!(
            premature.is_err(),
            "turn must stay live while streaming, got premature event {premature:?}"
        );
    }

    static NEXT_HARNESS_DIR: AtomicUsize = AtomicUsize::new(0);

    fn spawn_agent_task_harness(
        provider: Box<dyn firment_core::Provider>,
    ) -> (
        mpsc::Sender<AgentCmd>,
        mpsc::Receiver<AgentEvent>,
        tokio::task::JoinHandle<()>,
    ) {
        let (event_tx, event_rx) = mpsc::channel(256);
        // Unique per invocation: two harness tests run in parallel and must
        // not race each other's session files.
        let dir = std::env::temp_dir().join(format!(
            "firment-tui-stall-{}-{}",
            std::process::id(),
            NEXT_HARNESS_DIR.fetch_add(1, Ordering::SeqCst)
        ));
        let store = SessionStore::new(dir.clone());
        let session = Session::new(dir, "default", "stall");
        let agent = Agent::new(
            Some(provider),
            Arc::new(ToolRegistry::new()),
            session,
            store.clone(),
            Arc::new(firment_core::AutoApprove::everything()),
            Arc::new(ChannelSink { tx: event_tx }),
            10,
        );
        let (cancel_tx, cancel_signal) = agent.cancel_handle();
        let agent = Arc::new(tokio::sync::Mutex::new(agent));
        let turn_lock = Arc::new(tokio::sync::Mutex::new(()));
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let task = spawn_agent_task(
            cmd_rx,
            agent,
            cancel_tx,
            cancel_signal,
            turn_lock,
            store,
            firment_core::Config::default_with_provider(
                "default",
                firment_core::ProviderConfig {
                    r#type: "openai".to_string(),
                    base_url: None,
                    api_key_env: None,
                    api_key: None,
                    model: "stall".to_string(),
                    max_tokens: None,
                    temperature: None,
                },
            ),
            std::env::temp_dir().join("firment-tui-stall.toml"),
            firment_tools::plan_registry(),
            firment_tools::default_registry(),
            Arc::new(firment_core::AutoApprove::everything()),
            Arc::new(firment_core::AutoApprove::everything()),
        );
        (cmd_tx, event_rx, task)
    }

    #[tokio::test]
    async fn cancel_command_interrupts_a_running_turn() {
        let calls = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(tokio::sync::Notify::new());
        let (cmd_tx, mut event_rx, task) = spawn_agent_task_harness(Box::new(StallProvider {
            calls: calls.clone(),
            started: started.clone(),
        }));

        // Turn 1: must actually be streaming (TurnStart then silence) before
        // Cancel arrives, otherwise a stale cancel state would show up here as
        // a premature end-of-turn event.
        start_turn_and_expect_live_stream(&cmd_tx, &calls, 1, &mut event_rx).await;

        let begin = std::time::Instant::now();
        cmd_tx.send(AgentCmd::Cancel).await.unwrap();
        let mut ended = false;
        for _ in 0..8 {
            match tokio::time::timeout(Duration::from_secs(5), event_rx.recv()).await {
                Ok(Some(AgentEvent::TurnEnd { .. })) => {
                    ended = true;
                    break;
                }
                Ok(Some(_)) => continue,
                _ => break,
            }
        }
        assert!(ended, "turn must end after its Cancel");
        assert!(
            begin.elapsed() < Duration::from_secs(4),
            "turn must end within 4s of Cancel"
        );

        // Turn 2: reset_cancel fired; a second turn streams normally and a
        // second Cancel stops it too.
        start_turn_and_expect_live_stream(&cmd_tx, &calls, 2, &mut event_rx).await;
        cmd_tx.send(AgentCmd::Cancel).await.unwrap();
        let mut ended = false;
        for _ in 0..8 {
            match tokio::time::timeout(Duration::from_secs(5), event_rx.recv()).await {
                Ok(Some(AgentEvent::TurnEnd { .. })) => {
                    ended = true;
                    break;
                }
                Ok(Some(_)) => continue,
                _ => break,
            }
        }
        assert!(ended, "second turn must end after its Cancel");
        drop(cmd_tx);
        task.await.unwrap();
    }

    struct ErrorProvider;

    #[async_trait]
    impl firment_core::Provider for ErrorProvider {
        async fn stream(
            &self,
            _request: firment_core::ChatRequest,
        ) -> Result<firment_core::ProviderStream, firment_core::ProviderError> {
            Err(firment_core::ProviderError::Api {
                status: 500,
                message: "boom".to_string(),
            })
        }

        fn model(&self) -> &str {
            "error"
        }
    }

    #[tokio::test]
    async fn turn_error_still_closes_the_turn() {
        let (cmd_tx, mut event_rx, task) = spawn_agent_task_harness(Box::new(ErrorProvider));
        cmd_tx.send(AgentCmd::User("go".to_string())).await.unwrap();
        let mut saw_error = false;
        let mut saw_turn_end = false;
        for _ in 0..4 {
            if saw_error && saw_turn_end {
                break;
            }
            match tokio::time::timeout(Duration::from_secs(5), event_rx.recv()).await {
                Ok(Some(AgentEvent::Error(_))) => saw_error = true,
                Ok(Some(AgentEvent::TurnEnd { .. })) => saw_turn_end = true,
                Ok(Some(_)) => {}
                _ => break,
            }
        }
        assert!(saw_error, "provider failure must surface as Error");
        assert!(
            saw_turn_end,
            "failed turn must still emit TurnEnd so the TUI un-busies"
        );
        drop(cmd_tx);
        task.await.unwrap();
    }
}
