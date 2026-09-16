import { useMemo, useState } from 'react';
import { Bot, FolderOpen, ShieldCheck, Trash2, Zap } from 'lucide-react';
import type { CSSProperties } from 'react';

import pkg from '../../package.json';
import { formatStamp } from '../lib/format';
import type { SessionSummaryDto } from '../types';
import {
  Button,
  Chip,
  EmptyState,
  IconButton,
  PopConfirm,
  TextInput,
  Tooltip,
  useTooltip,
} from '../ui';
import type { ButtonProps, ChipStatus } from '../ui';
import styles from './SessionSidebar.module.css';

/**
 * The session rail: what to start, where to start it, and what is already there.
 *
 * Three things in here are deliberate and would be undone by accident:
 *
 * * **No chip is ever re-painted for a selected row.** The version of this file
 *   that ran on antd assembled every badge from `color.*` in JS and inverted the
 *   pair when the row was selected, because a transparent chip on acid is green
 *   text on green. `Chip` pairs its own fill and ink in CSS, and both are opaque,
 *   so the selection rule belongs to the row alone and the rail's JS has no
 *   colour in it at all.
 * * **A row is one `<button>` and its controls are its siblings.** Nested
 *   interactive content is invalid, and a `<div>` with buttons in it is why every
 *   action used to need an `e.stopPropagation()`.
 * * **Exactly one category chip per row.** `kind` arrives as an untyped `string`,
 *   and the old markup was three independent `&&`s, so a mainline that was also
 *   nested printed two. `kindOf` returns one and cannot return two.
 */

/** One row of the flattened tree, with the two facts the walk worked out. */
interface RailRow {
  session: SessionSummaryDto;
  depth: number;
  /** A mainline with branches under it: the project root the workbench opens. */
  isProjectRoot: boolean;
}

/**
 * Branches under their parent, everything else at the top, depth-first.
 *
 * A session whose parent is missing from the list is hoisted to a root rather
 * than dropped: the kernel keeps `parent_session` pointing at whatever it forked
 * from, and a row that rendered nowhere would be a session that could not be
 * deleted from the GUI. A session that is its own parent is hoisted for the same
 * reason, and because it would otherwise recurse forever as its own child.
 */
function buildRows(sessions: SessionSummaryDto[]): RailRow[] {
  const ids = new Set(sessions.map((s) => s.id));
  const byParent = new Map<string, SessionSummaryDto[]>();
  const roots: SessionSummaryDto[] = [];

  for (const s of sessions) {
    const parent = s.parent_session;
    if (parent && parent !== s.id && ids.has(parent)) {
      const kids = byParent.get(parent) ?? [];
      kids.push(s);
      byParent.set(parent, kids);
    } else {
      roots.push(s);
    }
  }

  // Newest first at the top, oldest first underneath: a mainline's branches read
  // as a timeline of the work, and the newest of them is the one still open.
  roots.sort((a, b) => b.updated_at - a.updated_at);
  for (const kids of byParent.values()) kids.sort((a, b) => a.updated_at - b.updated_at);

  const rows: RailRow[] = [];
  const walk = (s: SessionSummaryDto, depth: number) => {
    const kids = byParent.get(s.id) ?? [];
    rows.push({ session: s, depth, isProjectRoot: s.kind === 'mainline' && kids.length > 0 });
    for (const kid of kids) walk(kid, depth + 1);
  };
  for (const root of roots) walk(root, 0);
  return rows;
}

/** The row's one judgement-free label: what kind of session this is. */
function kindOf(session: SessionSummaryDto, depth: number): { status: ChipStatus; text: string } {
  if (session.kind === 'mainline') return { status: 'ok', text: 'MAINLINE' };
  if (session.kind === 'branch' || depth > 0) return { status: 'neutral', text: '↳ BRANCH' };
  return { status: 'neutral', text: 'NORMAL' };
}

/**
 * A control plus the tooltip that names it.
 *
 * `useTooltip` is per-control -- a tooltip hangs off one element -- so this
 * wrapper is what three of the rail's four actions need, and it is the same
 * shape `TitleBarActions.tsx` documents. The ref and the trigger attributes go
 * on the button itself: a `<span>` around a tooltip's target would be an
 * anonymous flex item between the row's controls and the row.
 */
function TipButton({ tipText, ...rest }: ButtonProps & { tipText: string }) {
  const tip = useTooltip<HTMLButtonElement>();
  return (
    <>
      <Button {...rest} ref={tip.anchorRef} {...tip.triggerProps} />
      <Tooltip tip={tip} text={tipText} />
    </>
  );
}

/** Jump to the workbench scoped to this project. Only a root has one. */
function WorkbenchAction({ cwd, onOpen }: { cwd: string; onOpen: (cwd: string) => void }) {
  const tip = useTooltip<HTMLButtonElement>();
  const label = "Open this project's workbench";
  return (
    <>
      <IconButton
        {...tip.triggerProps}
        ref={tip.anchorRef}
        size="sm"
        label={label}
        icon={FolderOpen}
        onClick={() => {
          tip.close();
          // The row's own click handler is not in this subtree, so opening the
          // workbench does not also select the session underneath.
          onOpen(cwd);
        }}
      />
      <Tooltip tip={tip} text={label} />
    </>
  );
}

export function SessionSidebar({
  sessions,
  currentId,
  workCwd,
  onWorkCwd,
  onSelect,
  onNew,
  onDelete,
  runningIds,
  onOpenWorkbench,
}: {
  sessions: SessionSummaryDto[];
  currentId: string | null;
  workCwd: string;
  onWorkCwd: (cwd: string) => void;
  onSelect: (id: string) => void;
  onNew: (mode: 'agent' | 'plan') => void;
  onDelete: (id: string) => void;
  /** Sessions with a turn currently streaming in the background. */
  runningIds?: Set<string>;
  /** Open the Workbench view scoped to this session's project path. */
  onOpenWorkbench: (cwd: string) => void;
}) {
  const rows = useMemo(() => buildRows(sessions), [sessions]);

  return (
    <div data-ui="session-rail" className={styles.root}>
      <div className={styles.head}>
        {/* Not `primary`: the acid and the cut both mean "the action this
            screen is for", and on a session that is Send. Two of them is how
            neither reads as the one. */}
        <TipButton
          tipText="New agent session, in the working directory below"
          tier="secondary"
          icon={Zap}
          onClick={() => onNew('agent')}
        >
          New
        </TipButton>
        <TipButton
          tipText="New plan-mode session (read-only tools)"
          aria-label="New plan-mode session"
          tier="secondary"
          icon={ShieldCheck}
          onClick={() => onNew('plan')}
        />
      </div>

      <TextInput
        mono
        aria-label="Working directory for new sessions"
        placeholder="C:\"
        spellCheck={false}
        value={workCwd}
        onChange={(e) => onWorkCwd(e.target.value)}
      />

      {rows.length === 0 ? (
        <div className={styles.empty}>
          <EmptyState
            icon={Bot}
            title="No sessions yet"
            hint="New above starts one in the working directory below."
          />
        </div>
      ) : (
        <ul className={styles.list}>
          {rows.map((row) => (
            <SessionRow
              key={row.session.id}
              row={row}
              selected={row.session.id === currentId}
              running={runningIds?.has(row.session.id) ?? false}
              onSelect={() => onSelect(row.session.id)}
              onOpenWorkbench={onOpenWorkbench}
              onDelete={() => onDelete(row.session.id)}
            />
          ))}
        </ul>
      )}

      <div className={styles.foot}>
        <span>
          {sessions.length} {sessions.length === 1 ? 'session' : 'sessions'}
        </span>
        <span className={styles.version}>v{pkg.version}</span>
      </div>
    </div>
  );
}

function SessionRow({
  row,
  selected,
  running,
  onSelect,
  onOpenWorkbench,
  onDelete,
}: {
  row: RailRow;
  selected: boolean;
  running: boolean;
  onSelect: () => void;
  onOpenWorkbench: (cwd: string) => void;
  onDelete: () => void;
}) {
  const { session, depth, isProjectRoot } = row;
  const kind = kindOf(session, depth);
  const updated = new Date(session.updated_at * 1000);

  return (
    <li
      className={styles.item}
      data-selected={selected || undefined}
      style={{ '--depth': String(depth) } as CSSProperties}
    >
      <button
        type="button"
        data-ui="session-row"
        className={styles.select}
        aria-current={selected ? 'true' : undefined}
        onClick={onSelect}
      >
        <span className={styles.top}>
          <Chip status={kind.status} size="sm">
            {kind.text}
          </Chip>
          {/* The tooltip carries the whole first message, not the 30 characters
              the row has room for. */}
          <span className={styles.title} title={session.preview}>
            {session.preview}
          </span>
        </span>
        <span className={styles.meta}>
          {running ? (
            <Chip status="running" size="sm" icon={Zap}>
              running
            </Chip>
          ) : null}
          <span className={styles.model} title={session.model}>
            {session.model}
          </span>
          <span className={styles.stamp} title={updated.toLocaleString()}>
            {formatStamp(session.updated_at)}
          </span>
        </span>
      </button>

      <span className={styles.controls}>
        {isProjectRoot ? <WorkbenchAction cwd={session.cwd} onOpen={onOpenWorkbench} /> : null}
        <DeleteAction preview={session.preview} onDelete={onDelete} />
      </span>
    </li>
  );
}

/**
 * Delete, with the question next to the row it is about.
 *
 * One `anchorRef` serves two panels here: the tooltip's and the confirmation's.
 * `useOutsideDismiss` ignores a press inside either of them, which is what lets
 * the trigger open the panel in a single gesture instead of opening it and
 * dismissing it again on the way out.
 */
function DeleteAction({ preview, onDelete }: { preview: string; onDelete: () => void }) {
  const tip = useTooltip<HTMLButtonElement>();
  const [open, setOpen] = useState(false);
  const label = 'Delete this session';
  return (
    <>
      <IconButton
        {...tip.triggerProps}
        ref={tip.anchorRef}
        size="sm"
        label={label}
        icon={Trash2}
        onClick={() => {
          tip.close();
          setOpen(true);
        }}
      />
      <Tooltip tip={tip} text={label} />
      <PopConfirm
        open={open}
        anchorRef={tip.anchorRef}
        onClose={() => setOpen(false)}
        onConfirm={() => {
          setOpen(false);
          onDelete();
        }}
        tone="danger"
        title="Delete this session?"
        message={preview}
        confirmLabel="Delete"
      />
    </>
  );
}
