import { Button, Input, List, Popconfirm, Space, Tag, Tooltip, Typography } from 'antd';
import {
  DeleteOutlined,
  FolderOpenOutlined,
  SafetyCertificateOutlined,
  ThunderboltOutlined,
} from '@ant-design/icons';
import type { SessionSummaryDto } from '../types';
import pkg from '../../package.json';
import { color, font, radius, space } from '../styles/tokens';

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
  runningIds,
}: {
  sessions: SessionSummaryDto[];
  currentId: string | null;
  workCwd: string;
  onWorkCwd: (cwd: string) => void;
  onSelect: (id: string) => void;
  onNew: (mode: 'agent' | 'plan') => void;
  onDelete: (id: string) => void;
  /** Sessions with a turn currently streaming in the background. */
  runningIds?: Set<string>;
  /** Open the Workbench view scoped to this session's project path. */
  onOpenWorkbench: (cwd: string) => void;
}) {
  // ---- build the session tree -------------------------------------------
  // Branch sessions (parent_session set) nest under their parent; everything
  // else is a root. Roots WITH children are project mainlines and get a
  // workbench jump button. Orphaned branches (parent deleted) are hoisted to
  // roots so they never vanish from the list.
  const ids = new Set(sessions.map((s) => s.id));
  const byParent = new Map<string, SessionSummaryDto[]>();
  const roots: SessionSummaryDto[] = [];
  for (const s of sessions) {
    if (s.parent_session && ids.has(s.parent_session)) {
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
    return (
      <div key={s.id}>
        {renderItem(s, depth, kids)}
        {kids.map((k) => renderRow(k, depth + 1))}
      </div>
    );
  };

  const renderItem = (s: SessionSummaryDto, depth: number, kids: SessionSummaryDto[]) => {
    // Category tag: every session carries exactly one of NORMAL / MAINLINE /
    // BRANCH so the workbench model is visible at a glance. MAINLINE badge:
    // a main-kind session with nested branches. The folder button: any
    // session with children is a project root worth jumping from.
    const isProjectRoot = s.kind === 'mainline' && kids.length > 0;
    return (
    <List.Item
      key={s.id}
      onClick={() => onSelect(s.id)}
      style={{
        cursor: 'pointer',
        borderRadius: radius.tile,
        padding: '8px 10px',
        paddingLeft: 10 + depth * 16,
        background: s.id === currentId ? color.brandAcid : undefined,
        border: s.id === currentId ? `1px solid ${color.outline}` : '1px solid transparent',
        boxShadow: s.id === currentId ? color.shadowMd : undefined,
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
                  style={{ color: s.id === currentId ? color.ink : color.infoInk }}
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
            style={{ color: s.id === currentId ? color.ink : undefined }}
          />
        </Popconfirm>,
      ]}
    >
      <List.Item.Meta
        title={
          <Space size={4}>
            {/* Category tag — exactly one per session. */}
            {s.kind === 'mainline' && (
              <Tag
                style={{
                  fontSize: 10,
                  marginRight: 0,
                  borderRadius: radius.chip,
                  border: `1px solid ${color.outline}`,
                  background: color.successBg,
                  color: color.successInk,
                  lineHeight: '16px',
                }}
              >
                MAINLINE
              </Tag>
            )}
            {(s.kind === 'branch' || depth > 0) && (
              <Tag
                style={{
                  fontSize: 10,
                  marginRight: 0,
                  borderRadius: radius.chip,
                  border: `1px solid ${color.outline}`,
                  background: color.surfaceRaised,
                  color: color.infoInk,
                  lineHeight: '16px',
                }}
              >
                ↳ BRANCH
              </Tag>
            )}
            {s.kind === 'normal' && (
              <Tag
                style={{
                  fontSize: 10,
                  marginRight: 0,
                  borderRadius: radius.chip,
                  border: `1px solid ${color.successBorder}`,
                  background: 'transparent',
                  color: color.successInk,
                  lineHeight: '16px',
                }}
              >
                NORMAL
              </Tag>
            )}
            <Text
              style={{
                fontSize: 13,
                fontWeight: s.id === currentId ? 700 : 500,
                color: s.id === currentId ? color.onAcid : color.ink,
              }}
              ellipsis={{ tooltip: s.preview }}
            >
              {/* ellipsis carries the FULL preview in its tooltip — the old
                  code truncated first, so the tooltip showed the same 30
                  chars as the row. */}
              {s.preview}
            </Text>
          </Space>
        }
        description={
          <Space size={4} wrap>
            {runningIds?.has(s.id) && (
              <Tag
                color={color.brandAcid}
                style={{
                  fontSize: 10,
                  marginRight: 0,
                  borderRadius: radius.chip,
                  color: color.onAcid,
                  fontWeight: 700,
                  lineHeight: '16px',
                }}
              >
                ⚡ running
              </Tag>
            )}
            <Tag
              style={{
                fontSize: 11,
                marginRight: 0,
                borderRadius: radius.chip,
                border: `1px solid ${color.outline}`,
                background: s.id === currentId ? color.bg : color.surfaceRaised,
                color: s.id === currentId ? color.ink : color.muted,
              }}
            >
              {s.model}
            </Tag>
            <Text
              type="secondary"
              style={{ fontSize: 11, color: s.id === currentId ? color.ink : color.muted }}
            >
              {new Date(s.updated_at * 1000).toLocaleString()}
            </Text>
          </Space>
        }
      />
    </List.Item>
    );
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', padding: 4, gap: 6 }}>
      {/*
        Two separate buttons, not `Space.Compact`.

        Compact welds adjacent buttons into one block with shared edges, and
        both of these were acid-filled -- so the sidebar opened with two green
        rectangles fused together, which reads as one malformed control rather
        than as two actions. They are also not the same weight: "New" is the
        primary action and the plan-mode variant is its sibling, so only one of
        them is filled. Neither carries a border any more: an acid fill inside a
        grey ring looks like a mistake, which is how it read.
      */}
      <div style={{ display: 'flex', gap: space.controlGap }}>
        <Tooltip title="New agent session (uses cwd below)">
          <Button
            icon={<ThunderboltOutlined />}
            onClick={() => onNew('agent')}
            type="primary"
            style={{ flex: 1, fontWeight: 700 }}
          >
            New
          </Button>
        </Tooltip>
        <Tooltip title="New plan-mode session (read-only tools)">
          <Button icon={<SafetyCertificateOutlined />} onClick={() => onNew('plan')} />
        </Tooltip>
      </div>
      <Input
        placeholder="working dir (default C:\)"
        size="small"
        value={workCwd}
        onChange={(e) => onWorkCwd(e.target.value)}
        style={{
          background: color.bg,
          border: `1px solid ${color.outline}`,
          borderRadius: radius.control,
          color: color.ink,
          fontFamily: font.mono,
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
          borderTop: `1px solid ${color.line}`,
          color: color.muted,
        }}
      >
        <Text style={{ fontSize: 10, letterSpacing: 1.2 }}>FIRMENT GUI</Text>
        <Text style={{ fontSize: 10, fontFamily: font.mono }}>v{pkg.version}</Text>
      </div>
    </div>
  );
}
