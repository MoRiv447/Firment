import { useState } from 'react';
import { font, radius, color, space, statusChip } from '../../styles/tokens';
import { ToolCard } from '../../components/ToolCard';
import type { SubagentState } from '../../lib/turnReducer';

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
  const busy = !agent.done;
  const failed = agent.steps.filter((s) => s.status === 'failed').length;
  const chip = statusChip(busy ? 'running' : failed > 0 ? 'failed' : 'ok');

  return (
    <div style={{ borderBottom: `1px solid ${color.line}` }}>
      <div
        onClick={() => setOpen((o) => !o)}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') setOpen((o) => !o);
        }}
        style={{
          display: 'flex',
          alignItems: 'baseline',
          gap: 8,
          padding: '7px 0',
          cursor: 'pointer',
          fontFamily: font.sans,
        }}
      >
        <span
          aria-hidden
          style={{
            width: 6,
            height: 6,
            flex: '0 0 auto',
            borderRadius: radius.chip,
            background: chip.color,
            transform: 'translateY(-1px)',
          }}
        />
        <span
          style={{
            flex: 1,
            minWidth: 0,
            fontSize: 12,
            color: color.ink,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
          title={agent.label}
        >
          {agent.label || '(no prompt)'}
        </span>
        {agent.depth > 1 && (
          // Nesting is worth a mark: a depth-2 agent was delegated *by* an agent,
          // and that changes how much you should trust its summary.
          <span style={{ fontSize: 10, color: color.muted, fontFamily: font.mono }}>
            d{agent.depth}
          </span>
        )}
        <span style={{ fontSize: 10, color: color.muted, fontFamily: font.mono, whiteSpace: 'nowrap' }}>
          {agent.steps.length} 步
        </span>
        <span style={{ fontSize: 10, color: chip.color, fontFamily: font.sans, whiteSpace: 'nowrap' }}>
          {busy ? '进行中' : failed > 0 ? '有失败' : '已完成'}
        </span>
      </div>
      {open && (
        <div style={{ paddingBottom: 8 }}>
          {agent.steps.length === 0 ? (
            <span style={{ fontSize: 11, color: color.muted }}>还没有工具调用</span>
          ) : (
            <>
              <div
                style={{
                  fontSize: 11,
                  color: color.muted,
                  fontFamily: font.mono,
                  marginBottom: 6,
                }}
              >
                {toolCounts(agent.steps)}
              </div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: space.controlGap }}>
                {agent.steps.map((t) => (
                  <ToolCard key={t.seq} tool={t} />
                ))}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}

export function AgentsPane({ subagents }: { subagents: SubagentState[] }) {
  if (subagents.length === 0) {
    return (
      <div style={{ fontSize: 11, lineHeight: 1.6, color: color.muted, fontFamily: font.sans }}>
        本轮没有派生子代理。agent 调用 <code>task</code> 工具时会派一个只读的研究子代理，
        它的步骤会显示在这里，而不是混进主对话。
      </div>
    );
  }
  return (
    <div style={{ display: 'flex', flexDirection: 'column' }}>
      {subagents.map((a) => (
        <SubagentRow key={a.id} agent={a} />
      ))}
    </div>
  );
}
