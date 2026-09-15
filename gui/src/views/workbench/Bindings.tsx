import { useState } from 'react';
import { Plus, X } from 'lucide-react';

import type { DeviceBindingDto, DeviceEntry } from '../../types';
import { Button, Card, Chip, IconButton, Select, TextInput } from '../../ui';
import styles from './Bindings.module.css';

/**
 * The nodes this project owns.
 *
 * A binding is what makes `device_cmd` reachable at all: the agent can only
 * command a node named here, so this card is the permission surface, not a device
 * browser. That is also why the row shows the allow list — when a command is
 * refused, the answer is on this line.
 *
 * The node half of the draft is a picker whenever the broker is hearing from
 * something unbound, and a text field otherwise. Picking from what is actually
 * publishing is the difference between binding `s3-node-1` and binding
 * `s3-node-l`, which looks the same in the list and resolves to nothing.
 *
 * Like `Decisions`, this owns its drafts: only here is it known whether a bind
 * was accepted, and a field that clears itself on a failed write loses the node
 * name someone had to read off the label.
 */
export function Bindings({
  bindings,
  devices,
  busy,
  onBind,
  onUnbind,
}: {
  bindings: DeviceBindingDto[];
  /** Live traffic: a bound node that is not talking is a bound node to notice. */
  devices: DeviceEntry[];
  busy: boolean;
  /** Returns true when the binding was written. */
  onBind: (node: string, role: string) => Promise<boolean>;
  onUnbind: (node: string) => void;
}) {
  const [node, setNode] = useState('');
  const [role, setRole] = useState('');
  const bound = new Set(bindings.map((b) => b.node));
  const online = devices.filter((d) => !bound.has(d.node));

  const submit = async () => {
    // Enter is not disabled by `busy` the way the button is, and the backend
    // writes per call, so an unguarded key path queues a second bind.
    if (busy || !node.trim()) return;
    if (await onBind(node.trim(), role.trim())) {
      setNode('');
      setRole('');
    }
  };

  return (
    <Card
      title="Devices"
      extra="nodes this project owns — the agent's device_cmd can only reach these"
    >
      {bindings.length === 0 && (
        <p className={styles.none}>
          No devices bound. Bind a node (use its MQTT node name) so the agent can send it
          commands via device_cmd.
        </p>
      )}
      {bindings.map((d) => {
        const live = devices.find((x) => x.node === d.node);
        return (
          <div key={d.node} className={styles.row}>
            {/* The dot is the fact and the node is its name: a chip that says
                only `pump-1` cannot answer "is it there right now?". */}
            <Chip size="sm" status={live ? 'ok' : 'neutral'}>
              {live ? `● ${d.node}` : `○ ${d.node}`}
            </Chip>
            <span className={styles.role}>
              {d.role || '—'}
              {d.note ? <span className={styles.note}> · {d.note}</span> : null}
              {d.allow.length > 0 ? (
                <span className={styles.allow}> [allow: {d.allow.join(', ')}]</span>
              ) : null}
            </span>
            {live ? (
              <span className={styles.seen}>
                ×{live.count} · {new Date(live.ts).toLocaleTimeString()}
              </span>
            ) : null}
            <IconButton
              tier="ghost"
              size="sm"
              icon={X}
              label={`Unbind node: ${d.node}`}
              disabled={busy}
              onClick={() => onUnbind(d.node)}
            />
          </div>
        );
      })}
      <div className={styles.draft}>
        {online.length > 0 ? (
          <Select
            ariaLabel="node to bind"
            placeholder="pick an online node"
            mono
            value={node || undefined}
            onChange={setNode}
            options={online.map((d) => ({ value: d.node, label: `${d.node} (${d.count} frames)` }))}
          />
        ) : (
          <TextInput
            mono
            size="sm"
            placeholder="node name (s3-node-1)"
            value={node}
            onChange={(e) => setNode(e.target.value)}
          />
        )}
        <TextInput
          size="sm"
          placeholder="role (main mcu / sensor node…)"
          value={role}
          onChange={(e) => setRole(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void submit();
          }}
        />
        <Button size="sm" icon={Plus} disabled={busy || !node.trim()} onClick={() => void submit()}>
          bind
        </Button>
      </div>
    </Card>
  );
}
