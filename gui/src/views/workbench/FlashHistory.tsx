import { Check, X } from 'lucide-react';

import { formatStamp } from '../../lib/format';
import type { FlashHistoryDto } from '../../types';
import { Card, Chip } from '../../ui';
import styles from './FlashHistory.module.css';

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
 *
 * A failed row now also says why. `error` was already in the record and the card
 * dropped it, so the row that exists to answer "did it reach the board?" answered
 * "no" and stopped there.
 */
export function FlashHistory({ history }: { history: FlashHistoryDto[] }) {
  return (
    <Card title="Flash history" extra=".firment/work/flash-history.jsonl">
      {history.length === 0 ? (
        <p className={styles.none}>
          No flashes recorded yet. Every run of the agent's flash tool lands here, successful or
          not.
        </p>
      ) : (
        history.map((f, i) => (
          <div key={`${f.ts}-${i}`} className={styles.row}>
            <Chip
              size="sm"
              status={f.ok ? 'ok' : 'failed'}
              icon={f.ok ? Check : X}
              title={f.ok ? undefined : (f.error ?? undefined)}
            >
              {f.ok ? 'OK' : 'FAIL'}
            </Chip>
            <span className={styles.chip} title={f.chip}>
              {f.chip}
            </span>
            <span className={styles.file} title={f.file}>
              {f.file}
            </span>
            <span className={styles.stamp} title={new Date(f.ts * 1000).toLocaleString()}>
              {formatStamp(f.ts)}
            </span>
            {!f.ok && f.error ? <span className={styles.error}>{f.error}</span> : null}
          </div>
        ))
      )}
    </Card>
  );
}
