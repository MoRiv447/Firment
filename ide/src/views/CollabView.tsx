import { Card, Empty, Space, Tag, Typography } from 'antd';
import { TeamOutlined } from '@ant-design/icons';

const { Text } = Typography;

/**
 * Collaboration panel (M4). The transport abstraction (CollabBackend) is
 * already wired in the backend; this view will consume `collab-event`
 * streams pushed from remote backends, and show presence + remote file
 * changes. For now it occupies the UI slot so the layout is stable.
 */
export function CollabView() {
  return (
    <div style={{ padding: 20, height: '100%', overflowY: 'auto' }}>
      <Card size="small" title="Team collaboration">
        <Space direction="vertical" size={12} style={{ width: '100%' }}>
          <Text type="secondary">
            Shared workspace sync is coming in M4. Planned capabilities:
          </Text>
          <Space wrap size={6}>
            {['presence', 'session event stream', 'file change stream', 'serial/peripherals lock'].map(
              (t) => (
                <Tag key={t} icon={<TeamOutlined />} color="blue">
                  {t}
                </Tag>
              ),
            )}
          </Space>
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={
              <Text type="secondary" style={{ fontSize: 12 }}>
                Outside the scope of this build - the backend seam
                (`CollabBackend` trait) is ready for a Git-based or relay-based
                implementation.
              </Text>
            }
          />
        </Space>
      </Card>
    </div>
  );
}