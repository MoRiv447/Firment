import { useState } from 'react';
import { Plus, RotateCw } from 'lucide-react';

import type { SessionSummaryDto } from '../../types';
import { Button, Card, Chip, EmptyState, IconButton, Segmented, Tooltip, useTooltip } from '../../ui';
import styles from './SessionTree.module.css';

/** The four ways to look at one project's sessions. */
type KindFilter = 'all' | 'normal' | 'mainline' | 'branch';

const FILTERS: { value: KindFilter; label: string }[] = [
  { value: 'all', label: 'ALL' },
  { value: 'normal', label: 'NORMAL' },
  { value: 'mainline', label: 'MAINLINE' },
  { value: 'branch', label: 'BRANCH' },
];

/**
 * The project's conversations, and what each one is.
 *
 * The filter lives here rather than upstream because nothing else reads it: it
 * decides which rows exist and whether the "new mainline" offer is shown (a
 * filtered view is already an answer to "there are none", so offering to create
 * one there would be answering a different question).
 *
 * A mainline is an *emphasis*, not a warning, which is why it wears the
 * attention pair rather than a colour that reads as a fault -- and the selection
 * is drawn with the selection token, not the brand: a 1px acid border on a light
 * ground is 1.27:1, so the selected row and its neighbours used to be the same
 * colour.
 */
export function SessionTree({
  sessions,
  mainlineSession,
  currentId,
  busy,
  onReload,
  onNewMainline,
  onSetMainline,
  onOpen,
  onBranch,
}: {
  sessions: SessionSummaryDto[];
  /** The session the project treats as its mainline, if it has one. */
  mainlineSession?: string;
  currentId: string | null;
  busy: boolean;
  onReload: () => void;
  onNewMainline: () => void;
  onSetMainline: (id: string) => void;
  onOpen: (id: string) => void;
  onBranch: (parentId: string) => void;
}) {
  const [filter, setFilter] = useState<KindFilter>('all');
  const reloadTip = useTooltip<HTMLButtonElement>();

  const rows = sessions
    .filter((s) => filter === 'all' || s.kind === filter)
    .map((s) => ({
      ...s,
      isMainline: mainlineSession === s.id || s.kind === 'mainline',
    }));

  return (
    <Card
      title="Session tree"
      extra={
        <div className={styles.head}>
          <Segmented
            ariaLabel="session kind"
            size="sm"
            options={FILTERS}
            value={filter}
            onChange={setFilter}
          />
          <button
            {...reloadTip.triggerProps}
            ref={reloadTip.anchorRef}
            type="button"
            aria-label="Reload sessions from disk"
            disabled={busy}
            onClick={onReload}
            className={styles.reload}
          >
            <RotateCw size={14} />
          </button>
          <Tooltip tip={reloadTip} text="Reload sessions from disk" side="left" />
        </div>
      }
    >
      {rows.length === 0 && (
        <div className={styles.empty}>
          <EmptyState title="No sessions under this path yet" />
          {filter === 'all' && (
            <Button tier="primary" disabled={busy} onClick={onNewMainline}>
              New mainline chat here
            </Button>
          )}
        </div>
      )}

      <div className={styles.rows}>
        {rows.map((s) => (
          <div key={s.id} className={styles.row} data-current={s.id === currentId} data-kind={s.kind}>
            <Chip
              size="sm"
              upper
              status={s.kind === 'mainline' ? 'attention' : s.kind === 'branch' ? 'running' : 'ok'}
            >
              {s.isMainline ? 'mainline' : s.kind}
            </Chip>
            <span className={styles.preview}>{s.preview || s.id.slice(0, 8)}</span>
            {s.parent_session && (
              <span className={styles.parent}>of {s.parent_session.slice(0, 8)}</span>
            )}
            {!s.isMainline && (
              <Button size="sm" disabled={busy} onClick={() => onSetMainline(s.id)}>
                set mainline
              </Button>
            )}
            <Button size="sm" disabled={busy} onClick={() => onOpen(s.id)}>
              open
            </Button>
            <IconButton
              size="sm"
              icon={Plus}
              label={`Branch from ${s.preview || s.id.slice(0, 8)}`}
              disabled={busy}
              onClick={() => onBranch(s.id)}
            />
          </div>
        ))}
      </div>
    </Card>
  );
}
