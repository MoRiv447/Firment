import { X } from 'lucide-react';

import { formatStamp } from '../../lib/format';
import type { EscalationEntry } from '../../types';
import { Button, Callout, Card, Chip, IconButton, Switch } from '../../ui';
import styles from './Escalations.module.css';

/**
 * Findings the guard decided are worth a diagnosis.
 *
 * The list is short by construction — `foldEscalation` drops most of what the
 * broker hears — so what this card is responsible for is the two decisions on
 * top of it: whether a finding can be acted on at all (there is no diagnosis
 * without a mainline session), and whether the app acts on its own (the auto
 * switch).
 *
 * The switch says what it does on the line below it rather than in a tooltip.
 * It hands a device's alert to the agent, unprompted, in a project you were not
 * looking at; a control that starts work needs to be legible without being
 * hovered.
 */
export function Escalations({
  entries,
  threshold,
  mainline,
  busy,
  autoRun,
  onAutoRun,
  onDiagnose,
  onDismiss,
}: {
  entries: EscalationEntry[];
  /** The project's `guard_escalate_sev`, echoed so the list is interpretable. */
  threshold: string;
  /** Empty when the project has no mainline: nothing can be diagnosed then. */
  mainline: string;
  busy: boolean;
  autoRun: boolean;
  onAutoRun: (on: boolean) => void;
  onDiagnose: (entry: EscalationEntry) => void;
  onDismiss: (id: string) => void;
}) {
  return (
    <Card
      title="Escalations (guard)"
      extra={
        <span className={styles.head}>
          <span className={styles.sev}>sev ≥ {threshold}</span>
          <Switch checked={autoRun} onChange={onAutoRun} label="auto" />
        </span>
      }
    >
      {autoRun && (
        <p className={styles.auto}>
          A new escalation is handed to the mainline session by itself, and leaves this list
          when it does.
        </p>
      )}
      {entries.length === 0 ? (
        <p className={styles.none}>
          No pending escalations. Alerts from bound nodes at or above the severity threshold
          land here for one-click diagnosis.
        </p>
      ) : (
        <>
          {mainline ? null : (
            <Callout tone="warn">
              No mainline session registered — set one first to enable diagnosis.
            </Callout>
          )}
          {entries.map((e) => (
            <div key={e.id} className={styles.row}>
              <div className={styles.line}>
                {/* `error` is the only level that gets the failure colour: an
                    escalation is already a decision to interrupt, so the chip
                    ranks it rather than alarming on all of them. */}
                <Chip size="sm" status={e.sev === 'error' ? 'failed' : 'attention'}>
                  {e.sev}
                </Chip>
                <Chip size="sm" status="neutral">
                  {e.node}
                </Chip>
                <span className={styles.summary} title={e.summary || e.payload}>
                  {e.summary || e.payload}
                </span>
                <Button
                  size="sm"
                  tier="primary"
                  disabled={busy || !mainline}
                  onClick={() => onDiagnose(e)}
                >
                  diagnose
                </Button>
                <IconButton
                  tier="ghost"
                  size="sm"
                  icon={X}
                  label={`Dismiss escalation: ${e.node} ${e.rule || e.summary}`}
                  disabled={busy}
                  onClick={() => onDismiss(e.id)}
                />
              </div>
              <p className={styles.meta}>
                {/* The list survives a restart, so a row can be days old; a bare
                    clock time would read as "this morning". `formatStamp` takes
                    seconds. */}
                {formatStamp(e.ts / 1000)} · {e.rule || 'no rule'} ·{' '}
                <span className={styles.payload}>{e.payload.slice(0, 120)}</span>
              </p>
            </div>
          ))}
        </>
      )}
    </Card>
  );
}
