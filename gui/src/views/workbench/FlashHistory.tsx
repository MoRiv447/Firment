import { Card, Tag, Typography } from 'antd';
import type { FlashHistoryDto } from '../../types';
import { color, font, radius, statusChip } from '../../styles/tokens';

const { Text } = Typography;

/**
 * The burn log: every `flash` the agent ran, success or failure.
 *
 * Extracted from `WorkbenchView.tsx` so the file can come down in size a section
 * at a time. This one was the easiest seam -- it needs `flashHistory` and
 * nothing else -- and each extraction is meant to arrive with its own test
 * rather than after one monolithic test of the parent view.
 *
 * The file it reads is `.firment/work/flash-history.jsonl`, so the list is the
 * durable record: a flash that failed is still a row, which is the point. The
 * history is how you tell "it never flashed" from "it flashed and the board did
 * nothing".
 */
export function FlashHistory({ history }: { history: FlashHistoryDto[] }) {
  return (
    <Card
      type="inner"
      title="Flash history"
      size="small"
      extra={
        <Text type="secondary" style={{ fontSize: 11 }}>
          .firment/work/flash-history.jsonl
        </Text>
      }
    >
      {history.length === 0 ? (
        <Text type="secondary" style={{ fontSize: 12 }}>
          暂无烧录记录。agent 的 flash 工具每次执行（成功或失败）都会记录在这里。
        </Text>
      ) : (
        history.map((f, i) => (
          <div
            key={`${f.ts}-${i}`}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              padding: '3px 6px',
              borderBottom: `1px solid ${color.line}`,
            }}
          >
            <Tag
              // A badge, so `radius.chip` -- not the tile tier. The fill comes
              // from the status pair, which is what makes the tick readable in
              // both schemes (docs/design/tokens.md, "Radius").
              style={{
                ...statusChip(f.ok ? 'ok' : 'failed'),
                borderRadius: radius.chip,
                fontSize: 10,
                fontWeight: 700,
              }}
            >
              {f.ok ? '✓' : '✗'}
            </Tag>
            <Text style={{ fontSize: 11, fontFamily: font.mono }}>{f.chip}</Text>
            <Text
              type="secondary"
              style={{
                fontSize: 11,
                flex: 1,
                overflow: 'hidden',
                whiteSpace: 'nowrap',
                textOverflow: 'ellipsis',
                fontFamily: font.mono,
              }}
            >
              {f.file}
            </Text>
            <Text type="secondary" style={{ fontSize: 10 }}>
              {new Date(f.ts * 1000).toLocaleString()}
            </Text>
          </div>
        ))
      )}
    </Card>
  );
}
