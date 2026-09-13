import { useState } from 'react';
import type { ReactNode } from 'react';
import { Tooltip } from 'antd';
import { font, radius, color } from '../styles/tokens';

/**
 * The right-hand column: what the session has produced, pinned next to it.
 *
 * It exists because the alternative -- what the app did until now -- was to give
 * each of these its own top-level tab. A tab is a place you go; these are things
 * you glance at *while* reading the conversation, and going to a tab means
 * leaving the conversation behind.
 *
 * Two sections are visible at once and the rest sit behind the tab strip. That
 * cap is deliberate: the reference this is modelled on stacks five sections
 * (plan / agents / output / sources / memory) and they all end up looking
 * equally important, which means none of them does. If everything is a panel,
 * nothing is.
 *
 * The column is collapsible because 360px is a real cost on a laptop, and the
 * honest default for someone reading a long diff is "off".
 */

export interface InspectorTab {
  key: string;
  label: string;
  /** A count worth showing next to the label. Omitted when there is nothing. */
  badge?: number;
  content: ReactNode;
}

export function Inspector({
  tabs,
  open,
  onToggle,
}: {
  tabs: InspectorTab[];
  open: boolean;
  onToggle: () => void;
}) {
  const [active, setActive] = useState(tabs[0]?.key ?? '');
  const current = tabs.find((t) => t.key === active) ?? tabs[0];

  if (!open) {
    // Collapsed to a strip of initials rather than to nothing: the strip is how
    // you get back, and a bare chevron says nothing about what is behind it.
    return (
      <aside
        style={{
          width: 38,
          flex: '0 0 auto',
          borderLeft: `1px solid ${color.line}`,
          background: color.surface,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          paddingTop: 6,
          gap: 2,
        }}
      >
        {tabs.map((t) => (
          <Tooltip key={t.key} title={t.label} placement="left">
            <button
              type="button"
              onClick={() => {
                setActive(t.key);
                onToggle();
              }}
              aria-label={t.label}
              style={{
                width: 28,
                height: 28,
                border: 'none',
                background: 'transparent',
                color: color.muted,
                cursor: 'pointer',
                borderRadius: radius.control,
                fontSize: 12,
                fontFamily: font.sans,
              }}
            >
              {t.label.slice(0, 1)}
              {t.badge ? <span style={{ color: color.ink }}>{t.badge}</span> : null}
            </button>
          </Tooltip>
        ))}
      </aside>
    );
  }

  return (
    <aside
      style={{
        // The pane that gives way. 360px is the right width on a normal display
        // and the wrong one on a laptop at its minimum window size, and the
        // thing that loses text when nothing shrinks here is the transcript.
        width: 'clamp(240px, 26vw, 360px)',
        flex: '0 0 auto',
        borderLeft: `1px solid ${color.line}`,
        background: color.surface,
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 2,
          padding: '0 6px',
          height: 36,
          borderBottom: `1px solid ${color.line}`,
          flex: '0 0 auto',
        }}
      >
        {tabs.map((t) => {
          const on = t.key === current?.key;
          return (
            <button
              key={t.key}
              type="button"
              onClick={() => setActive(t.key)}
              style={{
                border: 'none',
                background: 'transparent',
                cursor: 'pointer',
                padding: '4px 8px',
                borderRadius: radius.chip,
                fontFamily: font.sans,
                fontSize: 12,
                fontWeight: on ? 600 : 400,
                color: on ? color.ink : color.muted,
                // The one accent in the panel: a 2px brand rule under the live
                // tab. The same device as the progress row, so "current" reads
                // the same way in both places.
                boxShadow: on ? `inset 0 -2px 0 ${color.stepRule}` : undefined,
              }}
            >
              {t.label}
              {t.badge ? (
                <span style={{ marginLeft: 5, color: color.muted, fontFamily: font.mono }}>
                  {t.badge}
                </span>
              ) : null}
            </button>
          );
        })}
        <span style={{ flex: 1 }} />
        <Tooltip title="Collapse">
          <button
            type="button"
            onClick={onToggle}
            aria-label="Collapse the inspector"
            style={{
              border: 'none',
              background: 'transparent',
              color: color.muted,
              cursor: 'pointer',
              fontSize: 14,
              padding: '2px 6px',
            }}
          >
            ›
          </button>
        </Tooltip>
      </div>
      <div style={{ flex: 1, minHeight: 0, overflowY: 'auto', padding: 12 }}>{current?.content}</div>
    </aside>
  );
}
