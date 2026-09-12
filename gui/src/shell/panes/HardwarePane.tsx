import { useEffect, useState } from 'react';
import { Tooltip } from 'antd';
import { UsbOutlined, RocketOutlined } from '@ant-design/icons';
import { font, radius, color } from '../../styles/tokens';
import { SerialView } from '../../views/SerialView';
import { FlashView } from '../../views/FlashView';
import type { MonitorLine } from '../../types';

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
    <div style={{ display: 'flex', flexDirection: 'column', minHeight: 0, height: '100%', gap: 8 }}>
      <div style={{ display: 'flex', gap: 4, flex: '0 0 auto' }}>
        {(
          [
            ['serial', 'Serial', <UsbOutlined key="u" />],
            ['flash', 'Flash', <RocketOutlined key="r" />],
          ] as const
        ).map(([key, label, icon]) => {
          const on = tab === key;
          return (
            <button
              key={key}
              type="button"
              onClick={() => setTab(key)}
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 6,
                padding: '4px 10px',
                border: `1px solid ${on ? color.outline : 'transparent'}`,
                borderRadius: radius.control,
                background: on ? color.surfaceRaised : 'transparent',
                color: on ? color.ink : color.muted,
                cursor: 'pointer',
                fontFamily: font.sans,
                fontSize: 12,
                fontWeight: on ? 600 : 400,
              }}
            >
              {icon}
              {label}
            </button>
          );
        })}
        <span style={{ flex: 1 }} />
        {sawOutput && (
          <Tooltip title="The board has produced output in this session">
            <span
              style={{
                alignSelf: 'center',
                width: 6,
                height: 6,
                borderRadius: radius.chip,
                background: color.successInk,
              }}
            />
          </Tooltip>
        )}
      </div>
      <div style={{ flex: 1, minHeight: 0, overflowY: 'auto' }}>
        {tab === 'serial' ? <SerialView lines={monitorLines} /> : <FlashView />}
      </div>
    </div>
  );
}
