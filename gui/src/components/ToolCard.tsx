import { Alert, Card, Space, Tag, Typography } from 'antd';
import { DownOutlined, RightOutlined } from '@ant-design/icons';
import type { ToolCardState } from '../types';
import { color, font, radius, space, statusChip } from '../styles/tokens';
import type { StatusKind } from '../styles/tokens';
import { quickActionsFor } from '../lib/quickActions';
import { ActionButton } from './ActionButton';
import { describeArgs } from '../lib/toolArgs';

const { Text } = Typography;

function dangerousName(name: string, args: unknown): boolean {
  if (name === 'shell' || name === 'build' || name === 'verify') {
    const txt = typeof args === 'string' ? args : JSON.stringify(args ?? {});
    return /(^|[;&|]|&&|\|\|)\s*(rm|del|format|mkfs|dd|shutdown|reboot|:\(\)|curl|wget|systemctl|chmod|chown|--no-preserve-root|taskkill|reg\s+delete)/i.test(
      txt,
    );
  }
  return false;
}

/** How much of a diff body the card renders before it stops (`detail` itself
 * is already capped at 8000 chars upstream). */
const DIFF_MAX_CHARS = 4000;

/**
 * `+N -M` for the header, counted exactly the way the TUI counts it
 * (`crates/firment-tui/src/view.rs`): over the diff body only. The first line
 * is the "Edited <path>" header, which `DiffBody` drops, so counting it would
 * be counting a line nobody sees.
 */
function diffCounts(detail: string): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const line of detail.split('\n').slice(1)) {
    if (line.startsWith('+')) added += 1;
    else if (line.startsWith('-')) removed += 1;
  }
  return { added, removed };
}

/**
 * The file a tool touched, read from its arguments.
 *
 * `args` is `unknown` (it arrives as JSON from the backend), so every step is
 * checked rather than assumed. This exists because the card lost the path: the
 * diff body drops the "Edited <path>" header on the assumption that the summary
 * already shows it, and the summary is only rendered when there is *no* detail.
 * The two conditions cannot both hold, so the path was on screen in neither
 * place once a diff was attached.
 */
function editedPath(args: unknown): string | undefined {
  if (!args || typeof args !== 'object') return undefined;
  const record = args as Record<string, unknown>;
  for (const key of ['path', 'file_path', 'file']) {
    const value = record[key];
    if (typeof value === 'string' && value) return value;
  }
  return undefined;
}

/**
 * The change a tool made, line by line.
 *
 * The header line is dropped: it is the same "Edited <path> …" text the card
 * summary already shows, so printing it here would duplicate it. Colors are
 * the SUCCESS family on purpose — the acid brand green would read as "brand"
 * rather than "added", and the whole point is to tell added from removed.
 */
function DiffBody({ detail }: { detail: string }) {
  const body = detail.length > DIFF_MAX_CHARS ? `${detail.slice(0, DIFF_MAX_CHARS)}…` : detail;
  return (
    <div
      style={{
        border: `1px solid ${color.outline}`,
        background: color.bg,
        fontSize: 12,
        fontFamily: font.mono,
        maxHeight: 260,
        overflow: 'auto',
      }}
    >
      {body
        .split('\n')
        .slice(1)
        .map((line, i) => {
          const added = line.startsWith('+');
          const removed = line.startsWith('-');
          return (
            <div
              key={i}
              style={{
                padding: '0 6px',
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-all',
                color: added
                  ? color.diffAddedInk
                  : removed
                    ? color.diffRemovedInk
                    : color.diffMetaInk,
                background: added ? color.diffAddedBg : removed ? color.diffRemovedBg : undefined,
                fontWeight: added || removed ? 600 : 400,
              }}
            >
              {line || ' '}
            </div>
          );
        })}
    </div>
  );
}

export function ToolCard({
  tool,
  standalone,
  collapsible,
  onAction,
}: {
  tool: ToolCardState;
  standalone?: boolean;
  /** When set, the card title carries a chevron and toggles on click — used
   * by historical (collapsed-by-default) renderings so the tool name shows
   * exactly once in both states. */
  collapsible?: { open: boolean; onToggle: () => void };
  /** Sends a canned request to the agent. See lib/quickActions.ts. */
  onAction?: (prompt: string) => void;
}) {
  const danger = dangerousName(tool.name, tool.args);
  // An antd preset name ('green') is a colour written down outside the token
  // layer -- it is neither a hex literal nor a radius, so no-literal-tokens
  // cannot see it. statusChip() reads both halves from one measured pair.
  const tagStatus: StatusKind =
    tool.status === 'ok'
      ? 'ok'
      : tool.status === 'failed'
        ? 'failed'
        : tool.status === 'unknown'
          ? danger
            ? 'attention'
            : // Reopened history: no glyph, no green. The transcript does not
              // record whether this call worked, so the card says nothing about
              // it either way.
              'neutral'
          : danger
            ? 'attention'
            : 'running';
  const icon =
    tool.status === 'ok'
      ? '✓'
      : tool.status === 'failed'
        ? '✕'
        : tool.status === 'unknown'
          ? danger
            ? '⚠'
            : ''
          : danger
            ? '⚠'
            : '·';
  const path = editedPath(tool.args);
  // Not shown when it would only repeat the path the header already carries.
  const described = describeArgs(tool.args);
  const argsLine = described && described !== path ? described : '';
  const counts = tool.detail ? diffCounts(tool.detail) : null;
  const hasCounts = counts !== null && (counts.added > 0 || counts.removed > 0);

  const inner = (
    <Space direction="vertical" size={4} style={{ width: '100%' }}>
      {/*
        A described line, not `<pre>{JSON.stringify(args, null, 2)}</pre>`.
        Every card in the transcript opened with a pretty-printed API payload,
        which is a debug view of the request rather than a record of what
        happened -- `{"path":"src/foc/current.c"}` above a one-line file read.
        `describeArgs` names the subject and keeps the rest to a glance.

        The full arguments are not lost: expanding a card is what the raw shape
        was for, and the ledger has them verbatim.
      */}
      {argsLine && (
        <Text type="secondary" style={{ fontFamily: font.mono, fontSize: 12 }}>
          {argsLine}
        </Text>
      )}
      {tool.detail ? (
        <DiffBody detail={tool.detail} />
      ) : (
        tool.status !== 'running' &&
        tool.summary && (
          <Text type="secondary" style={{ whiteSpace: 'pre-wrap' }}>
            {tool.summary}
          </Text>
        )
      )}
      {danger && <Alert type="warning" showIcon message="Dangerous command - verify before allowing" />}
      {/*
        What you do *after* an edit, in the design system's own three tiers. Only
        once the edit has finished: an offer to build a change that is still
        being written is an offer to build something else.
      */}
      {onAction && tool.status !== 'running' && quickActionsFor(tool.name).length > 0 && (
        <div style={{ display: 'flex', alignItems: 'center', gap: space.controlGap, marginTop: 4 }}>
          {quickActionsFor(tool.name).map((action) => (
            <ActionButton
              key={action.key}
              tier={action.tier}
              onClick={() => onAction(action.prompt)}
            >
              {action.label}
            </ActionButton>
          ))}
        </div>
      )}
    </Space>
  );

  return (
    <Card
      size="small"
      style={{
        ...(standalone ? {} : { margin: '6px 0' }),
        borderRadius: radius.tile,
        border: `1px solid ${color.outline}`,
        boxShadow: color.shadowMd,
        background: color.surface,
      }}
      styles={{
        body: { paddingTop: 8 },
        header: {
          minHeight: 38,
          borderBottom: collapsible ? 'none' : `1px solid ${color.line}`,
          ...(collapsible ? { cursor: 'pointer' } : {}),
        },
      }}
      onClick={collapsible?.onToggle}
      title={
        // Flex rather than `Space`: the counts are right-aligned, and a `Space`
        // would pack them next to the tool name instead of at the far edge.
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, width: '100%' }}>
          <Space size={8}>
            {collapsible &&
              (collapsible.open ? (
                <DownOutlined style={{ fontSize: 9, color: color.muted }} />
              ) : (
                <RightOutlined style={{ fontSize: 9, color: color.muted }} />
              ))}
            {tool.status !== 'running' && <Text strong>{icon}</Text>}
            <Tag
              style={{
                ...statusChip(tagStatus),
                borderRadius: radius.chip,
                fontWeight: 700,
              }}
            >
              {tool.status === 'running' ? `${icon} ${tool.name}` : tool.name}
            </Tag>
            {path && (
              <Text style={{ fontFamily: font.mono, fontSize: 12 }}>{path}</Text>
            )}
            <Text type="secondary" style={{ fontSize: 12 }}>#{tool.seq}</Text>
          </Space>
          {hasCounts && (
            <span
              style={{
                marginLeft: 'auto',
                fontFamily: font.mono,
                fontSize: 12,
                fontWeight: 700,
                whiteSpace: 'nowrap',
              }}
            >
              {/* The diff family, not the brand green: an added line and a
                  passed check are not the same message. */}
              <span style={{ color: color.diffAddedInk }}>+{counts.added}</span>{' '}
              <span style={{ color: color.diffRemovedInk }}>-{counts.removed}</span>
            </span>
          )}
        </div>
      }
    >
      {inner}
    </Card>
  );
}