import {
  Alert,
  Button,
  Card,
  Empty,
  Input,
  List,
  Modal,
  Space,
  Statistic,
  Tag,
  Typography,
} from 'antd';
import { ReloadOutlined } from '@ant-design/icons';
import { useEffect, useState } from 'react';
import { api } from '../lib/api';
import type {
  ElfCardDto,
  QualityItemDto,
  SessionSummaryDto,
  TimelineEntryDto,
  WorkbenchStateDto,
} from '../types';

const { Text, Title } = Typography;

/**
 * Project workbench (W1): mainline + branch session tree over
 * .firment/workbench.toml, quick repo status. Multi-user scopes and the
 * small-model guard land in W2/W3 (docs/gui-workbench.md).
 */
export function WorkbenchView() {
  const [cwd, setCwd] = useState('');
  const [state, setState] = useState<WorkbenchStateDto | null>(null);
  const [sessions, setSessions] = useState<SessionSummaryDto[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [branchModal, setBranchModal] = useState<{ parentId: string; title: string } | null>(null);
  const [currentSessionId, setCurrentSessionId] = useState<string | null>(null);
  const [elf, setElf] = useState<ElfCardDto | null>(null);
  const [elfError, setElfError] = useState<string | null>(null);
  const [quality, setQuality] = useState<QualityItemDto[]>([]);
  const [timeline, setTimeline] = useState<TimelineEntryDto[]>([]);
  const [projects, setProjects] = useState<string[]>([]);

  const rememberProject = (dir: string) => {
    localStorage.setItem('workbench-last-cwd', dir);
    setProjects((prev) => {
      const norm = dir.replace(/\\/g, '/').replace(/\/$/, '');
      const next = [dir, ...prev.filter((p) => p.replace(/\\/g, '/').replace(/\/$/, '') !== norm)];
      localStorage.setItem('workbench-projects', JSON.stringify(next.slice(0, 8)));
      return next.slice(0, 8);
    });
  };

  const refresh = async (dir: string) => {
    setError(null);
    // Remember the project across view switches and app restarts.
    rememberProject(dir);
    try {
      const wb = await api.workbenchState(dir);
      const all = await api.listSessions();
      setState(wb);
      // Only show sessions belonging to this project root.
      const root = dir.replace(/\\/g, '/').replace(/\/$/, '').toLowerCase();
      setSessions(
        all.filter((s) => s.cwd.replace(/\\/g, '/').toLowerCase().startsWith(root)),
      );
      setCurrentSessionId(null);
      // Return the FRESH state so callers can chain insights on the new
      // mainline without waiting for the next React render.
      return wb;
    } catch (err) {
      setError(String(err));
      return null;
    }
  };

  // Restore the last opened project automatically, so navigating away and
  // back (or restarting the app) lands on the same workbench.
  useEffect(() => {
    try {
      const saved = localStorage.getItem('workbench-projects');
      if (saved) setProjects(JSON.parse(saved) as string[]);
    } catch {
      /* ignore malformed lists */
    }
    const last = localStorage.getItem('workbench-last-cwd');
    if (last) {
      setCwd(last);
      void refresh(last);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // W1d cards: ELF stats + verification badges + change timeline, scoped to
  // the project's mainline session. Takes the FRESH workbench state so the
  // first Open-project click already populates the cards.
  const refreshInsights = async (dir: string, mainlineSession: string) => {
    setElf(null);
    setElfError(null);
    setQuality([]);
    setTimeline([]);
    try {
      setElf(await api.workbenchElf(dir));
    } catch (err) {
      setElfError(String(err));
    }
    try {
      setQuality(await api.workbenchQuality(mainlineSession));
      setTimeline(await api.workbenchTimeline(mainlineSession, 10));
    } catch {
      /* keep cards empty */
    }
  };

  const load = async () => {
    if (!cwd.trim()) return;
    setBusy(true);
    const wb = await refresh(cwd.trim());
    if (wb?.config.mainline_session) {
      await refreshInsights(cwd.trim(), wb.config.mainline_session);
    } else {
      setElf(null);
      setElfError(null);
      setQuality([]);
      setTimeline([]);
    }
    setBusy(false);
  };

  const createBranch = async () => {
    if (!branchModal) return;
    setBusy(true);
    try {
      const id = await api.workbenchBranchCreate(branchModal.parentId, branchModal.title);
      setBranchModal(null);
      await refresh(cwd.trim());
      setCurrentSessionId(id);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const setMainline = async (sessionId: string) => {
    if (!state) return;
    setBusy(true);
    try {
      await api.workbenchSetMainline(state.root, sessionId);
      await refresh(state.root);
      await refreshInsights(state.root, sessionId);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  // One-click mainline bootstrap for a fresh project: create a session rooted
  // at the project path and register it as the mainline in workbench.toml.
  const createMainline = async () => {
    if (!state) return;
    setBusy(true);
    setError(null);
    try {
      const session = await api.newSession(state.root, 'agent');
      await api.workbenchSetMainline(state.root, session.id);
      setCurrentSessionId(session.id);
      const wb = await refresh(state.root);
      if (wb) await refreshInsights(state.root, wb.config.mainline_session);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const loadSession = async (id: string) => {
    setCurrentSessionId(id);
    try {
      await api.loadSession(id);
    } catch (err) {
      setError(String(err));
    }
  };

  // Session-tree kind filter: 'all' shows everything; the other values keep
  // only sessions of that category.
  const [kindFilter, setKindFilter] = useState<'all' | 'normal' | 'mainline' | 'branch'>('all');

  const tree = sessions
    .filter((s) => kindFilter === 'all' || s.kind === kindFilter)
    .map((s) => ({
      ...s,
      isMainline:
        state?.config.mainline_session === s.id || s.kind === 'mainline',
    }));

  return (
    <div style={{ padding: 20, height: '100%', overflowY: 'auto' }}>
      <Card size="small" title="Project workbench">
        <Space direction="vertical" size={12} style={{ width: '100%' }}>
          <Space wrap>
            <Input
              placeholder="project path (e.g. D:\fw\thermostat)"
              style={{ width: 380 }}
              value={cwd}
              onChange={(e) => setCwd(e.target.value)}
              onPressEnter={load}
            />
            <Button type="primary" loading={busy} onClick={load}>
              Open project
            </Button>
          </Space>

          {projects.length > 0 && (
            <Space wrap size={4}>
              <Text type="secondary" style={{ fontSize: 12 }}>
                known projects:
              </Text>
              {projects.map((p) => (
                <Tag
                  key={p}
                  style={{ cursor: 'pointer', fontSize: 12 }}
                  color={p === cwd ? 'blue' : 'default'}
                  onClick={() => {
                    setCwd(p);
                    void load();
                  }}
                >
                  {p}
                </Tag>
              ))}
            </Space>
          )}

          {error && <Alert type="error" showIcon message={error} />}

          {state && (
            <>
              <Card type="inner" title={`Project: ${state.config.project_name || '(unnamed)'}`} size="small">
                <Space wrap size={20}>
                  {state.git ? (
                    <>
                      <Statistic title="branch" value={state.git.branch || '(none)'} />
                      <Statistic title="dirty files" value={state.git.dirty_files} />
                    </>
                  ) : (
                    <Text type="secondary">not a git repository</Text>
                  )}
                  <Statistic
                    title="mainline"
                    value={
                      state.config.mainline_session
                        ? state.config.mainline_session.slice(0, 8)
                        : '(unset)'
                    }
                  />
                </Space>
                {state.config.toml_raw && (
                  <details style={{ marginTop: 8 }}>
                    <summary>
                      <Text type="secondary" style={{ fontSize: 12 }}>
                        .firment/workbench.toml
                      </Text>
                    </summary>
                    <pre style={{ fontSize: 11 }}>{state.config.toml_raw}</pre>
                  </details>
                )}
                {!state.config.toml_raw && (
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    No .firment/workbench.toml yet — creating a branch will generate it.
                  </Text>
                )}
              </Card>

              <Card
                type="inner"
                title="Insights"
                size="small"
                extra={
                  <Button
                    size="small"
                    icon={<ReloadOutlined />}
                    disabled={busy || !state.config.mainline_session}
                    onClick={() => refreshInsights(state.root, state.config.mainline_session)}
                  >
                    refresh
                  </Button>
                }
              >
                {elfError && (
                  <Alert
                    type="warning"
                    showIcon
                    style={{ marginBottom: 12 }}
                    message="ELF budget card unavailable"
                    description={elfError}
                  />
                )}
                {elf && (
                  <Card type="inner" size="small" title="ELF budget" style={{ marginBottom: 12 }}>
                    <Space wrap size={24}>
                      <Statistic title="flash" value={(elf.flash_bytes / 1024).toFixed(1)} suffix="KiB" />
                      <Statistic title="RAM (data+bss)" value={(elf.ram_bytes / 1024).toFixed(1)} suffix="KiB" />
                      <Statistic title="functions" value={elf.functions} />
                    </Space>
                    {elf.gate && (
                      <Text type="secondary" style={{ fontSize: 11, display: 'block', marginTop: 6 }}>
                        gate thresholds: stack +{elf.gate.stack_threshold}B · flash +
                        {elf.gate.flash_threshold_kib}KiB · ram +{elf.gate.ram_threshold_kib}KiB
                        {elf.gate.strict ? ' · strict' : ''}
                      </Text>
                    )}
                    <Text type="secondary" style={{ fontSize: 11, display: 'block' }}>
                      {elf.file}
                    </Text>
                  </Card>
                )}
                {quality.length > 0 && (
                  <Card type="inner" size="small" title="Verification badges (mainline)" style={{ marginBottom: 12 }}>
                    <Space wrap size={8}>
                      {quality.map((q) => (
                        <Tag key={q.tool} color={q.ok ? 'green' : 'red'} style={{ fontSize: 12 }}>
                          {q.tool}: {q.ok ? 'PASS' : 'FAIL'}
                        </Tag>
                      ))}
                    </Space>
                  </Card>
                )}
                {timeline.length > 0 && (
                  <Card type="inner" size="small" title="Change timeline (mainline)">
                    <List
                      size="small"
                      dataSource={timeline}
                      renderItem={(entry) => (
                        <List.Item style={{ padding: '4px 0' }}>
                          <div style={{ width: '100%' }}>
                            <Text type="secondary" style={{ fontSize: 11 }}>
                              #{entry.seq} · {new Date(entry.created_at * 1000).toLocaleString()}
                            </Text>
                            {entry.files.map((f) => (
                              <div key={f.path} style={{ fontSize: 12 }}>
                                <Text code>{f.path}</Text>{' '}
                                <Text type="secondary">
                                  {f.old_lines} → {f.new_lines}
                                </Text>
                              </div>
                            ))}
                          </div>
                        </List.Item>
                      )}
                    />
                  </Card>
                )}
              </Card>

              <Card
                type="inner"
                title="Session tree"
                size="small"
                extra={
                  <Space size={4}>
                    {(['all', 'normal', 'mainline', 'branch'] as const).map((f) => (
                      <Tag.CheckableTag
                        key={f}
                        checked={kindFilter === f}
                        onChange={() => setKindFilter(f)}
                        style={{ fontSize: 11 }}
                      >
                        {f.toUpperCase()}
                      </Tag.CheckableTag>
                    ))}
                  </Space>
                }
              >
                {tree.length === 0 && (
                  <Space direction="vertical" size={8} style={{ width: '100%' }}>
                    <Empty description="No sessions under this path yet" image={Empty.PRESENTED_IMAGE_SIMPLE} />
                    {kindFilter === 'all' && (
                      <Button type="primary" loading={busy} onClick={createMainline}>
                        New mainline chat here
                      </Button>
                    )}
                  </Space>
                )}
                <Space direction="vertical" size={6} style={{ width: '100%' }}>
                  {tree.map((s) => {
                    const tagColor =
                      s.kind === 'mainline' ? 'gold' : s.kind === 'branch' ? 'blue' : 'green';
                    return (
                    <div
                      key={s.id}
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        gap: 8,
                        padding: '4px 8px',
                        border: s.id === currentSessionId ? '1px solid #1677ff' : '1px solid #f0f0f0',
                        borderRadius: 6,
                      }}
                    >
                      <Tag color={tagColor}>
                        {s.isMainline ? 'MAINLINE' : s.kind.toUpperCase()}
                      </Tag>
                      <Text style={{ flex: 1, fontSize: 13 }} ellipsis>
                        {s.preview || s.id.slice(0, 8)}
                      </Text>
                      {s.parent_session && (
                        <Text type="secondary" style={{ fontSize: 11 }}>
                          of {s.parent_session.slice(0, 8)}
                        </Text>
                      )}
                      {!s.isMainline && (
                        <Button size="small" disabled={busy} onClick={() => setMainline(s.id)}>
                          set mainline
                        </Button>
                      )}
                      <Button size="small" type="default" disabled={busy} onClick={() => loadSession(s.id)}>
                        open
                      </Button>
                      <Button
                        size="small"
                        type="dashed"
                        disabled={busy}
                        onClick={() => setBranchModal({ parentId: s.id, title: '' })}
                      >
                        + branch
                      </Button>
                    </div>
                    );
                  })}
                </Space>
              </Card>

              <Title level={5} style={{ marginBottom: 0 }}>
                Coming next (W1d/W2)
              </Title>
              <Space wrap size={6}>
                {['ELF budget card', 'verification badges', 'change timeline', 'scopes & CRs', 'small-model guard'].map(
                  (t) => (
                    <Tag key={t}>{t}</Tag>
                  ),
                )}
              </Space>
            </>
          )}
        </Space>
      </Card>

      <Modal
        title="New branch conversation"
        open={!!branchModal}
        onOk={createBranch}
        onCancel={() => setBranchModal(null)}
        okButtonProps={{ disabled: !branchModal?.title.trim() }}
      >
        <Input
          placeholder="branch title (e.g. sensor drift hunt)"
          value={branchModal?.title ?? ''}
          onChange={(e) =>
            setBranchModal((prev) => (prev ? { ...prev, title: e.target.value } : prev))
          }
          onPressEnter={createBranch}
        />
        <Text type="secondary" style={{ fontSize: 12 }}>
          Fresh context linked to{' '}
          {branchModal?.parentId.slice(0, 8)} — inherits cwd/provider/model only.
        </Text>
      </Modal>
    </div>
  );
}
