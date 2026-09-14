import { useState } from 'react';
import { Plus, X } from 'lucide-react';

import type { DecisionEntryDto } from '../../types';
import { Button, Card, Chip, IconButton, TextInput } from '../../ui';
import styles from './Decisions.module.css';

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
    // The button is disabled while a write is in flight; Enter is not, and the
    // backend appends, so an unguarded key path queues a second decision.
    if (busy || !title.trim()) return;
    if (await onAdd(title, body)) {
      setTitle('');
      setBody('');
    }
  };

  return (
    <Card title="Decisions (ADR-lite)" extra="matching branches inherit these at creation">
      {decisions.length === 0 && (
        <p className={styles.none}>
          No decisions recorded. Log chip/peripheral/protocol choices here -- the agent's decision
          tool writes the same list.
        </p>
      )}
      {decisions.map((d, i) => (
        <div key={`${d.date}-${i}`} className={styles.row}>
          {/* A date is an identifier of when, not a judgement, so the chip is
              `neutral`; `mono` is what keeps two of them the same width. */}
          <span className={styles.date}>
            <Chip size="sm" mono>
              {d.date || '—'}
            </Chip>
          </span>
          <div className={styles.text}>
            <p className={styles.headline}>{d.title}</p>
            {d.body ? <p className={styles.rationale}>{d.body}</p> : null}
          </div>
          <IconButton
            tier="ghost"
            size="sm"
            icon={X}
            label={`Remove decision: ${d.title}`}
            disabled={busy}
            onClick={() => onRemove(i)}
          />
        </div>
      ))}
      <div className={styles.draft}>
        <TextInput
          size="sm"
          placeholder="decision headline (I2C bus at 400k)"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
        />
        <TextInput
          size="sm"
          placeholder="rationale / constraints (optional)"
          value={body}
          onChange={(e) => setBody(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') submit();
          }}
        />
        <Button size="sm" icon={Plus} disabled={busy || !title.trim()} onClick={submit}>
          record
        </Button>
      </div>
    </Card>
  );
}
