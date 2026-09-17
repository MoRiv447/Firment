import { useEffect, useRef, useState } from 'react';
import { ArrowDown, Bot, Brain, Square, Send } from 'lucide-react';

import { LiveRun } from '../components/LiveRun';
import { Markdown } from '../components/Markdown';
import { MessageList } from '../components/MessageList';
import { StepProgress } from '../components/StepProgress';
import { formatDuration } from '../lib/format';
import { shouldShowStallNotice, stallNotice } from '../lib/stallHint';
import { workflowSteps } from '../lib/steps';
import type { RunningTurn, SessionDto } from '../types';
import { Button, Callout, Chip, EmptyState, Icon, Spinner, TextArea } from '../ui';
import styles from './ChatView.module.css';

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
}: {
  session: SessionDto | null;
  running: boolean;
  turn: RunningTurn | null;
  infos: { id: number; text: string }[];
  onSend: (input: string) => void;
  onCancel: () => void;
}) {
  const [input, setInput] = useState('');
  const scrollRef = useRef<HTMLDivElement>(null);
  // Seconds since the last visible event, ticking every second while running —
  // the user can watch this climb to tell a slow model from a wedged turn.
  const [idleSecs, setIdleSecs] = useState(0);

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
  // Where the embedded workflow got to, computed from those same tools rather
  // than tracked separately -- see `lib/steps.ts`. Null for a chat that never
  // builds anything, so the row does not appear as empty furniture.
  const steps = workflowSteps(toolList);
  const runningTools = toolList.filter((t) => t.status === 'running');
  const lastRunning = runningTools[runningTools.length - 1];
  const waiting = running && !lastRunning && !turn?.text;

  return (
    <div data-ui="chat" className={styles.root}>
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
            <LiveRun tools={toolList} onAction={onSend} />
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
      <div className={styles.composer}>
        <div className={styles.inputRow}>
          <TextArea
            aria-label="Ask the agent"
            placeholder="Ask the agent… (Enter to send, Shift+Enter for newline)"
            value={input}
            rows={2}
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
          {running ? (
            <Button tier="danger" icon={Square} onClick={onCancel}>
              Stop
            </Button>
          ) : (
            <Button
              tier="primary"
              icon={Send}
              onClick={send}
              disabled={!session || !input.trim()}
            >
              Send
            </Button>
          )}
        </div>
        {session && (
          /* Under the field rather than over it: what you are about to send is
             the field, and the settings for it are the fine print. Both
             references put mode and model at the foot, next to the action. */
          <div className={styles.meta}>
            <span className={styles.cwd} title={session.cwd}>
              {session.cwd}
            </span>
            <span className={styles.setting}>
              <span className={styles.provider}>{session.provider}</span>
              <span className={styles.model}>{session.model}</span>
              <Chip status={session.mode === 'plan' ? 'attention' : 'ok'} size="sm">
                {session.mode}
              </Chip>
            </span>
          </div>
        )}
      </div>
    </div>
  );
}
