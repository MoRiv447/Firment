import { Button, Input, List, Popconfirm, Space, Tag, Tooltip, Typography } from 'antd';
import { DeleteOutlined, SafetyCertificateOutlined, ThunderboltOutlined } from '@ant-design/icons';
import type { SessionSummaryDto } from '../types';

const { Text } = Typography;

export function SessionSidebar({
  sessions,
  currentId,
  workCwd,
  onWorkCwd,
  onSelect,
  onNew,
  onDelete,
}: {
  sessions: SessionSummaryDto[];
  currentId: string | null;
  workCwd: string;
  onWorkCwd: (cwd: string) => void;
  onSelect: (id: string) => void;
  onNew: (mode: 'agent' | 'plan') => void;
  onDelete: (id: string) => void;
}) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', padding: 4, gap: 6 }}>
      <Space.Compact style={{ width: '100%' }}>
        <Tooltip title="New agent session (uses cwd below)">
          <Button
            icon={<ThunderboltOutlined />}
            onClick={() => onNew('agent')}
            type="primary"
            style={{ flex: 1, borderRadius: 0, border: '3px solid #000', boxShadow: '3px 3px 0 #000', fontWeight: 700 }}
          >
            New
          </Button>
        </Tooltip>
        <Tooltip title="New plan-mode session (read-only tools)">
          <Button
            icon={<SafetyCertificateOutlined />}
            onClick={() => onNew('plan')}
            style={{
              borderRadius: 0,
              border: '3px solid #000',
              background: '#facc15',
              color: '#000',
              boxShadow: '3px 3px 0 #000',
              fontWeight: 700,
            }}
          />
        </Tooltip>
      </Space.Compact>
      <Input
        placeholder="working dir (default C:\)"
        size="small"
        value={workCwd}
        onChange={(e) => onWorkCwd(e.target.value)}
        style={{
          background: '#0a0c10',
          border: '2px solid #000',
          borderRadius: 0,
          color: '#e6e9ef',
          fontFamily: 'Consolas, monospace',
        }}
      />
      <List
        size="small"
        dataSource={sessions}
        style={{ overflow: 'auto', flex: 1 }}
        renderItem={(s) => (
          <List.Item
            onClick={() => onSelect(s.id)}
            style={{
              cursor: 'pointer',
              borderRadius: 0,
              padding: '8px 10px',
              background: s.id === currentId ? '#2f6bff' : undefined,
              border: s.id === currentId ? '3px solid #000' : '3px solid transparent',
              boxShadow: s.id === currentId ? '3px 3px 0 #000' : undefined,
              transition: 'background 0.15s ease',
            }}
            actions={[
              <Popconfirm
                key="del"
                title="Delete this session?"
                onConfirm={(e) => {
                  e?.stopPropagation();
                  onDelete(s.id);
                }}
              >
                <Button
                  size="small"
                  type="text"
                  icon={<DeleteOutlined />}
                  onClick={(e) => e.stopPropagation()}
                  style={{ color: s.id === currentId ? '#fff' : undefined }}
                />
              </Popconfirm>,
            ]}
          >
            <List.Item.Meta
              title={
                <Text
                  style={{ fontSize: 13, fontWeight: s.id === currentId ? 700 : 500, color: s.id === currentId ? '#fff' : '#e6e9ef' }}
                  ellipsis={{ tooltip: true }}
                >
                  {s.preview.length > 30 ? `${s.preview.slice(0, 30)}…` : s.preview}
                </Text>
              }
              description={
                <Space size={4} wrap>
                  <Tag
                    style={{
                      fontSize: 11,
                      marginRight: 0,
                      borderRadius: 0,
                      border: '2px solid #000',
                      background: s.id === currentId ? '#0a0c10' : '#1a1e26',
                      color: s.id === currentId ? '#fff' : '#9aa3b2',
                    }}
                  >
                    {s.model}
                  </Tag>
                  <Text type="secondary" style={{ fontSize: 11, color: s.id === currentId ? '#dbe3f0' : '#6b7280' }}>
                    {new Date(s.updated_at * 1000).toLocaleString()}
                  </Text>
                </Space>
              }
            />
          </List.Item>
        )}
      />
    </div>
  );
}