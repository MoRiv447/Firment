import { useState } from 'react';

import { api } from '../lib/api';
import type { AskRequest, PermissionRequest, ToolCardState } from '../types';
import { Button, Callout, Chip, Modal, TextArea } from '../ui';
import styles from './Dialogs.module.css';
import { ToolCard } from './ToolCard';

/**
 * The two windows the kernel can interrupt a turn with.
 *
 * Both used to be an antd `Modal` with a `Space` of buttons in the footer, and
 * three of those decisions are load-bearing enough to be stated rather than
 * rediscovered:
 *
 * * **The permission prompt cannot be closed.** `dismissable={false}` takes away
 *   the ×, the click-past and Escape, because none of the three is an answer --
 *   the agent is blocked on `respond_permission`, and a window that vanished on
 *   Escape would leave a turn waiting for a reply nobody can send any more.
 * * **Deny comes first.** The focus trap puts the caret on the first control it
 *   finds, so the answer an accidental Enter produces is the one that runs
 *   nothing. `Confirm` follows the same rule for the same reason.
 * * **Options are body, not footer.** `ask_user` allows nine of them, and a
 *   footer that right-aligns a single row cannot hold nine; they belong under the
 *   question, where they wrap.
 *
 * The one thing not carried over is the acid badge naming which chat is asking.
 * It was painted from `color.brandAcid` in JS, and the pair that beats every
 * other element in this window is now `--selection`, which means "selected"
 * rather than "this one is talking". The chip is `neutral` and mono instead, and
 * the full id is its `title`.
 */

/** Which chat's agent is asking -- matters once several chats run in parallel. */
function Asking({ sid, hint }: { sid?: string; hint?: string }) {
  if (!sid && !hint) return null;
  return (
    <p className={styles.asking}>
      {sid ? (
        <Chip mono size="sm" status="neutral" title={sid}>
          chat {sid.slice(0, 8)}
        </Chip>
      ) : null}
      {hint ? <span className={styles.hint}>{hint}</span> : null}
    </p>
  );
}

export function PermissionDialog({
  req,
  onClose,
}: {
  req: PermissionRequest;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const tool: ToolCardState = { seq: 0, name: req.tool, args: req.args, status: 'running' };

  const respond = async (allowed: boolean) => {
    // The rest of the app is behind a scrim while this is open, so a double-click
    // is about the only gesture left, and `loading` spins without swallowing it.
    if (busy) return;
    setBusy(true);
    try {
      await api.respondPermission(req.id, allowed);
      onClose();
    } catch (err) {
      // Keep the dialog open and actionable: an IPC failure must not leave
      // an unhandled rejection plus a wedged modal.
      console.error('respond_permission failed:', err);
    } finally {
      // Not only the failure path: a second request can already be in the queue,
      // and then this instance is reused for it with `busy` still set.
      setBusy(false);
    }
  };

  return (
    <Modal
      open
      size="md"
      title="Permission requested"
      // Unreachable while the dialog is not dismissable, and deliberately a no-op
      // rather than `onClose`: if that ever changes, closing must not quietly
      // become a way to drop a request the agent is still blocked on.
      onClose={() => {}}
      dismissable={false}
      footer={
        <>
          <Button tier="danger" loading={busy} onClick={() => respond(false)}>
            Deny
          </Button>
          <Button tier="primary" loading={busy} onClick={() => respond(true)}>
            Allow
          </Button>
        </>
      }
    >
      <div className={styles.stack}>
        <Asking sid={req.session_id} hint="This chat wants to run a tool before continuing." />
        {/* The card is the question: it names the tool, shows the arguments, and
            raises its own warning when the command looks like a device wipe. */}
        <ToolCard tool={tool} />
        {req.reason ? <Callout tone="info">{req.reason}</Callout> : null}
      </div>
    </Modal>
  );
}

export function AskDialog({ req, onClose }: { req: AskRequest; onClose: () => void }) {
  const [busy, setBusy] = useState(false);
  const [custom, setCustom] = useState('');
  const hasOptions = req.options.length > 0;

  const respond = async (answer: string | null) => {
    if (busy) return;
    // Blank free text is not an answer. `null` is, and it is what Dismiss sends.
    if (answer !== null && !answer.trim()) return;
    setBusy(true);
    try {
      await api.respondAsk(req.id, answer);
      onClose();
    } catch (err) {
      console.error('respond_ask failed:', err);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      open
      size="md"
      title="Question from agent"
      // Closing this one is an answer of a kind -- the same thing Dismiss sends --
      // so unlike the permission prompt it keeps its ×, its Escape and its click-past.
      onClose={() => respond(null)}
      footer={
        <>
          <Button tier="ghost" loading={busy} onClick={() => respond(null)}>
            Dismiss
          </Button>
          {hasOptions ? null : (
            <Button
              tier="primary"
              loading={busy}
              disabled={!custom.trim()}
              onClick={() => respond(custom)}
            >
              Reply
            </Button>
          )}
        </>
      }
    >
      <div className={styles.stack}>
        <Asking sid={req.session_id} />
        <p className={styles.question}>{req.question}</p>
        {hasOptions ? (
          <div className={styles.options}>
            {req.options.map((option) => (
              <Button
                key={option}
                size="sm"
                title={option}
                loading={busy}
                onClick={() => respond(option)}
              >
                {option}
              </Button>
            ))}
          </div>
        ) : (
          <TextArea
            data-autofocus="true"
            rows={2}
            placeholder="Type your answer and press Reply (or press Enter)"
            value={custom}
            onChange={(e) => setCustom(e.target.value)}
            onKeyDown={(e) => {
              if (e.key !== 'Enter' || e.shiftKey) return;
              // Prevent the newline as well as sending the answer: a box that
              // keeps the character the user pressed Enter for is showing an
              // answer that is not the one that was sent.
              e.preventDefault();
              respond(custom);
            }}
          />
        )}
      </div>
    </Modal>
  );
}
