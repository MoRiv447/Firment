import type { ToolCardState } from '../types';
import { formatDuration } from './format';

/**
 * How long a step took, and what it is likely to take next time.
 *
 * The rule this module exists to keep: **a duration on screen is either measured or
 * absent.** The step row reports how long a finished step took, and how long the
 * running one has been going, because both are arithmetic on a clock. It offers
 * `~4.0s` for the running step **only** when this session has watched the same tool
 * finish at least twice, which makes that number a median of real runs rather than a
 * guess with a unit stuck on it.
 *
 * What it deliberately does not do: forecast the turn, forecast a step that has not
 * started, or carry a table of "typical" durations. A bar that promises twelve
 * seconds and takes forty is worse than no bar at all -- an unverifiable number is a
 * defect here, not a nicety.
 */

/** Completed runs per tool name, oldest first. */
const runs = new Map<string, number[]>();
/** The `seq` values already counted, so re-renders cannot inflate the samples. */
const counted = new Set<number>();

/**
 * How many runs a median is drawn from. Eight keeps it responsive to a project whose
 * builds just got slower without letting one outlier from this morning dominate an
 * afternoon of work.
 */
const RUNS_KEPT = 8;

/**
 * Two runs, not one. A single observation is not a median -- it is that run, and
 * printing it back would be the tool telling the user what they just watched.
 */
const RUNS_FOR_ESTIMATE = 2;

/** Above this, one decimal stops meaning anything and whole seconds read better. */
const TENTHS_CEILING_MS = 9_950;

/**
 * Count every finished run in `tools` that has not been counted yet.
 *
 * Idempotent by `seq`, which is what lets a caller run it on every render instead of
 * diffing the tool list against the previous one -- the double-render a test or a
 * strict-mode mount would otherwise turn into double the samples.
 */
export function recordCompleted(tools: readonly ToolCardState[]): void {
  for (const tool of tools) {
    const { startedAt, endedAt } = tool;
    // A reopened transcript card has neither, and a running one has no end yet.
    if (startedAt === undefined || endedAt === undefined) continue;
    if (counted.has(tool.seq)) continue;
    counted.add(tool.seq);

    const list = runs.get(tool.name) ?? [];
    list.push(Math.max(0, endedAt - startedAt));
    if (list.length > RUNS_KEPT) list.splice(0, list.length - RUNS_KEPT);
    runs.set(tool.name, list);
  }
}

/**
 * The median of this session's completed runs of `name`, or `null` when there is
 * not enough history to call it one.
 *
 * A median rather than a mean: one build that hit a cold page cache is exactly the
 * sample a mean would let speak for the tool.
 */
export function estimateFor(name: string): number | null {
  const list = runs.get(name);
  if (!list || list.length < RUNS_FOR_ESTIMATE) return null;
  const sorted = [...list].sort((a, b) => a - b);
  const mid = sorted.length >> 1;
  return sorted.length % 2 === 1
    ? sorted[mid]
    : Math.round((sorted[mid - 1] + sorted[mid]) / 2);
}

/** Forget every observation. For tests, and for a fresh session. */
export function resetRuns(): void {
  runs.clear();
  counted.clear();
}

export interface StepTiming {
  /** Measured: the whole run for a finished step, so far for a running one. */
  elapsedMs: number;
  /** Median of this session's runs of the same tool; `null` without history. */
  estimateMs: number | null;
  /** True once the outcome is known, so the elapsed stops moving. */
  finished: boolean;
}

/**
 * The timing for one step, or `null` when nothing has been measured.
 *
 * A step whose tool never started has no clock, and neither has one reopened from a
 * transcript -- both answer `null` rather than `0`, so the row prints nothing instead
 * of `0.0s`. A finished step carries no estimate: there is nothing left to predict.
 */
export function timingFor(tool: ToolCardState, now: number): StepTiming | null {
  const started = tool.startedAt;
  if (typeof started !== 'number' || !Number.isFinite(started)) return null;

  const finished = typeof tool.endedAt === 'number';
  const ended = finished ? (tool.endedAt as number) : now;
  return {
    elapsedMs: Math.max(0, ended - started),
    estimateMs: finished ? null : estimateFor(tool.name),
    finished,
  };
}

/**
 * A step's duration, at the precision its reader needs.
 *
 * Not `formatDuration`, and the difference is the reason this function exists: that
 * one answers "how long has this run been going" in whole seconds, and a step is a
 * smaller unit -- a flash that took 400ms would print `0s` under it, which reads as
 * "nothing happened". One decimal under ten seconds, whole seconds from there, and
 * the same minutes-and-hours shape as the run timer after that.
 */
export function formatStepDuration(ms: number): string {
  const total = Math.max(0, ms);
  // Anything under a tenth of a second is one fact, not two, so it is spelled once
  // rather than printed as `0.0s`.
  if (total < 100) return '<0.1s';
  // Rounded to tenths *before* the comparison, or 9 999 ms would print `10.0s` here
  // and `10s` one millisecond later.
  const tenths = Math.round(total / 100) / 10;
  if (total < TENTHS_CEILING_MS && tenths < 10) return `${tenths.toFixed(1)}s`;
  return formatDuration(total);
}
