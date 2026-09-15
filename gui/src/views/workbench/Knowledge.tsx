import { useState } from 'react';

import type { KbEntryDto } from '../../types';
import { Button, Card, Select, TextArea, TextInput } from '../../ui';
import styles from './Knowledge.module.css';

/**
 * The knowledge files the agent reads: AGENTS.md, the vendor index, cheatsheets.
 *
 * **This card owns one draft and the caller owns the rest, and that is the
 * deliberate difference from its neighbours.** Selection, draft text, dirty flag
 * and the save baseline change together in three of the four writes -- selecting
 * a file loads its text *and* freezes a baseline, deleting one selects and loads
 * its successor, creating one does all three -- so splitting them would turn one
 * atomic update in the caller into a prop-driven side effect down here. What does
 * move in is the new-cheatsheet name, which nothing outside this card has ever
 * read.
 *
 * The dirty marker on the save button is a dot rather than a colour: the button
 * is already the primary tier, and a second signal on it would have to compete
 * with the one that means "this is the action".
 */
export function Knowledge({
  files,
  selected,
  draft,
  dirty,
  busy,
  onSelect,
  onDraft,
  onSave,
  onDelete,
  onCreate,
}: {
  files: KbEntryDto[];
  selected: string | null;
  draft: string;
  dirty: boolean;
  busy: boolean;
  onSelect: (key: string) => void;
  onDraft: (text: string) => void;
  onSave: () => void;
  onDelete: () => void;
  /** Returns true when the file was created. */
  onCreate: (name: string) => Promise<boolean>;
}) {
  const [name, setName] = useState('');

  const create = async () => {
    if (busy || !name.trim()) return;
    if (await onCreate(name.trim())) setName('');
  };

  return (
    <Card
      title="Project knowledge"
      extra={
        <div className={styles.create}>
          <TextInput
            size="sm"
            mono
            placeholder="new-cheatsheet.toml"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void create();
            }}
          />
          <Button
            size="sm"
            aria-label="New cheatsheet"
            disabled={busy || !name.trim()}
            onClick={() => void create()}
          >
            +
          </Button>
        </div>
      }
    >
      {files.length === 0 ? (
        <p className={styles.none}>
          No knowledge files yet. AGENTS.md is injected into every session's system prompt;
          docs/vendor-index.toml is the hardware knowledge index; cheatsheets live under
          .firment/cheatsheets/.
        </p>
      ) : (
        <>
          {/* A grid cell, because the trigger is a button and a button in a block
              context is only as wide as its label. */}
          <div className={styles.pick}>
            <Select
              ariaLabel="knowledge file"
              mono
              value={selected ?? undefined}
              onChange={onSelect}
              options={files.map((f) => ({
                value: f.key,
                label: `${f.key}${f.exists ? '' : ' (new)'}`,
              }))}
            />
          </div>
          <TextArea
            value={draft}
            onChange={(e) => onDraft(e.target.value)}
            rows={10}
            mono
            // The antd version of this field had no accessible name at all -- it
            // leaned on the placeholder, which disappears the moment there is a
            // draft. Two unnamed text boxes on one card is also how a screen
            // reader user loses track of which one they are in.
            aria-label="knowledge file contents"
            placeholder={
              selected === 'AGENTS.md'
                ? 'Project memory for every session: coding rules, hardware notes, gotchas…'
                : undefined
            }
          />
          <div className={styles.actions}>
            <Button size="sm" tier="primary" disabled={busy || !dirty} onClick={onSave}>
              save{dirty ? ' •' : ''}
            </Button>
            {selected?.startsWith('cheatsheet:') && (
              <Button size="sm" tier="danger" disabled={busy} onClick={onDelete}>
                delete cheatsheet
              </Button>
            )}
          </div>
        </>
      )}
    </Card>
  );
}
