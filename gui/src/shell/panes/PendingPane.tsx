import { font, color } from '../../styles/tokens';

/**
 * The four panes the inspector can show.
 *
 * Three of them are honest placeholders: they say what will be here and why it
 * is not yet, rather than rendering an empty box that looks broken. A pane that
 * claims to be a feature and shows nothing is worse than one that admits it is
 * not built.
 *
 * Delivery order: 硬件 is wired now (the views already exist), 改动 lands with
 * the change cards, 子代理 and 待办 with the event fields the core needs to
 * attribute a step to the agent that took it.
 */
export function PendingPane({ title, body }: { title: string; body: string }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      <div style={{ fontSize: 12, fontWeight: 600, color: color.ink, fontFamily: font.sans }}>
        {title}
      </div>
      <div style={{ fontSize: 11, lineHeight: 1.6, color: color.muted, fontFamily: font.sans }}>
        {body}
      </div>
    </div>
  );
}
