import { Badge, Button, Popover, Space, Typography } from 'antd';
import { BellOutlined } from '@ant-design/icons';
import { font, radius, color, statusChip } from '../styles/tokens';
import type { StatusKind } from '../styles/tokens';
import type { NotificationEntry } from '../types';

const { Text } = Typography;

/**
 * The notification bell and its panel, lifted out of `App.tsx`.
 *
 * It was 90 lines of JSX in the middle of the shell's render, which is part of
 * why that file reached 1038 lines. Nothing here is shell logic -- it takes a
 * list and three callbacks.
 *
 * The kind-to-colour mapping is the one rule this file owns, and it goes through
 * `statusChip` rather than an antd preset name, so a guard alert and a failed
 * flash cannot drift into two different reds.
 */
const KIND_STATUS: Record<string, StatusKind> = {
  guard: 'failed',
  'build-fail': 'failed',
  'verify-fail': 'failed',
  'flash-fail': 'failed',
  'device-offline': 'neutral',
};

export function NotificationBell({
  notifications,
  unread,
  open,
  onOpenChange,
  onMarkAllRead,
  onClear,
  onOpenSession,
}: {
  notifications: NotificationEntry[];
  unread: number;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onMarkAllRead: () => void;
  onClear: () => void;
  /** Jump to the session a notification came from. */
  onOpenSession: (sid: string) => void;
}) {
  return (
    <Popover
      trigger="click"
      placement="bottomRight"
      open={open}
      onOpenChange={onOpenChange}
      content={
        <div style={{ width: 380, maxHeight: 420, overflowY: 'auto', fontFamily: font.sans }}>
          <Space style={{ marginBottom: 8, width: '100%', justifyContent: 'space-between' }}>
            <Button size="small" type="text" disabled={unread === 0} onClick={onMarkAllRead}>
              Mark all read
            </Button>
            <Button size="small" type="text" onClick={onClear}>
              Clear
            </Button>
          </Space>
          {notifications.length === 0 ? (
            <Text type="secondary" style={{ fontSize: 12 }}>
              No notifications yet. Guard alerts, build/verify/flash failures and node offline events
              land here.
            </Text>
          ) : (
            notifications.map((n) => {
              const chip = statusChip(KIND_STATUS[n.kind] ?? 'running');
              return (
                <div
                  key={n.id}
                  onClick={() => n.sid && onOpenSession(n.sid)}
                  style={{
                    display: 'flex',
                    gap: 8,
                    padding: '6px 4px',
                    borderBottom: `1px solid ${color.line}`,
                    cursor: n.sid ? 'pointer' : 'default',
                  }}
                >
                  <span
                    aria-hidden
                    style={{
                      width: 6,
                      height: 6,
                      marginTop: 6,
                      flex: '0 0 auto',
                      borderRadius: radius.chip,
                      background: chip.color,
                    }}
                  />
                  <div style={{ minWidth: 0 }}>
                    <Text style={{ fontSize: 12, fontWeight: 600 }}>{n.title}</Text>
                    <div>
                      <Text type="secondary" style={{ fontSize: 11 }}>
                        {n.kind} · {new Date(n.ts).toLocaleString()}
                      </Text>
                    </div>
                    <Text type="secondary" style={{ fontSize: 11 }}>
                      {n.body}
                    </Text>
                  </div>
                </div>
              );
            })
          )}
        </div>
      }
    >
      <Badge count={unread} size="small">
        <Button type="text" aria-label="Notifications" icon={<BellOutlined />} />
      </Badge>
    </Popover>
  );
}
