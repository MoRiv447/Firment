import type { ToolCardState } from '../types';

/**
 * Where a turn's wall clock actually went.
 *
 * One number per turn — "3m 20s" — answers nothing a developer asks. The question is
 * which of three things ate the time: a tool running, a person being asked, or the
 * model thinking. All three are already in the cards (`startedAt`, `endedAt`, and the
 * gate's `waitedMs`), so this is arithmetic rather than a new measurement.
 *
 * The arithmetic has one rule that makes it worth a module: **every millisecond of the
 * turn is counted once.** A turn runs its calls concurrently, so two tools' windows
 * overlap, and summing durations would exceed the wall clock; a wait can overlap
 * another tool's work, and then the reader needs to be told which one they were
 * waiting on. So the timeline is a sweep with a priority — a person being asked beats
 * a tool running, a tool running beats nothing — and the three results add up to the
 * wall time by construction.
 */

export interface TurnTime {
  /** First known moment to last, in ms. */
  wallMs: number;
  /** Milliseconds where a person was the reason nothing moved. */
  waitedMs: number;
  /** Milliseconds where at least one tool was running. */
  workMs: number;
  /** The rest: model streaming, provider latency, and any gap between two calls. */
  otherMs: number;
  /** True while something is still in flight, so the numbers are still moving. */
  running: boolean;
}

type Kind = 'wait' | 'work';

interface Span {
  start: number;
  end: number;
  kind: Kind;
}

/**
 * One call's window, split into its gate and its work.
 *
 * The split is not a guess about when the person answered: the gate is the first
 * thing a call does (`ToolRegistry::run_measured` asks before it runs the tool), so
 * the wait occupies the front of the window and the tool body the rest.
 *
 * A call still running has no `waitedMs` yet — it arrives with the end — so its
 * window reads as work until then. The bar corrects when the card closes; nothing
 * else would be honest, because nobody has measured the wait at that moment.
 */
function spansOf(tool: ToolCardState, now: number): Span[] {
  const start = tool.startedAt;
  if (typeof start !== 'number') return [];
  const end = typeof tool.endedAt === 'number' ? tool.endedAt : now;
  if (end <= start) return [];
  const wait = Math.min(Math.max(0, tool.waitedMs ?? 0), end - start);
  const spans: Span[] = [];
  if (wait > 0) spans.push({ start, end: start + wait, kind: 'wait' });
  spans.push({ start: start + wait, end, kind: 'work' });
  return spans;
}

/**
 * The turn's wall clock, attributed once to waiting, work, or everything else.
 *
 * `null` when no card carries a clock — a run reopened from a stored transcript,
 * which records what ran and never how long it took. Printing a bar built from
 * unknowns would be the same mistake as printing `0.0s` under such a card.
 *
 * `turnStartedAt` is the turn's own start, and it matters: without it the stretch
 * before the first tool — usually the longest model answer of the turn — is outside
 * the window and silently disappears from the total.
 */
export function turnTimeline(
  tools: readonly ToolCardState[],
  now: number,
  turnStartedAt?: number,
): TurnTime | null {
  const spans = tools.flatMap((t) => spansOf(t, now));
  const firstCard = Math.min(
    ...tools.map((t) => t.startedAt).filter((s): s is number => typeof s === 'number'),
  );
  if (!Number.isFinite(firstCard)) return null;

  const begin = typeof turnStartedAt === 'number' ? Math.min(turnStartedAt, firstCard) : firstCard;
  const running = tools.some((t) => t.status === 'running');
  const lastSeen = Math.max(
    ...spans.map((s) => s.end),
    // A turn with a start and no finished call yet still has a right edge.
    ...(Number.isFinite(begin) ? [begin] : []),
  );
  const end = running ? Math.max(now, lastSeen) : lastSeen;
  const wallMs = Math.max(0, end - begin);
  if (wallMs === 0) return null;

  // Sweep the endpoints; between two adjacent ones the active set cannot change, so
  // one probe per interval is exact rather than sampled.
  const points = [
    ...new Set([begin, end, ...spans.flatMap((s) => [s.start, s.end])]),
  ]
    .filter((p) => p >= begin && p <= end)
    .sort((a, b) => a - b);

  let waitedMs = 0;
  let workMs = 0;
  for (let i = 0; i + 1 < points.length; i += 1) {
    const from = points[i];
    const to = points[i + 1];
    if (to <= from) continue;
    // Any instant inside the interval answers the same question.
    const at = (from + to) / 2;
    const active = spans.filter((s) => s.start <= at && at < s.end);
    if (active.some((s) => s.kind === 'wait')) waitedMs += to - from;
    else if (active.length > 0) workMs += to - from;
  }

  return {
    wallMs,
    waitedMs,
    workMs,
    otherMs: Math.max(0, wallMs - waitedMs - workMs),
    running,
  };
}

/**
 * The sentence a reader hears when the bar has no pixels to spare.
 *
 * The three parts add up to the wall time, and saying so in the label is the point:
 * `3m 20s · tools 1m 02s · waiting on you 2m 18s · model 0.0s` reads as an
 * accounting, not as three unrelated guesses.
 */
export function describeTurnTime(t: TurnTime, format: (ms: number) => string): string {
  const parts: string[] = [];
  if (t.workMs > 0) parts.push(`tools ${format(t.workMs)}`);
  if (t.waitedMs > 0) parts.push(`waiting on you ${format(t.waitedMs)}`);
  if (t.otherMs > 0) parts.push(`model ${format(t.otherMs)}`);
  const wall = `${t.running ? 'running' : 'total'} ${format(t.wallMs)}`;
  return parts.length > 0 ? `${wall} · ${parts.join(' · ')}` : wall;
}
