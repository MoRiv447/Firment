import { useEffect, useRef, useState } from 'react';
import { ArrowDown, Bot, Brain } from 'lucide-react';

import { LiveRun } from '../components/LiveRun';
import { Markdown } from '../components/Markdown';
import { MessageList } from '../components/MessageList';
import { StepProgress } from '../components/StepProgress';
import { formatDuration } from '../lib/format';
import { shouldShowStallNotice, stallNotice } from '../lib/stallHint';
import { workflowSteps } from '../lib/steps';
import { recordCompleted } from '../lib/timing';
import type { RunningTurn, SessionDto } from '../types';
import { Button, Callout, Chip, EmptyState, Eyebrow, Icon, Menu, Spinner, TextArea } from '../ui';
import styles from './ChatView.module.css';

/**
 * The two settings the composer carries, and their vocabulary.
 *
 * They live here rather than being passed in because a control owns its own options:
 * the labels are what the chip and the menu say, and a caller that supplied them
 * would be a second place they could be spelled differently. The *value* still comes
 * from the session and the *handler* still comes from the shell -- this is only the
 * list of things that can be chosen.
 */
const MODE_OPTIONS = ['agent', 'plan'] as const;
const THINKING_LEVELS = ['off', 'low', 'medium', 'high', 'xhigh', 'max'] as const;

/**
 * The chat pane: the transcript, the live turn above it, and the composer below.
 *
 * Three things in here are load-bearing and easy to break by accident, so they
 * are the parts this file keeps on the record:
 *
 * * **Stick-to-bottom scrolling.** The stream follows only while the user is at
 *   the bottom, and a scroll up detaches until they re-engage -- a transcript
 *   that jumps while you are reading the middle of it is unusable.
 * * **The stall notice.** It waits until past the agent's own stream budget,
 *   because a model writing one enormous tool-call argument is silent without
 *   anything being wrong.
 * * **The two timers.** The row next to the spinner used to reset whenever any
 *   text arrived, so a chatty stream kept reporting "0s" while a 90-second build
 *   ran underneath it. One number is the running tool's own elapsed time, the
 *   other is the idle gap, and they are never the same reading.
 */
export function ChatView({
  session,
  running,
  turn,
  infos,
  onSend,
  onCancel,
  onMode,
  onThinking,
  progress,
}: {
  session: SessionDto | null;
  running: boolean;
  turn: RunningTurn | null;
  infos: { id: number; text: string }[];
  onSend: (input: string) => void;
  onCancel: () => void;
  /** Switch agent/plan. Absent in a read-only rendering, where the chips are hidden. */
  onMode?: (mode: string) => void;
  onThinking?: (level: string) => void;
  /**
   * The plan's own progress, when the session has one. It outranks the tool-call
   * count above the transcript because it is the number that answers "how much is
   * left", where a call count only answers "how much has happened" -- and a turn
   * that retried a failing build twice inflates the second without touching the
   * first.
   */
  progress?: { done: number; total: number } | null;
}) {
  const [input, setInput] = useState('');
  const [thinkOpen, setThinkOpen] = useState(false);
  const thinkAnchorRef = useRef<HTMLButtonElement | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  // Seconds since the last visible event, ticking every second while running —
  // the user can watch this climb to tell a slow model from a wedged turn.
  const [idleSecs, setIdleSecs] = useState(0);
  // The step row's clock. It rides the interval below rather than opening a second
  // one: two tickers a second apart would let the row's counter and `idleSecs`
  // disagree, and the view only needs to re-render at one rate.
  const [nowMs, setNowMs] = useState(() => Date.now());

  // Detect a stuck agent: running is on but nothing has changed in the
  // visible turn (text delta, new tool, or a tool finishing) for too long.
  // Tool STATUS transitions count as change: a 90s build is a running tool,
  // not a wedged turn. What does NOT count here is a model writing one huge
  // tool-call argument — which is why the notice waits until past the agent's
  // own 120s stream budget (see STALL_NOTICE_SECS) rather than crying at 60s.
  const lastChangeRef = useRef<number>(Date.now());
  const [stuck, setStuck] = useState(false);
  const notice = stallNotice(idleSecs);
  const turnKey = `${turn?.text.length}:${
    turn?.thinking.length ?? 0
  }:${turn?.tools ? Object.values(turn.tools).map((t) => t.status).join('') : ''}`;
  useEffect(() => {
    lastChangeRef.current = Date.now();
    setStuck(false);
    setIdleSecs(0);
  }, [turnKey]);
  useEffect(() => {
    if (!running) {
      setStuck(false);
      setIdleSecs(0);
      return;
    }
    const tick = setInterval(() => {
      const idle = Math.floor((Date.now() - lastChangeRef.current) / 1000);
      setIdleSecs(idle);
      setNowMs(Date.now());
      if (shouldShowStallNotice(idle)) setStuck(true);
    }, 1000);
    return () => clearInterval(tick);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [running]);

  const stickRef = useRef(true);
  const rafRef = useRef<number | null>(null);
  const [detached, setDetached] = useState(false);
  const followIfStuck = () => {
    if (!stickRef.current) return;
    const el = scrollRef.current;
    if (!el) return;
    if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      el.scrollTop = el.scrollHeight;
      rafRef.current = null;
    });
  };
  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 80;
    stickRef.current = atBottom;
    setDetached(!atBottom);
  };
  useEffect(() => {
    followIfStuck();
    return () => {
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
    };
  }, [session?.messages.length, turn?.text, turn?.tools]);

  // A different chat is a different scroll position. `stickRef` and `detached`
  // describe how the READER sits in this conversation, so carrying them across a
  // switch opened the new chat at the old offset — with following off and
  // "Jump to bottom" showing, which reads as the app ignoring a reply it is not
  // ignoring. Keyed on the id, not the message count: two chats can have the same
  // number of messages, and nothing else here changes.
  useEffect(() => {
    stickRef.current = true;
    setDetached(false);
    followIfStuck();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session?.id]);

  const jumpToBottom = () => {
    stickRef.current = true;
    setDetached(false);
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  };

  const send = () => {
    const trimmed = input.trim();
    if (!trimmed || running) return;
    setInput('');
    onSend(trimmed);
  };

  const toolList = turn ? Object.values(turn.tools) : [];
  // Every finished run feeds the estimate the step row may show, and the record is
  // idempotent per (session, seq), so it can run on each render instead of needing a
  // diff of the tools list against the previous one.
  useEffect(() => {
    // Scoped by session: `seq` restarts with every agent, so without the id the second
    // session's runs would look like ones already counted -- the estimate would go
    // quiet after the first session, which is the one thing the ledger is for.
    recordCompleted(toolList, session?.id ?? 'unsaved');
  });
  // Where the embedded workflow got to, computed from those same tools rather
  // than tracked separately -- see `lib/steps.ts`. Null for a chat that never
  // builds anything, so the row does not appear as empty furniture. `nowMs` is what
  // makes a running step's elapsed count up.
  const steps = workflowSteps(toolList, nowMs);
  /*
   * Every tool call the transcript already holds.
   *
   * A stored assistant message carries its own `tool_calls`, so the count is a walk
   * over the messages rather than a second piece of state that could disagree with
   * them. The live turn's calls are added to it separately, because they are not in
   * the transcript yet -- which is the whole reason this number moves while a turn
   * runs.
   */
  const transcriptCalls = (session?.messages ?? []).reduce(
    (n, m) => n + (m.role === 'assistant' ? (m.tool_calls?.length ?? 0) : 0),
    0,
  );
  const runningTools = toolList.filter((t) => t.status === 'running');
  const lastRunning = runningTools[runningTools.length - 1];
  const waiting = running && !lastRunning && !turn?.text;

  return (
    <div data-ui="chat" className={styles.root}>
      {/*
        * The strip above the transcript.
        *
        * It names the region and puts one reading at its right edge, which is the
        * shape the design uses above its own transcript. What it carries is the
        * session's **tool-call count**, and deliberately not a step counter: the named
        * workflow steps are already a row of their own directly below
        * (`StepProgress`), and a second count of the same thing one strip higher
        * would be two answers to one question.
        *
        * It counts the whole session -- the transcript's calls plus the live turn's,
        * which are not in the transcript yet -- because a strip that appeared only
        * while a turn was running would be missing from the screen you look at most.
        * The first version of this counted the live turn alone, and rendered nothing
        * at all for a session that had finished.
        */}
      {session && (
        <div className={styles.head}>
          <Eyebrow latin>Transcript</Eyebrow>
          <span className={styles.count}>
            {progress
              ? `${String(progress.done).padStart(2, '0')} / ${String(progress.total).padStart(2, '0')}`
              : `${transcriptCalls + toolList.length} tool call${
                  transcriptCalls + toolList.length === 1 ? '' : 's'
                }`}
          </span>
        </div>
      )}
      <div ref={scrollRef} onScroll={onScroll} className={styles.scroll}>
        {session ? (
          <>
            <MessageList messages={session.messages} onAction={onSend} />
            {/*
              * An empty state for the case that actually happens: a session with
              * nothing said yet. It carries no action because the action is the
              * field directly below it -- a button that focused the composer would
              * be a second way to do the thing that is already on screen.
              */}
            {!running && session.messages.length === 0 && (
              <div className={styles.emptyTranscript}>
                <EmptyState
                  icon={Bot}
                  title="Nothing said yet"
                  hint="Describe what you want done. The agent reads and writes inside this project, and shows every change it makes."
                />
              </div>
            )}
            {running && (
              <div className={styles.phase}>
                <Spinner size="sm" label="Working" />
                <span className={styles.phaseText}>
                  {lastRunning
                    ? `running ${lastRunning.name}…`
                    : waiting
                      ? 'thinking…'
                      : 'generating…'}
                </span>
                <span className={styles.phaseTimer}>
                  {lastRunning
                    ? // The RUNNING TOOL's own elapsed, not time since the last
                      // visible event.
                      `tool ${formatDuration(Date.now() - (lastRunning.startedAt ?? Date.now()))}`
                    : `idle ${formatDuration(idleSecs * 1000)}`}
                </span>
                {shouldShowStallNotice(idleSecs) && (
                  <Chip status="attention" size="sm">
                    no events for {idleSecs}s
                  </Chip>
                )}
              </div>
            )}
            {infos.map((i) => (
              <Callout key={i.id} tone="warn">
                {i.text}
              </Callout>
            ))}
            {stuck && (
              <Callout tone="warn" title={notice.message}>
                {notice.description}
              </Callout>
            )}
            {steps && <StepProgress steps={steps} />}
            <LiveRun
              tools={toolList}
              now={nowMs}
              onAction={onSend}
              turnStartedAt={turn?.startedAt}
            />
            {!!turn?.thinking && !turn.text && (
              <div className={styles.thinking}>
                <Icon src={Brain} tone="muted" />
                <span>{turn.thinking.slice(-400)}</span>
              </div>
            )}
            {!!turn?.thinking && !!turn.text && (
              <details className={styles.reasoning}>
                <summary className={styles.reasoningHead}>
                  <Icon src={Brain} tone="muted" />
                  reasoning
                </summary>
                <div className={styles.reasoningBody}>{turn.thinking.slice(-1200)}</div>
              </details>
            )}
            {!!turn?.text && <Markdown>{turn.text}</Markdown>}
          </>
        ) : (
          <EmptyState
            icon={Bot}
            title="No session open"
            hint="Open or create a session from the sidebar to begin."
          />
        )}
        {detached && (
          <Button
            size="sm"
            icon={ArrowDown}
            className={styles.jump}
            onClick={jumpToBottom}
            aria-label="Jump to the bottom of the transcript"
          >
            Jump to bottom
          </Button>
        )}
      </div>
      {/*
        * The composer is ONE bordered field: the textarea, and under it a row with
        * what you are about to send *as* -- mode and thinking -- on the left and the
        * action on the right. It used to be a frameless textarea beside a button with
        * a row of readings under both, which is three things stacked where the design
        * has one object.
        *
        * The two settings are chips because they are settings, not readings: they
        * change what happens when you press the button next to them, and putting them
        * in the same box is what says so. `Context` stays in the status bar -- it is a
        * measurement, and the thing it measures is not this field.
        */}
      {session && (
        <div className={styles.composer}>
          <div className={styles.field}>
            <TextArea
              bare
              mono
              aria-label="Ask the agent"
              placeholder="Ask the agent, or type / for a command…"
              title="Enter to send, Shift+Enter for a newline"
              value={input}
              rows={1}
              maxRows={8}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey) {
                  e.preventDefault();
                  send();
                }
              }}
              disabled={running || !session}
            />
            <div className={styles.foot}>
              <span className={styles.chips}>
                {/* The mode is two chips rather than one menu: it has two answers and
                    both fit, and a control whose options are visible is a control you
                    do not have to open to understand. */}
                {MODE_OPTIONS.map((option) => (
                  <button
                    key={option}
                    type="button"
                    className={styles.chip}
                    data-on={session.mode === option || undefined}
                    disabled={running}
                    onClick={() => onMode?.(option)}
                  >
                    {option}
                  </button>
                ))}
                <button
                  ref={thinkAnchorRef}
                  type="button"
                  className={styles.chip}
                  aria-haspopup="menu"
                  aria-expanded={thinkOpen}
                  title="Change the thinking level"
                  disabled={running}
                  onClick={() => setThinkOpen((o) => !o)}
                >
                  thinking · {session.thinking}
                </button>
              </span>
              {running ? (
                <Button tier="danger" size="sm" onClick={onCancel}>
                  Stop
                </Button>
              ) : (
                /*
                 * Text only, no glyph.
                 *
                 * The design's button is 52x27 with a word in it, and a 15px icon plus
                 * an 8px gap made this one about 71 wide at the same height -- 2.5:1
                 * instead of 1.9:1, which reads as a flat bar rather than as a button.
                 * The word was always the label; the glyph was decoration that cost
                 * the shape.
                 *
                 * And **not** disabled on an empty field. The design's send button is
                 * the solid acid, and a *dimmed* acid cannot be both weaker and still
                 * green: in light a fill pale enough to keep a dark ink readable stops
                 * looking green at all. So emptiness is not a state of this button --
                 * `send()` already returns on an empty input. The spent state is kept
                 * where it means something real: `SerialView` disables Start while the
                 * port is not open.
                 */
                <Button tier="primary" size="sm" onClick={send}>
                  Send
                </Button>
              )}
            </div>
            <Menu
              open={thinkOpen}
              anchorRef={thinkAnchorRef}
              onClose={() => setThinkOpen(false)}
              items={THINKING_LEVELS.map((level) => ({
                key: level,
                label: `thinking: ${level}`,
                onSelect: () => onThinking?.(level),
              }))}
            />
          </div>
        </div>
      )}
    </div>
  );
}
