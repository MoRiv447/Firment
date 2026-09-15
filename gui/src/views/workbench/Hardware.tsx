import { useState } from 'react';
import { RotateCw } from 'lucide-react';

import type { HardwareInfoDto } from '../../types';
import { Button, Card, Chip, TextInput, Tooltip, useTooltip } from '../../ui';
import styles from './Hardware.module.css';

/**
 * What the toolchain can see right now: serial ports, probe-rs probes, and the
 * chip the `flash` tool targets when a call names none.
 *
 * The chip is the only editable thing here and it is the odd one out: a *global*
 * setting on a project screen, which is why the tooltip says so out loud. The
 * write itself belongs to the caller -- it owns the loaded payload and the error
 * slot -- so `onSaveChip` answers with whether it landed, and the editor only
 * closes then. A failed global write that closed the field would look saved.
 *
 * Enumeration is not automatic: listing probes and ports touches hardware, and a
 * screen that did it on mount would spin up a USB scan every time the user
 * changed tabs.
 */
export function Hardware({
  hardware,
  busy,
  onRefresh,
  onSaveChip,
}: {
  hardware: HardwareInfoDto | null;
  busy: boolean;
  onRefresh: () => Promise<void>;
  /** Returns true when the backend accepted the chip. */
  onSaveChip: (chip: string) => Promise<boolean>;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  const chipTip = useTooltip<HTMLButtonElement>();

  const save = async () => {
    if (busy) return;
    if (await onSaveChip(draft.trim())) setEditing(false);
  };

  return (
    <Card
      title="Hardware"
      extra={
        <Button size="sm" icon={RotateCw} disabled={busy} onClick={() => void onRefresh()}>
          refresh
        </Button>
      }
    >
      {!hardware ? (
        <p className={styles.none}>
          Not loaded yet — hit refresh to enumerate serial ports and probe-rs probes.
        </p>
      ) : (
        <>
          <div className={styles.row}>
            {/* A chip you can click is a control, and it wears the chip shape
                because that is what it is showing. */}
            <button
              {...chipTip.triggerProps}
              ref={chipTip.anchorRef}
              type="button"
              disabled={editing}
              onClick={() => {
                setDraft(String(hardware.default_chip));
                setEditing(true);
              }}
              className={styles.chipButton}
            >
              chip: {hardware.default_chip || '(unset)'} ✎
            </button>
            <Tooltip
              tip={chipTip}
              text="Used by the flash tool when no chip parameter is passed. Saved to global config."
              side="top"
            />
            <Chip size="sm" status={hardware.probe_rs_available ? 'ok' : 'neutral'}>
              probe-rs {hardware.probe_rs_available ? 'available' : 'not installed'}
            </Chip>
          </div>

          {editing && (
            <div className={styles.edit}>
              <TextInput
                size="sm"
                mono
                placeholder="default chip (e.g. stm32g431rb — empty to unset)"
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') void save();
                }}
              />
              <Button size="sm" tier="primary" disabled={busy} onClick={() => void save()}>
                save
              </Button>
              <Button size="sm" onClick={() => setEditing(false)}>
                cancel
              </Button>
            </div>
          )}

          {hardware.serial_ports.length === 0 ? (
            <p className={styles.none}>No serial ports found.</p>
          ) : (
            <div className={styles.ports}>
              {hardware.serial_ports.map((port) => (
                <Chip key={port} size="sm" status="running" mono>
                  {port}
                </Chip>
              ))}
            </div>
          )}

          {hardware.probes.length > 0 && (
            <>
              <p className={styles.label}>probe-rs probes:</p>
              {hardware.probes.map((probe) => (
                <p key={probe} className={styles.probe}>
                  {probe}
                </p>
              ))}
            </>
          )}
        </>
      )}
    </Card>
  );
}
