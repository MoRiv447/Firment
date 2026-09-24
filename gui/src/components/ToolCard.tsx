import { useId } from 'react';
import { ChevronDown, ChevronRight, CircleCheck, CircleX, TriangleAlert } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';

import { editedPath } from '../lib/changes';
import { parseDiff } from '../lib/diff';
import { quickActionsFor } from '../lib/quickActions';
import { describeArgs } from '../lib/toolArgs';
import type { ReviewFinding, ToolCardState } from '../types';
import { Button, Callout, Chip, Icon } from '../ui';
import type { ChipStatus } from '../ui';
import styles from './ToolCard.module.css';

/**
 * One tool call, and what came back.
 *
 * The old card was an antd `Card` with a hand-painted header: `Space` for the
 * row, `Tag` for the name, `Text` for everything else, and a colour read out of
 * `styles/tokens.ts` for each of them. Two of those reads are the reason the
 * header is written in CSS now -- `statusChip('attention')` spread a fill and an
 * ink into an inline style, and an inline style cannot follow a theme flip.
 *
 * What the card says about the call is capped by what the transcript knows:
 * `status === 'unknown'` is a reopened session, where the stored record has the
 * call and the returned text but never whether the call worked -- a denial and a
 * timeout are both plain strings in a tool message. So an unknown call gets no
 * mark and no green.
 */

/**
 * Is this the kind of command that should not be one keystroke from a device?
 *
 * The pattern is deliberately over-broad: the card is not enforcing anything, it
 * is deciding whether to say "look at this before you allow it".
 */
function dangerousName(name: string, args: unknown): boolean {
  if (name === 'shell' || name === 'build' || name === 'verify') {
    const txt = typeof args === 'string' ? args : JSON.stringify(args ?? {});
    return /(^|[;&|]|&&|\|\|)\s*(rm|del|format|mkfs|dd|shutdown|reboot|:\(\)|curl|wget|systemctl|chmod|chown|--no-preserve-root|taskkill|reg\s+delete)/i.test(
      txt,
    );
  }
  return false;
}

/** The mark the chip carries. `unknown` has none, because nothing was measured. */
function markFor(status: ToolCardState['status'], danger: boolean): LucideIcon | null {
  if (status === 'running') return null;
  if (status === 'ok') return CircleCheck;
  if (status === 'failed') return CircleX;
  // `unknown` is a reopened session: nothing was measured, so nothing is marked,
  // unless the command itself is one worth flagging.
  return danger ? TriangleAlert : null;
}

function chipStatus(status: ToolCardState['status'], danger: boolean): ChipStatus {
  switch (status) {
    case 'ok':
      return 'ok';
    case 'failed':
      return 'failed';
    case 'unknown':
      // Reopened history: neutral, and the danger case is the one thing worth
      // colouring -- the command is still as dangerous as it was when it ran.
      return danger ? 'attention' : 'neutral';
    default:
      return danger ? 'attention' : 'running';
  }
}

/**
 * The change, line by line, in the states `parseDiff` produced.
 *
 * Colour per line kind rather than per character run: the diff family is a pair
 * of fills and a pair of inks measured against each other, and hunk headers are
 * `--diff-meta-ink` because they are structure, not content.
 *
 * Exported because the Changes pane renders the same diffs, and a second diff
 * renderer is a second opinion on what `+12 −3` means.
 */
export function DiffBody({ detail }: { detail: string }) {
  const diff = parseDiff(detail);
  if (!diff) {
    // Not a diff: a build's output, a file's contents. It still gets the mono
    // block -- the card is not the place to guess at prose.
    return (
      <pre data-ui="tool-output" className={styles.raw}>
        {detail}
      </pre>
    );
  }
  return (
    <div data-ui="tool-diff" className={styles.diff}>
      {diff.lines.map((line, i) => (
        <div key={i} data-kind={line.kind} className={styles.line}>
          {line.text || ' '}
        </div>
      ))}
      {diff.truncated && (
        <div data-kind="hunk" className={styles.line}>
          …
        </div>
      )}
    </div>
  );
}

/** The severity a badge reports. A card has one badge, so it reports the worst finding. */
function worstSeverity(findings: ReviewFinding[]): ReviewFinding['severity'] {
  return findings.some((f) => f.severity === 'high') ? 'high' : 'medium';
}

export function ToolCard({
  tool,
  collapsible,
  onAction,
}: {
  tool: ToolCardState;
  /** When set, the header is a button that opens and closes the body -- used by
   * historical (collapsed-by-default) renderings so the tool name shows exactly
   * once in both states. */
  collapsible?: { open: boolean; onToggle: () => void };
  /** Sends a canned request to the agent. See lib/quickActions.ts. */
  onAction?: (prompt: string) => void;
}) {
  // §16.2: nothing is shown for a run under two seconds — a line that appears and vanishes while
  // you are reading the one above it is worse than silence. The card's own start time is the
  // clock, and the gate lives here rather than in the reducer because the reducer is pure.
  const showProgress =
    tool.progress !== undefined &&
    tool.startedAt !== undefined &&
    Date.now() - tool.startedAt >= 2000;

  const danger = dangerousName(tool.name, tool.args);
  const status = chipStatus(tool.status, danger);
  const mark = markFor(tool.status, danger);
  const open = collapsible ? collapsible.open : true;
  const bodyId = useId();
  const path = editedPath(tool.args);
  // Not shown when it would only repeat the path the header already carries.
  const described = describeArgs(tool.args);
  const argsLine = described && described !== path ? described : '';
  const diff = parseDiff(tool.detail);

  const head = (
    <>
      {collapsible && (
        <Icon src={collapsible.open ? ChevronDown : ChevronRight} size="sm" tone="muted" />
      )}
      <Chip status={status} icon={mark ?? undefined} size="sm">
        {tool.name}
      </Chip>
      {path && <span className={styles.path}>{path}</span>}
      <span className={styles.seq}>#{tool.seq}</span>
      {showProgress && (
        <span data-ui="tool-progress" className={styles.progress}>
          {tool.progress}
        </span>
      )}
      {(tool.findings?.length ?? 0) > 0 && (
        // The badge carries the count and the worst severity; the body carries the findings.
        // A badge alone would make a reader open a diff to learn what was wrong with it, and
        // the list alone would hide that the card has anything to say at all.
        <span
          data-ui="tool-review"
          data-severity={worstSeverity(tool.findings ?? [])}
          className={styles.review}
          title={`${tool.findings?.length ?? 0} review finding(s)`}
        >
          {tool.findings?.length ?? 0}
        </span>
      )}
      {diff && (diff.added > 0 || diff.removed > 0) && (
        // Right-aligned by `margin-inline-start: auto`, so the counts sit at the
        // far edge of the row rather than next to the tool name.
        <span className={styles.counts}>
          <span data-kind="added">+{diff.added}</span>
          <span data-kind="removed">-{diff.removed}</span>
        </span>
      )}
    </>
  );

  return (
    <article data-ui="tool-card" data-open={open ? 'true' : 'false'} className={styles.card}>
      {collapsible ? (
        <button
          type="button"
          data-ui="tool-card-head"
          data-fold="true"
          className={styles.head}
          aria-expanded={open}
          aria-controls={bodyId}
          onClick={collapsible.onToggle}
        >
          {head}
        </button>
      ) : (
        <div data-ui="tool-card-head" className={styles.head}>
          {head}
        </div>
      )}
      {open && (
        <div id={bodyId} className={styles.body}>
          {/*
            A described line, not `<pre>{JSON.stringify(args, null, 2)}</pre>`.
            Every card used to open with a pretty-printed API payload -- a debug
            view of the request in the most-read part of the interface, with the
            tool's own subject buried inside it. `describeArgs` names the subject
            and keeps the rest to a glance; the raw shape is still in the ledger.
          */}
          {argsLine && <p className={styles.args}>{argsLine}</p>}
          {tool.detail ? (
            <DiffBody detail={tool.detail} />
          ) : (
            tool.status !== 'running' &&
            tool.summary && <p className={styles.summary}>{tool.summary}</p>
          )}
          {tool.findings?.map((finding) => (
            // Tone `failed` for high: the kit's vocabulary is info/warn/failed, and a finding
            // that should stop a release is the closest thing to a failure it has. Medium is
            // `warn` — worth knowing, no decision forced.
            <Callout
              key={finding.id}
              tone={finding.severity === 'high' ? 'failed' : 'warn'}
              title={finding.title}
            >
              <p>{finding.description}</p>
              {finding.impact && <p>{finding.impact}</p>}
              {finding.fix && <p>{finding.fix}</p>}
            </Callout>
          ))}
          {danger && <Callout tone="warn">Dangerous command — verify before allowing</Callout>}
          {/*
            What you do *after* an edit. Only once the edit has finished: an offer
            to build a change that is still being written is an offer to build
            something else.
          */}
          {onAction && tool.status !== 'running' && quickActionsFor(tool.name).length > 0 && (
            <div className={styles.actions}>
              {quickActionsFor(tool.name).map((action) => (
                <Button
                  key={action.key}
                  tier={action.tier}
                  size="sm"
                  onClick={() => onAction(action.prompt)}
                >
                  {action.label}
                </Button>
              ))}
            </div>
          )}
        </div>
      )}
    </article>
  );
}
