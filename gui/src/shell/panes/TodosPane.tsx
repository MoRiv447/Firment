import { font, color } from '../../styles/tokens';
import type { TodoDto } from '../../types';

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
      <div style={{ fontSize: 11, lineHeight: 1.6, color: color.muted, fontFamily: font.sans }}>
        {loading
          ? 'Loading…'
          : 'No todos in this session yet. When the agent breaks a multi-step task down with the todo tool the list appears here — it lives in the session directory and survives context compaction.'}
      </div>
    );
  }

  const done = todos.filter((t) => t.done).length;
  const pct = Math.round((done / todos.length) * 100);
  // The first unfinished item is "where it is now": everything above it is
  // already accounted for, and marking it is what makes the list readable at a
  // glance rather than a list you have to scan.
  const currentAt = todos.findIndex((t) => !t.done);

  return (
    <div style={{ fontFamily: font.sans }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'baseline',
          gap: 8,
          marginBottom: 8,
          fontSize: 11,
          color: color.muted,
        }}
      >
        <span style={{ fontFamily: font.mono, color: color.ink }}>
          {done}/{todos.length}
        </span>
        <span style={{ flex: 1, height: 2, background: color.line, position: 'relative' }}>
          <span
            style={{
              position: 'absolute',
              inset: 0,
              width: `${pct}%`,
              background: color.stepRule,
            }}
          />
        </span>
      </div>
      <ol style={{ listStyle: 'none', margin: 0, padding: 0 }}>
        {todos.map((t, i) => {
          const current = i === currentAt;
          return (
            <li
              key={`${i}-${t.text}`}
              style={{
                display: 'flex',
                gap: 8,
                padding: '5px 0',
                borderBottom: `1px solid ${color.line}`,
                fontSize: 12,
                lineHeight: 1.5,
                color: t.done ? color.muted : color.ink,
                // Done items are struck through rather than removed: the list is
                // the record of the plan, and a plan that erases itself cannot be
                // checked against what actually happened.
                textDecoration: t.done ? 'line-through' : undefined,
                fontWeight: current ? 600 : 400,
              }}
            >
              <span
                aria-hidden
                style={{
                  width: 12,
                  flex: '0 0 auto',
                  fontFamily: font.mono,
                  color: t.done ? color.successInk : current ? color.stepRule : color.muted,
                }}
              >
                {t.done ? '✓' : current ? '▸' : '○'}
              </span>
              <span style={{ minWidth: 0, wordBreak: 'break-word' }}>{t.text}</span>
            </li>
          );
        })}
      </ol>
    </div>
  );
}

/** The one-line form for the status bar: `✓ 3/7`. */
export function todoSummary(todos: TodoDto[]): string | null {
  if (todos.length === 0) return null;
  return `${todos.filter((t) => t.done).length}/${todos.length}`;
}
