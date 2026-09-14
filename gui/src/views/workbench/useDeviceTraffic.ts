import { useEffect, useRef, useState } from 'react';

import { api, onAgentEvent } from '../../lib/api';
import type { AlertEntry, DeviceEntry } from '../../types';
import type { GuardLink } from './guard';
import { readGuardFrame } from './guard';

/**
 * The SBC data plane's raw traffic, as one subscription.
 *
 * This card subscribes itself to the event stream rather than being fed by the
 * App-level router: it has to work with no project open and no session loaded.
 * The view that owns it stays mounted on every tab -- hidden, not unmounted -- so
 * detection does not go blind when the user looks somewhere else.
 */
export interface DeviceTraffic {
  /** One row per node that has said something, newest first. */
  devices: DeviceEntry[];
  /** Alert frames, newest first. The card lists five of them. */
  alerts: AlertEntry[];
  /** The broker link, as the guard last reported it. */
  link: GuardLink;
}

/** Alert frames kept for this window; the ring is not a history. */
const MAX_ALERTS = 30;

/**
 * @param onAlert sees every alert frame before it reaches the card, raw. This is
 * where a project decides whether the frame is an escalation -- see
 * `foldEscalation`, which is where that decision is written.
 */
export function useDeviceTraffic(
  onAlert?: (frame: string, node: string, ts: number) => void,
): DeviceTraffic {
  const [devices, setDevices] = useState<Record<string, DeviceEntry>>({});
  const [alerts, setAlerts] = useState<AlertEntry[]>([]);
  const [guardFrame, setGuardFrame] = useState<string | null>(null);

  // The listener is attached once, so it reaches the current handler through a
  // ref: the closure made on the first render holds the first render's state.
  const alertRef = useRef(onAlert);
  alertRef.current = onAlert;

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    // Pull the backend's last known status first. Frames emitted before this
    // listener attached are gone, and without the pull the card would sit on
    // "unknown" until the next heartbeat.
    void api
      .mqttStatus()
      .then((frame) => {
        if (!cancelled && frame) setGuardFrame(frame);
      })
      .catch(() => {});
    void onAgentEvent((e) => {
      if (cancelled) return;
      if (e.type === 'guard_status') {
        setGuardFrame(e.frame);
        return;
      }
      if (e.type !== 'device_frame') return;
      const ts = Date.now();
      setDevices((prev) => ({
        ...prev,
        [e.node]: {
          node: e.node,
          lastKind: e.kind,
          lastFrame: e.frame.slice(0, 200),
          ts,
          count: (prev[e.node]?.count ?? 0) + 1,
        },
      }));
      if (e.kind !== 'alert') return;
      setAlerts((prev) =>
        [{ node: e.node, frame: e.frame.slice(0, 300), ts }, ...prev].slice(0, MAX_ALERTS),
      );
      alertRef.current?.(e.frame, e.node, ts);
    }).then((u) => {
      if (cancelled) u();
      else unlisten = u;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
    // One subscription for the life of the view; see the comment on `alertRef`.
  }, []);

  return {
    devices: Object.values(devices).sort((a, b) => b.ts - a.ts),
    alerts,
    link: readGuardFrame(guardFrame),
  };
}
