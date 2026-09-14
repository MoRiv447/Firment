import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { api } from '../../../lib/api';
import type { FrontendEvent } from '../../../types';
import { useDeviceTraffic } from '../useDeviceTraffic';

/**
 * The one subscription the workbench keeps open.
 *
 * The hook's whole job is to be right about who is listening, when, and with
 * which closure — and each of those has a failure mode that a component test
 * cannot see:
 *
 * * Frames published before the listener attached are gone. Without the
 *   status pull the badge sits on "unknown" until the next heartbeat, which on
 *   a quiet broker means until the user opens the drawer and distrusts it.
 * * The handler is attached once, so the callback it holds is the one from the
 *   FIRST render. A project's bindings change; if the callback came from render
 *   one, an alert would be judged against the bindings of a project that is no
 *   longer open.
 * * Dropping the subscription matters because this view is hidden, not
 *   unmounted — but the tests that mount it are not, and a leaked listener
 *   writes into an unmounted tree.
 *
 * `onAgentEvent` is replaced so the event bus is a local array; `api` is the
 * real module with `mqttStatus` spied, so the pull is still a call on the
 * object the app uses.
 */

const bus = vi.hoisted(() => ({
  handlers: new Map<number, (e: FrontendEvent) => void>(),
  next: 0,
}));

vi.mock('../../../lib/api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../../../lib/api')>();
  return {
    ...real,
    onAgentEvent: (cb: (e: FrontendEvent) => void) => {
      const id = bus.next;
      bus.next += 1;
      bus.handlers.set(id, cb);
      return Promise.resolve(() => bus.handlers.delete(id));
    },
  };
});

/**
 * Publish on the bus the way the backend does: every live listener hears it, and
 * a map rather than an array is what lets an unsubscribe drop exactly one.
 */
function emit(e: FrontendEvent) {
  act(() => {
    bus.handlers.forEach((h) => h(e));
  });
}

const frame = (over: Partial<Record<string, unknown>> = {}) =>
  JSON.stringify({ node: 'pump-1', rule: 'temp-high', sev: 'error', ...over });

const telemetry = (node = 'pump-1'): FrontendEvent => ({
  type: 'device_frame',
  node,
  kind: 'telemetry',
  frame: '{"c":1}',
});

const alertFrame = (node = 'pump-1', payload = frame()): FrontendEvent => ({
  type: 'device_frame',
  node,
  kind: 'alert',
  frame: payload,
});

const status = (frameText: string): FrontendEvent => ({ type: 'guard_status', frame: frameText });

let clock = 0;

beforeEach(() => {
  clock = 0;
  // Strictly increasing, so "newest first" is a real order and not a tie that
  // jsdom happens to resolve the same way twice.
  vi.spyOn(Date, 'now').mockImplementation(() => (clock += 1));
  vi.spyOn(api, 'mqttStatus').mockResolvedValue('');
});

afterEach(() => {
  bus.handlers.clear();
  vi.restoreAllMocks();
});

describe('useDeviceTraffic', () => {
  it('starts from the last status the backend has, not from silence', async () => {
    vi.mocked(api.mqttStatus).mockResolvedValue('{"connected":true}');
    const { result } = renderHook(() => useDeviceTraffic());
    await waitFor(() => expect(result.current.link.state).toBe('on'));
    // No event needed: the frames before the listener attached are not coming.
    expect(bus.handlers.size).toBe(1);
  });

  it('a failed pull leaves the link unknown, not off', async () => {
    vi.mocked(api.mqttStatus).mockRejectedValue(new Error('no runtime'));
    const { result } = renderHook(() => useDeviceTraffic());
    await waitFor(() => expect(bus.handlers.size).toBe(1));
    expect(result.current.link).toEqual({ state: 'unknown', error: null });
  });

  it('keeps one row per node and counts what it says', () => {
    const { result } = renderHook(() => useDeviceTraffic());
    emit(telemetry());
    emit(telemetry());
    emit(telemetry('probe-b'));
    expect(result.current.devices.map((d) => [d.node, d.count])).toEqual([
      ['probe-b', 1],
      ['pump-1', 2],
    ]);
    expect(result.current.devices[1]).toMatchObject({ lastKind: 'telemetry', lastFrame: '{"c":1}' });
  });

  it('shows the loudest node first', () => {
    const { result } = renderHook(() => useDeviceTraffic());
    emit(telemetry('pump-1'));
    emit(telemetry('probe-b'));
    expect(result.current.devices[0].node).toBe('probe-b');
  });

  it('takes the guard at its word, both ways', () => {
    const { result } = renderHook(() => useDeviceTraffic());
    emit(status('{"connected":false,"error":"refused"}'));
    expect(result.current.link).toEqual({ state: 'off', error: 'refused' });
    emit(status('{"connected":true}'));
    expect(result.current.link.state).toBe('on');
  });

  it('says nothing to the project about a frame that is not an alert', () => {
    const onAlert = vi.fn();
    renderHook(() => useDeviceTraffic(onAlert));
    emit(telemetry());
    expect(onAlert).not.toHaveBeenCalled();
  });

  it('hands the project the frame it received, whole', () => {
    // The card lists a 300-character excerpt; the decision needs the rest —
    // a payload cut here is a payload cut from `rule` and from `sev` too.
    const onAlert = vi.fn();
    const long = frame({ payload: 'x'.repeat(400) });
    const { result } = renderHook(() => useDeviceTraffic(onAlert));
    emit(alertFrame('pump-1', long));
    expect(onAlert).toHaveBeenCalledWith(long, 'pump-1', expect.any(Number));
    expect(result.current.alerts[0].frame).toHaveLength(300);
  });

  it('judges an alert with the callback from this render', () => {
    // The listener is attached once; if it kept the first closure, `late` would
    // never hear about a frame and the project would silently stop escalating.
    const early = vi.fn();
    const late = vi.fn();
    const { rerender } = renderHook(({ cb }) => useDeviceTraffic(cb), {
      initialProps: { cb: early },
    });
    rerender({ cb: late });
    emit(alertFrame());
    expect(early).not.toHaveBeenCalled();
    expect(late).toHaveBeenCalledTimes(1);
  });

  it('subscribes once, however often the view re-renders', () => {
    const { rerender } = renderHook(() => useDeviceTraffic());
    rerender();
    rerender();
    expect(bus.handlers.size).toBe(1);
  });

  it('stops listening when the view goes away', async () => {
    const onAlert = vi.fn();
    const { unmount } = renderHook(() => useDeviceTraffic(onAlert));
    unmount();
    // The unsubscribe is a promise away, so a frame can still reach the
    // listener after the view is gone. The `cancelled` guard is what drops it.
    emit(alertFrame());
    expect(onAlert).not.toHaveBeenCalled();
    await waitFor(() => expect(bus.handlers.size).toBe(0));
  });

  it('rings the alert list instead of growing it', () => {
    const { result } = renderHook(() => useDeviceTraffic());
    for (let i = 0; i < 31; i += 1) emit(alertFrame(`node-${i}`));
    expect(result.current.alerts).toHaveLength(30);
    expect(result.current.alerts[0].node).toBe('node-30');
    // Every node is still a row: the ring is the alert list, not the traffic.
    expect(result.current.devices).toHaveLength(31);
  });
});
