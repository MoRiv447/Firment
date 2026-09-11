import { useState } from 'react';
import { Button, Card, Input, Space, Tag, Tooltip, Typography } from 'antd';
import type { DecisionEntryDto } from '../../types';
import { color, radius } from '../../styles/tokens';

const { Text } = Typography;

/**
 * The decision log -- ADR-lite.
 *
 * Unlike the report cards, this one owns something: the two draft fields. They
 * belong here rather than with the caller because only this component knows
 * when a draft has been accepted -- the caller's add either records it or
 * reports an error, and a draft that clears itself on failure loses what the
 * user just typed.
 *
 * So `onAdd` returns whether the decision was recorded, and the drafts are
 * cleared only on `true`. That is what the inline version did too (it cleared
 * them inside the success branch), but the knowledge now lives next to the
 * fields instead of in a 1500-line parent.
 */
export function Decisions({
  decisions,
  busy,
  onAdd,
  onRemove,
}: {
  decisions: DecisionEntryDto[];
  busy: boolean;
  /** Returns true when the decision was recorded. */
  onAdd: (title: string, body: string) => Promise<boolean>;
  /** 0-based, as rendered. The caller translates to the backend's 1-based list. */
  onRemove: (index: number) => void;
}) {
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');

  const submit = async () => {
    if (!title.trim()) return;
    if (await onAdd(title, body)) {
      setTitle('');
      setBody('');
    }
  };

  return (
    <Card
      type="inner"
      title="Decisions (ADR-lite)"
      size="small"
      extra={
        <Tooltip title="Branches whose title matches a decision automatically inherit it at creation">
          <Text type="secondary" style={{ fontSize: 11 }}>
            inherited by matching branches
          </Text>
        </Tooltip>
      }
    >
      {decisions.length === 0 && (
        <Text type="secondary" style={{ fontSize: 12 }}>
          No decisions recorded. Log chip/peripheral/protocol choices here — the agent's
          decision tool writes the same list.
        </Text>
      )}
      {decisions.map((d, i) => (
        <div
          key={`${d.date}-${i}`}
          style={{
            display: 'flex',
            alignItems: 'flex-start',
            gap: 8,
            padding: '4px 6px',
            borderBottom: `1px solid ${color.line}`,
          }}
        >
          <Tag style={{ borderRadius: radius.chip, fontSize: 10, minWidth: 76, textAlign: 'center' }}>
            {d.date || '—'}
          </Tag>
          <div style={{ flex: 1, minWidth: 0 }}>
            <Text style={{ fontSize: 12, fontWeight: 600 }}>{d.title}</Text>
            {d.body && (
              <div>
                <Text type="secondary" style={{ fontSize: 11 }}>
                  {d.body}
                </Text>
              </div>
            )}
          </div>
          <Button
            size="small"
            type="text"
            danger
            disabled={busy}
            aria-label={`Remove decision: ${d.title}`}
            onClick={() => onRemove(i)}
          >
            ✕
          </Button>
        </div>
      ))}
      <Space.Compact style={{ width: '100%', marginTop: 8 }}>
        <Input
          size="small"
          placeholder="decision headline (I2C bus at 400k)"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          style={{ maxWidth: 260 }}
        />
        <Input
          size="small"
          placeholder="rationale / constraints (optional)"
          value={body}
          onChange={(e) => setBody(e.target.value)}
          onPressEnter={submit}
        />
        <Button size="small" type="dashed" disabled={busy || !title.trim()} onClick={submit}>
          record
        </Button>
      </Space.Compact>
    </Card>
  );
}
