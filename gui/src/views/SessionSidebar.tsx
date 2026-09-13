import { useState } from 'react';
import { Button, Input, List, Popconfirm, Tag, Tooltip, Typography } from 'antd';
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

/**
 * Chip colours for one row.
 *
 * A chip's own pair (`successBg` / `successInk` and friends) is measured against
 * `bg` and `surface`, because that is where chips normally live. On a selected
 * row neither ground is what the chip is sitting on, and the collision is not
 * uniform: the transparent `NORMAL` chip is fine on dark acid and 1.3:1 on the
 * light green fill, which is exactly the kind of bug a scheme switch is supposed
 * to surface rather than hide.
 *
 * So a selected row inverts its chips instead: ground `onSelection`, ink
 * `selection`. That pair is readable by construction (13.28:1 dark, 6.21:1
 * light), needs no per-chip ground decisions, and costs only the chip's hue on
 * the one row that is already marked by its fill -- the label itself still says
 * which kind it is.
 */
function chipStyle(
  selected: boolean,
  pair: { background: string; color: string; border?: string },
): { background: string; color: string; border: string } {
  return selected
    ? { background: color.onSelection, color: color.selection, border: color.onSelection }
    : { background: pair.background, color: pair.color, border: pair.border ?? color.outline };
}

/** The one shape every chip on a row shares, so the only thing that differs
 *  between them is what they say. `flex: 0 0 auto` matters as much as the
 *  colours: without it a chip gives up width before the title does, and the
 *  title is the part you are reading. */
function tagStyle(c: { background: string; color: string; border: string }, fontSize: number) {
  return {
    fontSize,
    marginRight: 0,
    borderRadius: radius.chip,
    border: `1px solid ${c.border}`,
    background: c.background,
    color: c.color,
    lineHeight: '16px',
    flex: '0 0 auto',
  } as const;
}

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
  // Pointer feedback. `transition: background` on a row was animating a change
  // nothing ever made, which left a list of clickable rows that gave no sign of
  // being clickable.
  const [hovered, setHovered] = useState<string | null>(null);

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
    const selected = s.id === currentId;
    return (
    <List.Item
      key={s.id}
      onClick={() => onSelect(s.id)}
      onMouseEnter={() => setHovered(s.id)}
      onMouseLeave={() => setHovered((h) => (h === s.id ? null : h))}
      style={{
        cursor: 'pointer',
        borderRadius: radius.tile,
        padding: '8px 10px',
        paddingLeft: 10 + depth * 16,
        background: selected ? color.selection : hovered === s.id ? color.hover : undefined,
        // Present in both states so selecting a row never shifts its text by a
        // pixel -- and transparent in both, because the fill is the signal. A
        // grey ring around a saturated ground is what made the old selection
        // look like a bordered box that happened to be green.
        border: '1px solid transparent',
        boxShadow: selected ? color.shadowSm : undefined,
        transition: 'background 0.15s ease',
        minWidth: 0,
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
                  // `onSelection`, not `ink`: body ink on the dark scheme's
                  // acid selection is 1.01:1, which is not a dimmed icon, it is
                  // a missing one.
                  style={{ color: selected ? color.onSelection : color.infoInk }}
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
            style={{ color: selected ? color.onSelection : undefined }}
          />
        </Popconfirm>,
      ]}
    >
      {/*
        Our own flex column rather than `List.Item.Meta`.

        antd's meta wrapper is a flex item with `flex: 1` and no `min-width: 0`,
        so its width came from the longest thing inside it: a first message with
        a long word in it pushed the row past the rail and the text clipped
        mid-glyph, with nothing on screen to say there was more. `minWidth: 0`
        down this chain is what lets the title's own ellipsis do its job.
      */}
      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          gap: 2,
          flex: '1 1 auto',
          minWidth: 0,
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 4, minWidth: 0 }}>
          {/* Category tag — exactly one per session. */}
          {s.kind === 'mainline' && (
            <Tag
              style={tagStyle(
                chipStyle(selected, { background: color.successBg, color: color.successInk }),
                10,
              )}
            >
              MAINLINE
            </Tag>
          )}
          {(s.kind === 'branch' || depth > 0) && (
            <Tag
              style={tagStyle(
                chipStyle(selected, {
                  background: color.surfaceRaised,
                  color: color.infoInk,
                }),
                10,
              )}
            >
              ↳ BRANCH
            </Tag>
          )}
          {s.kind === 'normal' && (
            <Tag
              style={tagStyle(
                chipStyle(selected, {
                  // Opaque even when unselected: `transparent` was the one chip
                  // ground that let a selected row's fill show through the badge
                  // and put green text on green.
                  background: color.surface,
                  color: color.successInk,
                  border: color.successBorder,
                }),
                10,
              )}
            >
              NORMAL
            </Tag>
          )}
          <Text
            style={{
              fontSize: 13,
              fontWeight: selected ? 700 : 500,
              color: selected ? color.onSelection : color.ink,
              flex: '1 1 auto',
              minWidth: 0,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
            ellipsis={{ tooltip: s.preview }}
          >
            {/* ellipsis carries the FULL preview in its tooltip — the old
                code truncated first, so the tooltip showed the same 30
                chars as the row. */}
            {s.preview}
          </Text>
        </div>
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 4,
            flexWrap: 'wrap',
            minWidth: 0,
          }}
        >
          {runningIds?.has(s.id) && (
            <Tag
              // No `color=` prop: antd takes a preset colour name and derives
              // its own background, and what it derives is a white-based tint
              // (rgb(241,254,231)) even in dark mode. The fill has to be stated.
              style={{
                ...tagStyle(
                  chipStyle(selected, {
                    background: color.brandAcid,
                    color: color.onAcid,
                    border: color.brandAcid,
                  }),
                  10,
                ),
                fontWeight: 700,
              }}
            >
              ⚡ running
            </Tag>
          )}
          <Tag
            style={tagStyle(
              chipStyle(selected, { background: color.surfaceRaised, color: color.muted }),
              11,
            )}
          >
            {s.model}
          </Tag>
          <Text style={{ fontSize: 11, color: selected ? color.onSelection : color.muted }}>
            {new Date(s.updated_at * 1000).toLocaleString()}
          </Text>
        </div>
      </div>
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
