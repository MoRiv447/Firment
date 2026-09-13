import { useEffect, useState } from 'react';
import { Rocket, Usb } from 'lucide-react';

import { Segmented, StatusDot } from '../../ui';
import { FlashView } from '../../views/FlashView';
import { SerialView } from '../../views/SerialView';
import type { MonitorLine } from '../../types';
import styles from './HardwarePane.module.css';

/**
 * Serial and Flash, together, inside the inspector.
 *
 * These were two of five top-level tabs. They were the wrong axis for a tab: a
 * tab is a place you *go*, and going there meant leaving the conversation --
 * which is exactly when you want to see both, because you are about to paste
 * what the board said back into the chat.
 *
 * They share a pane rather than getting one each because they are the same
 * subject (the board in front of you) and you are rarely doing both at once.
 */
export function HardwarePane({ monitorLines }: { monitorLines: Record<string, MonitorLine[]> }) {
  const [tab, setTab] = useState<'serial' | 'flash'>('serial');

  // Nothing on the board yet is the common case; the flash tab is where you go
  // when you already have an image, so the default stays on the wire.
  const [sawOutput, setSawOutput] = useState(false);
  useEffect(() => {
    if (Object.values(monitorLines).some((l) => l.length > 0)) setSawOutput(true);
  }, [monitorLines]);

  return (
    <div data-ui="hardware-pane" className={styles.root}>
      <div className={styles.head}>
        <Segmented
          size="sm"
          ariaLabel="Hardware view"
          value={tab}
          onChange={setTab}
          options={[
            { value: 'serial', label: 'Serial', icon: Usb },
            { value: 'flash', label: 'Flash', icon: Rocket },
          ]}
        />
        <span className={styles.spacer} />
        {/*
         * A word, not a dot with a tooltip. The old mark was 6px of green whose only
         * explanation appeared on hover, which put the fact in the one place a
         * keyboard, a touch screen and a screenshot could not reach. It latches
         * because a board that has spoken once has been connected, and that is the
         * actual claim.
         */}
        {sawOutput && (
          <span className={styles.live}>
            <StatusDot status="ok" />
            live
          </span>
        )}
      </div>
      <div className={styles.body}>
        {tab === 'serial' ? <SerialView lines={monitorLines} /> : <FlashView />}
      </div>
    </div>
  );
}
