import type { EscalationEntry } from '../../types';

/**
 * Reading the guard's frames, and folding an alert into the pending list.
 *
 * These decisions used to be written inline in the body of an event listener in
 * a 1500-line view, where the five ways an alert can be dropped are five
 * `return`s nobody can see from the outside -- and the whole point of the list is
 * that most alerts ARE dropped: an unbound node is traffic, a `debug` under a
 * `warn` threshold is noise, and a second copy of a pending alert is the same
 * finding twice.
 *
 * So they are functions here. `useDeviceTraffic` only says that an alert frame
 * arrived; the view decides what it means.
 */

/** Guard severity levels, quietest first. */
const SEV_ORDER = ['debug', 'info', 'warn', 'error'] as const;

/** How loud a level is, as a number. An unrecognised level is `info`'s rank. */
export function sevRank(sev: string): number {
  const at = (SEV_ORDER as readonly string[]).indexOf(sev);
  return at === -1 ? 1 : at;
}

/** An alert as it arrived on the wire, before anyone decided what to do with it. */
export interface DeviceAlert extends EscalationEntry {
  /** The guard re-issued a pending alert: same node + rule, better information. */
  revised: boolean;
}

/** A JSON object from a frame, or nothing at all when the frame is not one. */
function fields(frame: string): Record<string, unknown> {
  try {
    const raw: unknown = JSON.parse(frame);
    return raw !== null && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
  } catch {
    /* a plain-text frame is still an alert; it just carries no fields */
    return {};
  }
}

const text = (value: unknown, fallback = ''): string =>
  value === undefined || value === null ? fallback : String(value);

/**
 * Read an alert frame.
 *
 * A frame that is not JSON still becomes an alert with defaults: the node it was
 * published on and the raw payload are all the guard promised, and a device that
 * cannot format its own error is exactly the case someone needs to see.
 */
export function alertFromFrame(frame: string, node: string, ts: number): DeviceAlert {
  const parsed = fields(frame);
  const from = text(parsed.node, node);
  return {
    // One pending alert per node + rule, which is also what makes a revision
    // findable: `revised: true` updates the row with this id.
    id: `${from}-${text(parsed.rule)}`,
    ts,
    node: from,
    sev: text(parsed.sev, 'warn'),
    rule: text(parsed.rule),
    summary: text(parsed.summary),
    payload: text(parsed.payload, frame).slice(0, 300),
    revised: parsed.revised === true,
  };
}

export interface EscalationRules {
  /** Fold alerts at or above this level; the project's `guard_escalate_sev`. */
  threshold: string;
  /** Does this project own the node the alert came from? */
  bound: boolean;
}

export type EscalationFold =
  | { kind: 'none' }
  /** A revision of a row already pending: the row is replaced, its age is kept. */
  | { kind: 'updated'; entries: EscalationEntry[] }
  /** A new row, which is the only fold that may start a diagnosis. */
  | { kind: 'escalated'; entries: EscalationEntry[]; entry: EscalationEntry };

/** Rows kept for one project; the list is a ring, not a history. */
const MAX_PENDING = 20;

/**
 * Fold one alert into the pending list.
 *
 * `none` is the common answer and it is not a failure: an alert from a node this
 * project does not own is traffic, a `debug` under a `warn` threshold is noise,
 * and a revision of a row the user already dismissed stays dismissed.
 *
 * A revision keeps the row's ORIGINAL `ts`. Re-stamping it would move a finding
 * that has been waiting five minutes back to the top of the list, which is how a
 * chatty device makes its own oldest problem look newest.
 */
export function foldEscalation(
  prev: EscalationEntry[],
  alert: DeviceAlert,
  rules: EscalationRules,
): EscalationFold {
  const { revised: _revised, ...entry } = alert;
  const at = prev.findIndex((x) => x.id === alert.id);

  if (alert.revised) {
    if (at === -1) return { kind: 'none' };
    const entries = [...prev];
    entries[at] = { ...entries[at], ...entry, ts: prev[at].ts };
    return { kind: 'updated', entries };
  }
  if (!rules.bound || at !== -1) return { kind: 'none' };
  if (sevRank(alert.sev) < sevRank(rules.threshold)) return { kind: 'none' };
  const entries = [entry, ...prev].slice(0, MAX_PENDING);
  return { kind: 'escalated', entries, entry };
}

/** What the MQTT link is doing, as the card has to say it. */
export interface GuardLink {
  /**
   * `unknown` means "no news yet", and it is a state of its own because the
   * alternative is rendering silence as a down link.
   */
  state: 'on' | 'off' | 'unknown';
  /** Why it is down. Only set when `state` is `off`. */
  error: string | null;
}

const UNKNOWN_LINK: GuardLink = { state: 'unknown', error: null };

/**
 * Read the guard's status frame.
 *
 * Called from two places in the old render, each doing its own `JSON.parse` and
 * each catching its own error, so the badge and the warning banner were two
 * readers of one frame with no shared answer.
 */
export function readGuardFrame(frame: string | null): GuardLink {
  if (!frame) return UNKNOWN_LINK;
  const parsed = fields(frame);
  if (parsed.connected === true) return { state: 'on', error: null };
  if (parsed.connected === false) {
    return { state: 'off', error: text(parsed.error, 'disconnected') };
  }
  return UNKNOWN_LINK;
}
