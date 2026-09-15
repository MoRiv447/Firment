import { useState } from 'react';

import { Button, Modal, TextInput } from '../../ui';
import styles from './BranchDialog.module.css';

/**
 * Naming the new conversation a branch starts.
 *
 * The note is not decoration: a branch inherits the working directory, the
 * provider and the model, and *nothing else* -- no context, no history. That is
 * the entire reason to branch, so the dialog says it instead of leaving it to be
 * discovered after the fact.
 *
 * The name is this dialog's own draft, and it survives a refused create for the
 * usual reason: a name you had to think of should not vanish because the backend
 * was busy.
 */
export function BranchDialog({
  parentId,
  busy,
  onCancel,
  onCreate,
}: {
  /** The session being branched from; `null` means the dialog is closed. */
  parentId: string | null;
  busy: boolean;
  onCancel: () => void;
  /** Returns true when the branch was created. */
  onCreate: (title: string) => Promise<boolean>;
}) {
  const [title, setTitle] = useState('');

  const create = async () => {
    if (busy || !title.trim()) return;
    if (await onCreate(title.trim())) setTitle('');
  };

  const cancel = () => {
    // Reopening with the last attempt still in the field would look like the
    // branch already exists.
    setTitle('');
    onCancel();
  };

  return (
    <Modal
      open={parentId !== null}
      title="New branch conversation"
      onClose={cancel}
      footer={
        <>
          <Button size="sm" onClick={cancel}>
            cancel
          </Button>
          <Button
            size="sm"
            tier="primary"
            disabled={busy || !title.trim()}
            onClick={() => void create()}
          >
            create branch
          </Button>
        </>
      }
    >
      <TextInput
        mono
        aria-label="branch title"
        placeholder="branch title (e.g. sensor drift hunt)"
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        onKeyDown={(e) => {
          // Enter is not disabled by `busy` the way the button is.
          if (e.key === 'Enter') void create();
        }}
      />
      <p className={styles.note}>
        Fresh context linked to {parentId?.slice(0, 8)} — inherits cwd/provider/model only.
      </p>
    </Modal>
  );
}
