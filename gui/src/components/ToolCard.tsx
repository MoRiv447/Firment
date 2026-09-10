import { Alert, Card, Space, Tag, Typography } from 'antd';
import { DownOutlined, RightOutlined } from '@ant-design/icons';
import type { ToolCardState } from '../types';
import { color, font, radius } from '../styles/tokens';

const { Text } = Typography;

function formatArgs(args: unknown): string {
  if (args === undefined || args === null) return '';
  try {
    const s = typeof args === 'string' ? args : JSON.stringify(args, null, 2);
    return s.length > 800 ? `${s.slice(0, 800)}…` : s;
  } catch {
    return String(args);
  }
}

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
        border: `2px solid ${color.outline}`,
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
}: {
  tool: ToolCardState;
  standalone?: boolean;
  /** When set, the card title carries a chevron and toggles on click — used
   * by historical (collapsed-by-default) renderings so the tool name shows
   * exactly once in both states. */
  collapsible?: { open: boolean; onToggle: () => void };
}) {
  const danger = dangerousName(tool.name, tool.args);
  const tagColor =
    tool.status === 'ok' ? 'green' : tool.status === 'failed' ? 'red' : danger ? 'orange' : 'blue';
  const icon = tool.status === 'ok' ? '✓' : tool.status === 'failed' ? '✕' : danger ? '⚠' : '·';

  const inner = (
    <Space direction="vertical" size={4} style={{ width: '100%' }}>
      {tool.args !== undefined && (
        <pre style={{ margin: 0, whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>
          {formatArgs(tool.args)}
        </pre>
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
    </Space>
  );

  return (
    <Card
      size="small"
      style={{
        ...(standalone ? {} : { margin: '6px 0' }),
        borderRadius: radius.brand,
        border: `2px solid ${color.outline}`,
        boxShadow: `3px 3px 0 ${color.outline}`,
        background: color.surface,
      }}
      styles={{
        body: { paddingTop: 8 },
        header: {
          minHeight: 38,
          borderBottom: collapsible ? 'none' : `2px solid ${color.outline}`,
          ...(collapsible ? { cursor: 'pointer' } : {}),
        },
      }}
      onClick={collapsible?.onToggle}
      title={
        <Space size={8}>
          {collapsible &&
            (collapsible.open ? (
              <DownOutlined style={{ fontSize: 9, color: color.muted }} />
            ) : (
              <RightOutlined style={{ fontSize: 9, color: color.muted }} />
            ))}
          {tool.status !== 'running' && <Text strong>{icon}</Text>}
          <Tag
            color={tagColor}
            style={{
              borderRadius: radius.chip,
              border: `2px solid ${color.outline}`,
              color: color.ink,
              fontWeight: 700,
            }}
          >
            {tool.status === 'running' ? `${icon} ${tool.name}` : tool.name}
          </Tag>
          <Text type="secondary" style={{ fontSize: 12 }}>#{tool.seq}</Text>
        </Space>
      }
    >
      {inner}
    </Card>
  );
}