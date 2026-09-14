import { useId, useState } from 'react';
import { Diff } from 'lucide-react';

import { DiffBody } from '../../components/ToolCard';
import { changeTotals, shortenPath } from '../../lib/changes';
import type { FileChange } from '../../lib/changes';
import { EmptyState } from '../../ui';
import styles from './ChangesPane.module.css';

/**
 * Every file the agent changed this session, newest touch first.
 *
 * This is the one pane that answers a question the transcript cannot answer by
 * scrolling for it: *what is different on disk now, and by how much*. Each row is
 * a path, the last diff that touched it, and the count of calls that went into it
 * — so a file edited three times is one row rather than three, and the numbers
 * agree with the body below them.
 *
 * The list is derived from the same pairing the transcript uses
 * (`lib/transcript.ts`), which is what makes it work on a session reopened from
 * disk: the stored record has the call and the returned text but never said which
 * call answered which, and without that pairing the pane is empty exactly when
 * you are reviewing finished work.
 *
 * Two honest limits, both inherited from the kernel rather than chosen here. Only
 * `edit_file` and `write_file` print a diff, so a `shell` call that rewrote a
 * file is missing from this list. And a nested subagent's calls live in the
 * nested session, not the parent's transcript, so its edits are here while the
 * turn is on screen and gone after a reopen.
 */

/** `+12 -3`, or the one true thing to say when there is no diff to count. */
function Counts({ added, removed }: { added: number | null; removed: number | null }) {
  if (added === null || removed === null) {
    // Not `+0 -0`: a file written into existence was not measured, and a zero
    // would be read as "it changed nothing".
    return <span className={styles.created}>new file</span>;
  }
  return (
    <span className={styles.counts}>
      <span data-kind="added">+{added}</span>
      <span data-kind="removed">-{removed}</span>
    </span>
  );
}

function ChangeRow({ change }: { change: FileChange }) {
  const [open, setOpen] = useState(false);
  const bodyId = useId();
  // A create that was never edited has no body to open, so its row is a plain
  // line of text: only a row that folds is allowed to invite a click.
  const foldable = change.diff !== null;

  const head = (
    <>
      <span className={styles.path} title={change.path}>
        {shortenPath(change.path)}
      </span>
      {change.edits > 1 && (
        // The counts describe the diff shown, so this is where the rest of the
        // story goes: `×3` says two earlier diffs were superseded by this one.
        <span className={styles.meta} title={`${change.edits} touches, last one counted`}>
          ×{change.edits}
        </span>
      )}
      {change.truncated && (
        <span className={styles.meta} title="the diff is cut short">
          part
        </span>
      )}
      <Counts added={change.added} removed={change.removed} />
    </>
  );

  return (
    <li className={styles.item}>
      {foldable ? (
        <button
          type="button"
          data-ui="change-row"
          data-fold="true"
          className={styles.row}
          aria-expanded={open}
          aria-controls={open ? bodyId : undefined}
          onClick={() => setOpen((o) => !o)}
        >
          {head}
        </button>
      ) : (
        <div data-ui="change-row" className={styles.row}>
          {head}
        </div>
      )}
      {open && change.diff && (
        <div id={bodyId} className={styles.body}>
          {/* The card's own renderer: the transcript and this pane must not hold
              two opinions of what a `+` line looks like. */}
          <DiffBody detail={change.diff} />
        </div>
      )}
    </li>
  );
}

export function ChangesPane({ changes }: { changes: FileChange[] }) {
  if (changes.length === 0) {
    return (
      <EmptyState
        icon={Diff}
        title="No files changed"
        hint="Edits and writes land here as they finish, each with the diff the tool printed. A change made through the shell tool — a generator, `sed -i`, a checkout — is not one of them, and a call still running has no result to show yet."
      />
    );
  }

  const { files, added, removed, uncounted } = changeTotals(changes);
  const measured = added > 0 || removed > 0;

  return (
    <div data-ui="changes-pane" className={styles.root}>
      <div className={styles.head}>
        {measured && <Counts added={added} removed={removed} />}
        <span className={styles.tally}>
          {files} {files === 1 ? 'file' : 'files'}
        </span>
        {uncounted > 0 && <span className={styles.meta}>{uncounted} new</span>}
      </div>
      <ul className={styles.list}>
        {changes.map((change) => (
          <ChangeRow key={change.path} change={change} />
        ))}
      </ul>
    </div>
  );
}
