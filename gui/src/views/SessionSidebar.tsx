import { Button, Input, List, Popconfirm, Space, Tag, Tooltip, Typography } from 'antd';
import {
  DeleteOutlined,
  FolderOpenOutlined,
  SafetyCertificateOutlined,
  ThunderboltOutlined,
} from '@ant-design/icons';
import type { SessionSummaryDto } from '../types';
import pkg from '../../package.json';

const { Text } = Typography;

export function SessionSidebar({
  sessions,
  currentId,
  workCwd,
  onWorkCwd,
  onSelect,
  onNew,
  onDelete,
  onOpenWorkbench,
}: {
  sessions: SessionSummaryDto[];
  currentId: string | null;
  workCwd: string;
  onWorkCwd: (cwd: string) => void;
  onSelect: (id: string) => void;
  onNew: (mode: 'agent' | 'plan') => void;
  onDelete: (id: string) => void;
  /** Open the Workbench view scoped to this session's project path. */
  onOpenWorkbench: (cwd: string) => void;
}) {
  // ---- build the session tree -------------------------------------------
  // Branch sessions (parent_session set) nest under their parent; everything
  // else is a root. Roots WITH children are project mainlines and get a
  // workbench jump button.
  const byParent = new Map<string, SessionSummaryDto[]>();
  const roots: SessionSummaryDto[] = [];
  for (const s of sessions) {
    if (s.parent_session) {
      const arr = byParent.get(s.parent_session) ?? [];
      arr.push(s);
      byParent.set(s.parent_session, arr);
    } else {
      roots.push(s);
    }
  }
  roots.sort((a, b) => b.updated_at - a.updated_at);
  for (const [, arr] of byParent) arr.sort((a, b) => a.updated_at - b.updated_at);

  const renderRow = (s: SessionSummaryDto, depth: number) => {
    const kids = byParent.get(s.id) ?? [];
    const isProjectRoot = depth === 0 && kids.length > 0;
    return (
      <div key={s.id}>
        {renderItem(s, depth, isProjectRoot)}
        {kids.map((k) => renderRow(k, depth + 1))}
      </div>
    );
  };

  const renderItem = (s: SessionSummaryDto, depth: number, isProjectRoot: boolean) => (
    <List.Item
      key={s.id}
      onClick={() => onSelect(s.id)}
      style={{
        cursor: 'pointer',
        borderRadius: 0,
        padding: '8px 10px',
        paddingLeft: 10 + depth * 16,
        background: s.id === currentId ? '#2f6bff' : undefined,
        border: s.id === currentId ? '3px solid #000' : '3px solid transparent',
        boxShadow: s.id === currentId ? '3px 3px 0 #000' : undefined,
        transition: 'background 0.15s ease',
      }}
      actions={[
        ...(isProjectRoot
          ? [
              <Tooltip key="wb" title="Open this project's workbench">
                <Button
                  size="small"
                  type="text"
                  icon={<FolderOpenOutlined />}
                  onClick={(e) => {
                    e.stopPropagation();
                    onOpenWorkbench(s.cwd);
                  }}
                  style={{ color: s.id === currentId ? '#fff' : '#7dd3fc' }}
                />
              </Tooltip>,
            ]
          : []),
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
          <Space size={4}>
            {(s.kind === 'branch' || depth > 0) && (
              <Tag
                style={{
                  fontSize: 10,
                  marginRight: 0,
                  borderRadius: 0,
                  border: '2px solid #000',
                  background: '#1a1e26',
                  color: '#7dd3fc',
                  lineHeight: '16px',
                }}
              >
                ↳ BRANCH
              </Tag>
            )}
            {isProjectRoot && (
              <Tag
                style={{
                  fontSize: 10,
                  marginRight: 0,
                  borderRadius: 0,
                  border: '2px solid #000',
                  background: '#14532d',
                  color: '#bbf7d0',
                  lineHeight: '16px',
                }}
              >
                MAINLINE
              </Tag>
            )}
            <Text
              style={{
                fontSize: 13,
                fontWeight: s.id === currentId ? 700 : 500,
                color: s.id === currentId ? '#fff' : '#e6e9ef',
              }}
              ellipsis={{ tooltip: true }}
            >
              {s.preview.length > 30 ? `${s.preview.slice(0, 30)}…` : s.preview}
            </Text>
          </Space>
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
  );

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
        dataSource={roots}
        style={{ overflow: 'auto', flex: 1 }}
        renderItem={(root) => renderRow(root, 0)}
      />
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          marginTop: 'auto',
          paddingTop: 6,
          borderTop: '2px solid #000000',
          color: '#9aa3b2',
        }}
      >
        <Text style={{ fontSize: 10, letterSpacing: 1.2 }}>FIRMENT GUI</Text>
        <Text style={{ fontSize: 10, fontFamily: 'Consolas, monospace' }}>v{pkg.version}</Text>
      </div>
    </div>
  );
}
