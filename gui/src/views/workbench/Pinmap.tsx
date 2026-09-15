import { useState } from 'react';
import { X } from 'lucide-react';

import type { BoardPinmapDto } from '../../types';
import { Button, Card, Chip, IconButton, TextInput } from '../../ui';
import styles from './Pinmap.module.css';

/**
 * Which pin does what, per board.
 *
 * The agent reads this same table through its `pinmap` tool, which is why every
 * row shows an owner: a claim the agent made is not the same as one you made, and
 * the difference matters when a pin turns out to be wrong.
 *
 * Selection is the caller's, not this component's: the board name is part of
 * every write (`claim`, `release`), so it lives where those writes are issued.
 * The three drafts are the opposite -- nothing outside this card reads them, and
 * a claim that fails must leave the pin and the function typed in place, because
 * reading a pin number off a schematic is the expensive part.
 */
export function Pinmap({
  boards,
  selected,
  busy,
  onSelectBoard,
  onClaimPin,
  onRemovePin,
}: {
  boards: BoardPinmapDto[];
  selected: string | null;
  busy: boolean;
  onSelectBoard: (board: string) => void;
  /** Returns true when the claim was written. */
  onClaimPin: (pin: string, func: string) => Promise<boolean>;
  onRemovePin: (pin: string) => void;
}) {
  const [boardDraft, setBoardDraft] = useState('');
  const [pinDraft, setPinDraft] = useState('');
  const [funcDraft, setFuncDraft] = useState('');
  const active = selected ? boards.find((b) => b.board === selected) : undefined;

  const useBoard = () => {
    const name = boardDraft.trim();
    if (name) onSelectBoard(name);
    setBoardDraft('');
  };

  const claim = async () => {
    if (busy || !pinDraft.trim() || !funcDraft.trim()) return;
    if (await onClaimPin(pinDraft.trim(), funcDraft.trim())) {
      setPinDraft('');
      setFuncDraft('');
    }
  };

  return (
    <Card
      title="Pin assignments"
      extra={<span className={styles.hint}>board-scoped, shared with the agent's pinmap tool</span>}
    >
      {boards.length === 0 && !selected && (
        <p className={styles.none}>
          No boards/pins yet. Pick a board name (use the device's MQTT node name, e.g. s3-node-1)
          and claim pins — the agent sees the same table.
        </p>
      )}

      <div className={styles.boards}>
        {boards.map((b) => (
          <button
            key={b.board}
            type="button"
            aria-pressed={b.board === selected}
            onClick={() => onSelectBoard(b.board)}
            className={styles.board}
          >
            {b.board} ({b.pins.length})
          </button>
        ))}
        <TextInput
          size="sm"
          mono
          placeholder="new board name"
          value={boardDraft}
          onChange={(e) => setBoardDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') useBoard();
          }}
        />
        <Button size="sm" disabled={busy || !boardDraft.trim()} onClick={useBoard}>
          use board
        </Button>
      </div>

      {selected && (
        <>
          {!active || active.pins.length === 0 ? (
            <p className={styles.none}>Board "{selected}" has no claimed pins yet.</p>
          ) : (
            <div className={styles.pins}>
              {active.pins.map((p) => (
                <div key={p.pin} className={styles.pin}>
                  <Chip size="sm" status="running" mono>
                    {p.pin}
                  </Chip>
                  <span className={styles.func}>{p.func}</span>
                  <span className={styles.owner}>{p.owner || '—'}</span>
                  <IconButton
                    tier="ghost"
                    size="sm"
                    icon={X}
                    label={`Release pin: ${p.pin}`}
                    disabled={busy}
                    onClick={() => onRemovePin(p.pin)}
                  />
                </div>
              ))}
            </div>
          )}

          <div className={styles.claim}>
            <TextInput
              size="sm"
              mono
              placeholder="pin (PA5)"
              value={pinDraft}
              onChange={(e) => setPinDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void claim();
              }}
            />
            <TextInput
              size="sm"
              placeholder="function (LED / USART1_TX…)"
              value={funcDraft}
              onChange={(e) => setFuncDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void claim();
              }}
            />
            {/* Enter is not disabled by `busy` the way the button is, and the
                backend writes per call, so the key path needs its own guard. */}
            <Button
              size="sm"
              disabled={busy || !pinDraft.trim() || !funcDraft.trim()}
              onClick={() => void claim()}
            >
              claim on {selected}
            </Button>
          </div>
        </>
      )}
    </Card>
  );
}
