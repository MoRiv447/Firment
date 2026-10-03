import { useMemo, useRef, useState } from 'react';
import { Bot, FolderOpen, MoreHorizontal, Pencil, ShieldCheck, Trash2, Zap } from 'lucide-react';
import type { CSSProperties } from 'react';

import pkg from '../../package.json';
import { isComposing } from '../lib/ime';
import type { SessionSummaryDto } from '../types';
import {
  confirm,
  Field,
  Button,
  EmptyState,
  Eyebrow,
  Icon,
  Menu,
  StatusDot,
  TextInput,
  Tooltip,
  useTooltip,
} from '../ui';
import type { ButtonProps } from '../ui';
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
  onRename,
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
  /**
   * Rename a session, or clear the name with an empty string.
   *
   * The core decides what a name means -- it prefers an explicit title over the
   * name derived from the first message, so clearing one hands the row back to
   * it. Nothing is interpreted on this side.
   */
  onRename: (id: string, title: string) => void;
}) {
  const rows = useMemo(() => buildRows(sessions), [sessions]);

  return (
    <div data-ui="session-rail" className={styles.root}>
      <div className={styles.head}>
        {/*
          * Solid, and this reverses an earlier call of mine.
          *
          * The argument for the outline was "one solid per context, and the context
          * is for sending -- New is pressed once, so a solid pulls the eye to the
          * least-used corner". Both halves of that are wrong about this rail. The
          * rail's context *is* sessions, so New is not a side door: it is the one
          * thing this region is for. And the two solids do not compete -- they are in
          * different regions and never on screen at the same moment in a way that
          * makes either ambiguous; the composer's Send answers "send this", the rail's
          * bar answers "start something", and a rail whose only coloured element is a
          * border is a rail you have to read to use.
          */}
        <TipButton
          tipText="New agent session, in the working directory below (Ctrl+N)"
          tier="primary"
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

      {/*
        * Labelled, because it was the one control in this rail that read as a
        * mistake: a bare box holding `C:\` and nothing to say what it was for.
        * It is not the open session's path -- that one is in the title bar and in
        * the composer -- it is where the next session will start, which is why it
        * stays a field rather than becoming a read-out.
        */}
      <Field label="new sessions in">
        <TextInput
          mono
          placeholder="C:\"
          spellCheck={false}
          value={workCwd}
          onChange={(e) => onWorkCwd(e.target.value)}
        />
      </Field>

      {rows.length === 0 ? (
        <div className={styles.empty}>
          <EmptyState
            icon={Bot}
            title="No sessions yet"
            hint="New above starts one in the working directory below."
          />
        </div>
      ) : (
        <div className={styles.listGroup}>
          {/*
            * The list gets a label, above the thing it labels.
            *
            * It lives inside this branch rather than above the ternary because the
            * empty pane already says what it is: a heading over nothing is the
            * "configured but empty" furniture docs/design/tokens.md rules out.
            */}
          <Eyebrow latin>Sessions</Eyebrow>
          <ul className={styles.list}>
            {rows.map((row) => (
              <SessionRow
                key={row.session.id}
                row={row}
                selected={row.session.id === currentId}
                running={runningIds?.has(row.session.id) ?? false}
                onSelect={() => onSelect(row.session.id)}
                onOpenWorkbench={onOpenWorkbench}
                onRename={(title) => onRename(row.session.id, title)}
                onDelete={() => onDelete(row.session.id)}
              />
            ))}
          </ul>
        </div>
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
  onRename,
  onDelete,
}: {
  row: RailRow;
  selected: boolean;
  running: boolean;
  onSelect: () => void;
  onOpenWorkbench: (cwd: string) => void;
  onRename: (title: string) => void;
  onDelete: () => void;
}) {
  const { session, depth, isProjectRoot } = row;
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState('');
  const [menuOpen, setMenuOpen] = useState(false);
  const moreRef = useRef<HTMLButtonElement | null>(null);
  const name = session.preview || 'New session';

  const commit = () => {
    setRenaming(false);
    // Nothing to say if it did not change -- and an unchanged name would still bump
    // `updated_at` and re-sort the rail under the pointer.
    if (draft.trim() !== name) onRename(draft.trim());
  };

  return (
    <li
      className={styles.item}
      data-selected={selected || undefined}
      data-renaming={renaming || undefined}
      style={{ '--depth': String(depth) } as CSSProperties}
    >
      {renaming ? (
        /*
         * Renaming happens in the row, not in a dialog.
         *
         * A bare `<input>` rather than the layer's `TextInput`: that control is 36px
         * with a frame of its own, and this is a 28px row where the row *is* the field.
         * A framed box appearing inside a list row would push every row below it down
         * for as long as the rename lasts.
         */
        <input
          className={styles.rename}
          autoFocus
          defaultValue={name}
          aria-label="Session name"
          onChange={(e) => setDraft(e.target.value)}
          onFocus={() => setDraft(name)}
          onKeyDown={(e) => {
            // Both keys belong to the input method while a candidate is open: Enter confirms the
            // candidate and Escape closes the candidate window. Treated as field commands they
            // would commit a half-written name and throw the edit away.
            if (isComposing(e)) return;
            if (e.key === 'Enter') {
              e.preventDefault();
              commit();
            }
            if (e.key === 'Escape') setRenaming(false);
          }}
          onBlur={commit}
        />
      ) : (
        <>
          <button
            type="button"
            data-ui="session-row"
            className={styles.select}
            aria-current={selected ? 'true' : undefined}
            title={name}
            onClick={onSelect}
          >
            {/*
              * The running mark survives the trim, and it is the only thing that does.
              * The row used to carry a kind chip, the model and a timestamp under the
              * name; the name is the row now. A 6px dot is not a second line, and
              * without it the rail has no way at all to say which session is working.
              */}
            {running ? <StatusDot status="running" pulse /> : null}
            <span className={styles.title} data-untitled={session.preview ? undefined : true}>
              {name}
            </span>
          </button>
          <button
            ref={moreRef}
            type="button"
            className={styles.more}
            aria-label={`Options for ${name}`}
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            onClick={() => setMenuOpen((open) => !open)}
          >
            <Icon src={MoreHorizontal} size="sm" tone="muted" />
          </button>
          {/*
            * The destructive action lives in here now instead of on the row.
            *
            * It was a bin icon rendered on every row, which put the one irreversible
            * thing in the rail at the same weight as the row's own name, on a surface
            * you hover to read. A menu is one press further away and it is where every
            * other client puts it.
            */}
          <Menu
            open={menuOpen}
            anchorRef={moreRef}
            onClose={() => setMenuOpen(false)}
            labelledBy={`${session.id}-options`}
            align="end"
            items={[
              {
                key: 'rename',
                label: 'Rename',
                icon: Pencil,
                onSelect: () => {
                  setDraft(name);
                  setRenaming(true);
                },
              },
              ...(isProjectRoot
                ? [
                    {
                      key: 'workbench',
                      label: 'Open workbench',
                      icon: FolderOpen,
                      onSelect: () => onOpenWorkbench(session.cwd),
                    },
                  ]
                : []),
              { separator: true as const, key: 'sep' },
              {
                key: 'delete',
                label: 'Delete',
                icon: Trash2,
                danger: true,
                onSelect: () => {
                  void confirm({
                    title: 'Delete this session?',
                    message: name,
                    confirmLabel: 'Delete',
                    tone: 'danger',
                  }).then((ok) => {
                    if (ok) onDelete();
                  });
                },
              },
            ]}
          />
        </>
      )}
    </li>
  );
}
