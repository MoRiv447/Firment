import type { CSSProperties } from 'react';
import { ListChecks } from 'lucide-react';

import { EmptyState } from '../../ui';
import type { TodoDto } from '../../types';
import styles from './TodosPane.module.css';

/**
 * The session's todo list, as the agent's `todo` tool left it.
 *
 * The list belongs to the *agent*, not to the reader: it is how a long task
 * keeps its own place, and the tool's contract says it survives context
 * compaction. So this pane is read-only. A checkbox that looked clickable and
 * did nothing would be worse than no checkbox, and one that worked would be a
 * second writer racing the tool's atomic save.
 *
 * It refreshes when the tool reports, so the list tracks the agent's own idea of
 * progress rather than a poll interval.
 */
export function TodosPane({ todos, loading }: { todos: TodoDto[]; loading: boolean }) {
  if (todos.length === 0) {
    return (
      <EmptyState
        icon={ListChecks}
        title={loading ? 'Loading…' : 'No todos in this session yet'}
        hint={
          loading
            ? undefined
            : 'When the agent breaks a multi-step task down with the todo tool the list appears here — it lives in the session directory and survives context compaction.'
        }
      />
    );
  }

  const done = todos.filter((t) => t.done).length;
  const pct = Math.round((done / todos.length) * 100);
  // The first unfinished item is "where it is now": everything above it is
  // already accounted for, and marking it is what makes the list readable at a
  // glance rather than a list you have to scan.
  const currentAt = todos.findIndex((t) => !t.done);

  return (
    <div data-ui="todos-pane" className={styles.root}>
      <div className={styles.head}>
        <span className={styles.tally}>
          {done}/{todos.length}
        </span>
        {/*
         * A progressbar with numbers, not just a line of pixels. `aria-valuenow`
         * is the count rather than the rounded percentage so the announcement and
         * the number next to it are the same fact.
         */}
        <span
          className={styles.bar}
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={todos.length}
          aria-valuenow={done}
          aria-valuetext={`${done} of ${todos.length} done`}
          style={{ '--progress': `${pct}%` } as CSSProperties}
        />
      </div>
      <ol className={styles.list}>
        {todos.map((t, i) => (
          <li
            key={`${i}-${t.text}`}
            className={styles.item}
            data-done={t.done ? 'true' : undefined}
            data-current={i === currentAt ? 'true' : undefined}
          >
            {/* Three glyphs, not three colours: the mark has to survive a
                colour-blind reader and a printed screenshot alike. */}
            <span aria-hidden className={styles.mark}>
              {t.done ? '✓' : i === currentAt ? '▸' : '○'}
            </span>
            <span className={styles.text}>{t.text}</span>
          </li>
        ))}
      </ol>
    </div>
  );
}

/** The one-line form for the status bar: `3/7`. */
export function todoSummary(todos: TodoDto[]): string | null {
  if (todos.length === 0) return null;
  return `${todos.filter((t) => t.done).length}/${todos.length}`;
}
