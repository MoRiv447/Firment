import { Alert, Card, Space, Tag, Typography } from 'antd';
import type { ToolCardState } from '../types';

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

export function ToolCard({ tool, standalone }: { tool: ToolCardState; standalone?: boolean }) {
  const danger = dangerousName(tool.name, tool.args);
  const color =
    tool.status === 'ok' ? 'green' : tool.status === 'failed' ? 'red' : danger ? 'orange' : 'blue';
  const icon = tool.status === 'ok' ? '✓' : tool.status === 'failed' ? '✕' : danger ? '⚠' : '·';

  const inner = (
    <Space direction="vertical" size={4} style={{ width: '100%' }}>
      {tool.args !== undefined && (
        <pre style={{ margin: 0, whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>
          {formatArgs(tool.args)}
        </pre>
      )}
      {tool.status !== 'running' && tool.summary && (
        <Text type="secondary" style={{ whiteSpace: 'pre-wrap' }}>
          {tool.summary}
        </Text>
      )}
      {danger && <Alert type="warning" showIcon message="Dangerous command - verify before allowing" />}
    </Space>
  );

  return (
    <Card
      size="small"
      style={{
        ...(standalone ? {} : { margin: '6px 0' }),
        borderRadius: 0,
        border: '2px solid #000000',
        boxShadow: '3px 3px 0 #000000',
        background: '#12151b',
      }}
      styles={{
        body: { paddingTop: 8 },
        header: { minHeight: 38, borderBottom: '2px solid #000000' },
      }}
      title={
        <Space size={8}>
          {tool.status !== 'running' && <Text strong>{icon}</Text>}
          <Tag
            color={color}
            style={{ borderRadius: 0, border: '2px solid #000', color: '#e6e9ef', fontWeight: 700 }}
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