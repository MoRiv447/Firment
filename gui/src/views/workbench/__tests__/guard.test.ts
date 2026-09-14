import { describe, expect, it } from 'vitest';
import { alertFromFrame, foldEscalation, readGuardFrame, sevRank } from '../guard';
import type { EscalationEntry } from '../../../types';

/**
 * The escalation policy, tested where it could not be reached before.
 *
 * This logic lived inside a `useEffect` listener in WorkbenchView, and the five
 * ways an alert gets dropped were five `return` statements with no handle to
 * pull on: to see any of them a test had to fake the Tauri event bus, drive a
 * project open, bind a node and then publish. What is actually being decided
 * here is a function of three arguments, so that is what is tested.
 *
 * `none` is the answer most of the time and most of these cases are about it.
 * A pending list that fills with a stranger node's noise, or with the same
 * finding twice, is a list nobody reads.
 */

const frame = (over: Record<string, unknown> = {}) =>
  JSON.stringify({ node: 'pump-1', rule: 'temp-high', sev: 'error', summary: '61C', ...over });

const alert = (over: Record<string, unknown> = {}) => alertFromFrame(frame(over), 'pump-1', 1000);

const pending = (over: Partial<EscalationEntry> = {}): EscalationEntry => ({
  id: 'pump-1-temp-high',
  ts: 900,
  node: 'pump-1',
  sev: 'error',
  rule: 'temp-high',
  summary: '61C',
  payload: '{}',
  ...over,
});

const OWNED = { threshold: 'warn', bound: true };

describe('alertFromFrame', () => {
  it('reads the fields the guard promises', () => {
    expect(alert()).toMatchObject({
      id: 'pump-1-temp-high',
      node: 'pump-1',
      sev: 'error',
      rule: 'temp-high',
      summary: '61C',
      revised: false,
    });
  });

  it('trusts the frame over the topic it arrived on', () => {
    // A gateway forwards several nodes on one topic; the frame says who saw it.
    expect(alert({ node: 'probe-b' }).node).toBe('probe-b');
  });

  it('keeps a plain-text frame as an alert, with defaults', () => {
    const raw = alertFromFrame('sensor on fire', 'pump-1', 5);
    expect(raw).toMatchObject({
      id: 'pump-1-',
      node: 'pump-1',
      sev: 'warn',
      rule: '',
      summary: '',
      payload: 'sensor on fire',
    });
  });

  it('does not let one field make a row up whole', () => {
    // The frame is a JSON array here, not an object: `parsed.node` would be
    // undefined and the payload would have to survive being indexed.
    const odd = alertFromFrame('[1,2,3]', 'pump-1', 5);
    expect(odd.node).toBe('pump-1');
    expect(odd.payload).toBe('[1,2,3]');
  });

  it('trims a payload to what the row can show', () => {
    const long = alertFromFrame(JSON.stringify({ payload: 'x'.repeat(400) }), 'pump-1', 5);
    expect(long.payload).toHaveLength(300);
  });

  it('notices a revision', () => {
    expect(alert({ revised: true }).revised).toBe(true);
    // Anything that is not exactly `true` is a new finding.
    expect(alert({ revised: 'yes' }).revised).toBe(false);
  });
});

describe('foldEscalation', () => {
  it('adds a loud alert from a node this project owns', () => {
    const fold = foldEscalation([], alert(), OWNED);
    expect(fold.kind).toBe('escalated');
    expect(fold.kind === 'escalated' && fold.entry.id).toBe('pump-1-temp-high');
    expect(fold.kind === 'escalated' && fold.entries).toHaveLength(1);
  });

  it('drops the `revised` flag from the row it stores', () => {
    // `revised` is wire bookkeeping, not part of EscalationEntry; the pending
    // list is persisted to localStorage and read back as entries.
    const fold = foldEscalation([], alert(), OWNED);
    expect(fold.kind === 'escalated' && fold.entries[0]).not.toHaveProperty('revised');
  });

  it('is quiet about a node that is not bound to this project', () => {
    expect(foldEscalation([], alert(), { ...OWNED, bound: false }).kind).toBe('none');
  });

  it('is quiet below the project threshold', () => {
    const noisy = alert({ sev: 'info' });
    expect(foldEscalation([], noisy, OWNED).kind).toBe('none');
    // The threshold is inclusive: `warn` escalates at `warn`.
    expect(foldEscalation([], alert({ sev: 'warn' }), OWNED).kind).toBe('escalated');
  });

  it('ranks an unknown level as info rather than as the loudest', () => {
    // A guard that invents a level must not escalate everything by accident.
    expect(sevRank('catastrophe')).toBe(sevRank('info'));
    expect(foldEscalation([], alert({ sev: 'catastrophe' }), OWNED).kind).toBe('none');
  });

  it('will not list the same finding twice', () => {
    // Nor start a second diagnosis of it: the view keys the auto-run off
    // `escalated`, and a duplicate is `none`.
    expect(foldEscalation([pending()], alert(), OWNED).kind).toBe('none');
  });

  it('updates a pending row on a revision, and keeps its age', () => {
    const fold = foldEscalation([pending()], alert({ revised: true, summary: '63C' }), OWNED);
    expect(fold.kind).toBe('updated');
    const row = fold.kind === 'updated' && fold.entries[0];
    expect(row).toMatchObject({ summary: '63C', ts: 900 });
    // A revision is the same finding, so it does not restart the clock — and
    // it is not a new one, so the auto-run stays out of it.
    expect(fold.kind).not.toBe('escalated');
  });

  it('respects a row the user dismissed', () => {
    // The revised alert says nothing about the dismissal; re-raising it would
    // put back what someone deliberately removed.
    expect(foldEscalation([], alert({ revised: true }), OWNED).kind).toBe('none');
  });

  it('replaces the row instead of appending a second one', () => {
    const fold = foldEscalation([pending()], alert({ revised: true }), OWNED);
    expect(fold.kind === 'updated' && fold.entries).toHaveLength(1);
  });

  it('keeps the newest twenty when a device will not stop', () => {
    const backlog = Array.from({ length: 25 }, (_, i) => pending({ id: `r-${i}`, rule: `r-${i}` }));
    const fold = foldEscalation(backlog, alert({ rule: 'fresh' }), OWNED);
    expect(fold.kind === 'escalated' && fold.entries).toHaveLength(20);
    expect(fold.kind === 'escalated' && fold.entries[0].rule).toBe('fresh');
  });

  it('does not mutate the list it was given', () => {
    const before = [pending()];
    foldEscalation(before, alert({ revised: true }), OWNED);
    foldEscalation(before, alert({ rule: 'other' }), OWNED);
    expect(before).toHaveLength(1);
    expect(before[0].summary).toBe('61C');
  });
});

describe('readGuardFrame', () => {
  it('has no opinion before the first frame', () => {
    // Silence is not an outage: the badge says `?`, not `off`.
    expect(readGuardFrame(null)).toEqual({ state: 'unknown', error: null });
    expect(readGuardFrame('not json')).toEqual({ state: 'unknown', error: null });
    expect(readGuardFrame('{"connected":"yes"}')).toEqual({ state: 'unknown', error: null });
  });

  it('reports a live link as live', () => {
    expect(readGuardFrame('{"connected":true}')).toEqual({ state: 'on', error: null });
  });

  it('carries the reason for a dead one', () => {
    expect(readGuardFrame('{"connected":false,"error":"refused"}')).toEqual({
      state: 'off',
      error: 'refused',
    });
    expect(readGuardFrame('{"connected":false}').error).toBe('disconnected');
  });
});
