import { font, radius, color, statusChip } from '../styles/tokens';
import type { StatusKind } from '../styles/tokens';

/**
 * The bottom strip: what is happening right now.
 *
 * Every value here used to be a coloured dropdown chip in the header, competing
 * with the navigation for attention. They are readings, so they read as one
 * quiet line: a state dot per item, monospace for the values, and colour
 * reserved for the dot.
 *
 * `StatusItem` is the only shape a status bar entry takes. Keeping it to one
 * component is what stops this from becoming another row of coloured pills --
 * the failure mode of the header it replaces, which ended up with five hues in
 * 200px because each chip chose its own antd preset.
 */
export function StatusItem({
  kind,
  label,
  value,
  onClick,
  title,
}: {
  /** Drives the dot's colour. `neutral` is "no judgement". */
  kind: StatusKind;
  /** What the value is, in the interface's own words. Omitted for a bare dot. */
  label?: string;
  value: string;
  onClick?: () => void;
  title?: string;
}) {
  const chip = statusChip(kind);
  return (
    <span
      title={title}
      onClick={onClick}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        padding: '0 4px',
        cursor: onClick ? 'pointer' : 'default',
        fontSize: 11,
        fontFamily: font.sans,
        whiteSpace: 'nowrap',
      }}
    >
      <span
        aria-hidden
        style={{
          width: 6,
          height: 6,
          borderRadius: radius.chip,
          background: chip.color,
          // The dot carries the hue; nothing else in the row does, which is what
          // keeps a bar with six readings from looking like six alerts.
          opacity: kind === 'neutral' ? 0.5 : 1,
        }}
      />
      {label && <span style={{ color: color.muted }}>{label}</span>}
      <span style={{ color: color.ink, fontFamily: font.mono }}>{value}</span>
    </span>
  );
}

export function StatusBar({ children }: { children: React.ReactNode }) {
  return (
    <footer
      style={{
        height: 28,
        flex: '0 0 auto',
        display: 'flex',
        alignItems: 'center',
        gap: 10,
        padding: '0 12px',
        background: color.surface,
        borderTop: `1px solid ${color.line}`,
        overflow: 'hidden',
      }}
    >
      {children}
    </footer>
  );
}

/** The separator between status groups. A 1px rule, not a border. */
export function StatusDivider() {
  return <span aria-hidden style={{ width: 1, height: 12, background: color.line }} />;
}
