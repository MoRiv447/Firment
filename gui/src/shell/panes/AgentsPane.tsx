import { useId, useState } from 'react';
import { Bot } from 'lucide-react';

import { EmptyState, StatusDot } from '../../ui';
import type { ChipStatus } from '../../ui';
import { ToolCard } from '../../components/ToolCard';
import type { SubagentState } from '../../lib/turnReducer';
import styles from './AgentsPane.module.css';

/**
 * The subagents this turn spawned.
 *
 * One row each, the shape the reference uses: a state dot, the thing's name, and
 * a right-aligned status. The name is the **prompt**, because that is the only
 * truthful label -- an id names nothing a person recognises and the tool name is
 * the same word for every delegation.
 *
 * Their steps are here rather than in the transcript on purpose. A subagent is
 * the agent going away to read things; interleaving its twenty tool calls with
 * the main conversation would put the research back in the middle of the answer,
 * which is what delegation was for.
 */

/** `read_file ×4 · grep` -- what it did, not how many calls it took. */
function toolCounts(steps: SubagentState['steps']): string {
  const counts = new Map<string, number>();
  for (const s of steps) counts.set(s.name, (counts.get(s.name) ?? 0) + 1);
  return [...counts.entries()]
    .map(([name, n]) => (n > 1 ? `${name} ×${n}` : name))
    .join(' · ');
}

function SubagentRow({ agent }: { agent: SubagentState }) {
  const [open, setOpen] = useState(false);
  const stepsId = useId();
  const busy = !agent.done;
  const failed = agent.steps.filter((s) => s.status === 'failed').length;
  const status: ChipStatus = busy ? 'running' : failed > 0 ? 'failed' : 'ok';

  return (
    <li className={styles.item}>
      {/*
       * A real `<button>` with `aria-expanded`. It used to be a `<div role="button">`
       * with a hand-written key handler, which got Space wrong (the page scrolled as
       * well as toggling) and never told anyone the row was a disclosure.
       */}
      <button
        type="button"
        className={styles.row}
        aria-expanded={open}
        aria-controls={open ? stepsId : undefined}
        onClick={() => setOpen((o) => !o)}
      >
        <StatusDot status={status} pulse={busy} />
        <span className={styles.label} title={agent.label}>
          {agent.label || '(no prompt)'}
        </span>
        {agent.depth > 1 && (
          // Nesting is worth a mark: a depth-2 agent was delegated *by* an agent,
          // and that changes how much you should trust its summary.
          <span className={styles.meta}>d{agent.depth}</span>
        )}
        <span className={styles.meta}>{agent.steps.length} steps</span>
        {/* The dot carries the colour; the word only names the state. Two coloured
            texts in a 320px column is a row that reads as an alert. */}
        <span className={styles.state}>{busy ? 'running' : failed > 0 ? 'failed' : 'done'}</span>
      </button>
      {open && (
        <div id={stepsId} className={styles.body}>
          {agent.steps.length === 0 ? (
            <p className={styles.none}>No tool calls yet</p>
          ) : (
            <>
              <p className={styles.tally}>{toolCounts(agent.steps)}</p>
              <div className={styles.steps}>
                {agent.steps.map((t) => (
                  <ToolCard key={t.seq} tool={t} />
                ))}
              </div>
            </>
          )}
        </div>
      )}
    </li>
  );
}

export function AgentsPane({ subagents }: { subagents: SubagentState[] }) {
  if (subagents.length === 0) {
    return (
      <EmptyState
        icon={Bot}
        title="No subagents this turn"
        hint="When the agent calls the task tool it delegates to a read-only research subagent, and that subagent's steps appear here rather than being interleaved with the conversation."
      />
    );
  }
  return (
    <ul data-ui="agents-pane" className={styles.list}>
      {subagents.map((a) => (
        <SubagentRow key={a.id} agent={a} />
      ))}
    </ul>
  );
}
