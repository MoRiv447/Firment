import {
  Alert,
  Button,
  Card,
  Empty,
  Input,
  List,
  Modal,
  Select,
  Space,
  Statistic,
  Tag,
  Tooltip,
  Typography,
} from 'antd';
import { ReloadOutlined } from '@ant-design/icons';
import { useEffect, useState } from 'react';
import { api, notifySessionsChanged } from '../lib/api';
import type {
  DecisionEntryDto,
  ElfCardDto,
  KbEntryDto,
  PinEntryDto,
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
  // Pin/resource registry ([pinmap] in workbench.toml), shared with the
  // agent's `pinmap` tool — GUI edits and agent claims land on one table.
  const [pinmap, setPinmap] = useState<PinEntryDto[]>([]);
  const [newPin, setNewPin] = useState('');
  const [newFunc, setNewFunc] = useState('');
  // ADR-lite decision log ([[decision]]); branches whose title matches a
  // decision inherit it automatically at creation.
  const [decisions, setDecisions] = useState<DecisionEntryDto[]>([]);
  const [newTitle, setNewTitle] = useState('');
  const [newBody, setNewBody] = useState('');
  // Project knowledge files (AGENTS.md / vendor index / private cheatsheets).
  const [kbFiles, setKbFiles] = useState<KbEntryDto[]>([]);
  const [kbKey, setKbKey] = useState<string | null>(null);
  const [kbDraft, setKbDraft] = useState('');
  const [kbDirty, setKbDirty] = useState(false);
  const [newCheatName, setNewCheatName] = useState('');

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
    try {
      setPinmap(await api.workbenchPinmapList(cwd.trim()));
    } catch {
      setPinmap([]);
    }
    try {
      setDecisions(await api.workbenchDecisionList(cwd.trim()));
    } catch {
      setDecisions([]);
    }
    try {
      const files = await api.workbenchKbList(cwd.trim());
      setKbFiles(files);
      // Keep the current selection if it still exists; else default to
      // AGENTS.md so the editor is never stuck on a deleted file.
      setKbKey((prev) => {
        if (prev && files.some((f) => f.key === prev)) return prev;
        return files[0]?.key ?? null;
      });
    } catch {
      setKbFiles([]);
    }
    if (wb?.config.mainline_session) {
      await refreshInsights(cwd.trim(), wb.config.mainline_session);
    } else {
      setElf(null);
      setElfError(null);
      setQuality([]);
      setTimeline([]);
    }
    // Manual refresh heals the sidebar too (e.g. sessions changed on disk
    // while the app was open).
    notifySessionsChanged();
    setBusy(false);
  };

  const addPin = async () => {
    if (!cwd.trim() || !newPin.trim() || !newFunc.trim()) return;
    setBusy(true);
    try {
      setPinmap(await api.workbenchPinmapSet(cwd.trim(), newPin, newFunc, 'user'));
      setNewPin('');
      setNewFunc('');
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const removePin = async (pin: string) => {
    if (!cwd.trim()) return;
    setBusy(true);
    try {
      setPinmap(await api.workbenchPinmapRemove(cwd.trim(), pin));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const addDecision = async () => {
    if (!cwd.trim() || !newTitle.trim()) return;
    setBusy(true);
    try {
      setDecisions(await api.workbenchDecisionAdd(cwd.trim(), newTitle, newBody));
      setNewTitle('');
      setNewBody('');
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const removeDecision = async (index: number) => {
    if (!cwd.trim()) return;
    setBusy(true);
    try {
      // Card renders decisions in list order; backend expects the same
      // 1-based indexing.
      setDecisions(await api.workbenchDecisionRemove(cwd.trim(), index + 1));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const selectKbFile = (key: string) => {
    const f = kbFiles.find((x) => x.key === key);
    setKbKey(key);
    setKbDraft(f?.content ?? '');
    setKbDirty(false);
  };

  const saveKbFile = async () => {
    if (!cwd.trim() || !kbKey || !kbDirty) return;
    setBusy(true);
    try {
      await api.workbenchKbSave(cwd.trim(), kbKey, kbDraft);
      setKbFiles((prev) =>
        prev.map((f) => (f.key === kbKey ? { ...f, content: kbDraft, exists: true } : f)),
      );
      setKbDirty(false);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const deleteKbFile = async () => {
    if (!cwd.trim() || !kbKey?.startsWith('cheatsheet:')) return;
    setBusy(true);
    try {
      await api.workbenchKbDelete(cwd.trim(), kbKey);
      const files = await api.workbenchKbList(cwd.trim());
      setKbFiles(files);
      setKbKey(files[0]?.key ?? null);
      setKbDraft(files[0]?.content ?? '');
      setKbDirty(false);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const newCheatsheet = async () => {
    if (!cwd.trim()) return;
    let name = newCheatName.trim();
    if (!name) return;
    if (!name.endsWith('.toml')) name += '.toml';
    const key = `cheatsheet:${name}`;
    setBusy(true);
    try {
      // Create empty, reload list, and jump straight into editing it.
      await api.workbenchKbSave(cwd.trim(), key, '# project cheatsheet\n');
      const files = await api.workbenchKbList(cwd.trim());
      setKbFiles(files);
      setKbKey(key);
      setKbDraft(files.find((f) => f.key === key)?.content ?? '');
      setKbDirty(false);
      setNewCheatName('');
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const createBranch = async () => {
    if (!branchModal) return;
    setBusy(true);
    try {
      const id = await api.workbenchBranchCreate(branchModal.parentId, branchModal.title);
      setBranchModal(null);
      await refresh(cwd.trim());
      setCurrentSessionId(id);
      // The sidebar owns its own session list: tell it a branch was added.
      notifySessionsChanged();
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
      // Promotion demotes the old mainline too — both rows change tag.
      notifySessionsChanged();
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
      // Without this the sidebar still shows the fresh session as NORMAL.
      notifySessionsChanged();
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
                title="Pin assignments"
                size="small"
                extra={
                  <Text type="secondary" style={{ fontSize: 11 }}>
                    shared with the agent's pinmap tool
                  </Text>
                }
              >
                {pinmap.length === 0 && (
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    No pins claimed yet. The agent registers allocations here before wiring
                    peripherals; add known ones manually below.
                  </Text>
                )}
                {pinmap.length > 0 && (
                  <div style={{ marginBottom: 8 }}>
                    {pinmap.map((p) => (
                      <div
                        key={p.pin}
                        style={{
                          display: 'flex',
                          alignItems: 'center',
                          gap: 8,
                          padding: '3px 6px',
                          borderBottom: '1px solid #f0f0f0',
                        }}
                      >
                        <Tag color="blue" style={{ borderRadius: 0, fontWeight: 700, minWidth: 64, textAlign: 'center' }}>
                          {p.pin}
                        </Tag>
                        <Text style={{ flex: 1, fontSize: 12 }}>{p.func}</Text>
                        <Text type="secondary" style={{ fontSize: 11 }}>
                          {p.owner || '—'}
                        </Text>
                        <Button size="small" type="text" danger disabled={busy} onClick={() => removePin(p.pin)}>
                          ✕
                        </Button>
                      </div>
                    ))}
                  </div>
                )}
                <Space.Compact style={{ width: '100%', marginTop: 4 }}>
                  <Input
                    size="small"
                    placeholder="pin (PA5)"
                    value={newPin}
                    onChange={(e) => setNewPin(e.target.value)}
                    onPressEnter={addPin}
                    style={{ maxWidth: 110, fontFamily: 'Consolas, monospace' }}
                  />
                  <Input
                    size="small"
                    placeholder="function (LED / USART1_TX…)"
                    value={newFunc}
                    onChange={(e) => setNewFunc(e.target.value)}
                    onPressEnter={addPin}
                  />
                  <Button size="small" type="dashed" disabled={busy} onClick={addPin}>
                    claim
                  </Button>
                </Space.Compact>
              </Card>

              <Card
                type="inner"
                title="Decisions (ADR-lite)"
                size="small"
                extra={
                  <Tooltip title="Branches whose title matches a decision automatically inherit it at creation">
                    <Text type="secondary" style={{ fontSize: 11 }}>
                      inherited by matching branches
                    </Text>
                  </Tooltip>
                }
              >
                {decisions.length === 0 && (
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    No decisions recorded. Log chip/peripheral/protocol choices here — the agent's
                    decision tool writes the same list.
                  </Text>
                )}
                {decisions.map((d, i) => (
                  <div
                    key={`${d.date}-${i}`}
                    style={{
                      display: 'flex',
                      alignItems: 'flex-start',
                      gap: 8,
                      padding: '4px 6px',
                      borderBottom: '1px solid #f0f0f0',
                    }}
                  >
                    <Tag style={{ borderRadius: 0, fontSize: 10, minWidth: 76, textAlign: 'center' }}>
                      {d.date || '—'}
                    </Tag>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <Text style={{ fontSize: 12, fontWeight: 600 }}>{d.title}</Text>
                      {d.body && (
                        <div>
                          <Text type="secondary" style={{ fontSize: 11 }}>
                            {d.body}
                          </Text>
                        </div>
                      )}
                    </div>
                    <Button size="small" type="text" danger disabled={busy} onClick={() => removeDecision(i)}>
                      ✕
                    </Button>
                  </div>
                ))}
                <Space.Compact style={{ width: '100%', marginTop: 8 }}>
                  <Input
                    size="small"
                    placeholder="decision headline (I2C bus at 400k)"
                    value={newTitle}
                    onChange={(e) => setNewTitle(e.target.value)}
                    style={{ maxWidth: 260 }}
                  />
                  <Input
                    size="small"
                    placeholder="rationale / constraints (optional)"
                    value={newBody}
                    onChange={(e) => setNewBody(e.target.value)}
                    onPressEnter={addDecision}
                  />
                  <Button size="small" type="dashed" disabled={busy || !newTitle.trim()} onClick={addDecision}>
                    record
                  </Button>
                </Space.Compact>
              </Card>

              <Card
                type="inner"
                title="Project knowledge"
                size="small"
                extra={
                  <Space size={4}>
                    <Input
                      size="small"
                      placeholder="new-cheatsheet.toml"
                      value={newCheatName}
                      onChange={(e) => setNewCheatName(e.target.value)}
                      onPressEnter={newCheatsheet}
                      style={{ width: 150, fontFamily: 'Consolas, monospace', fontSize: 11 }}
                    />
                    <Button size="small" type="dashed" disabled={busy || !newCheatName.trim()} onClick={newCheatsheet}>
                      +
                    </Button>
                  </Space>
                }
              >
                {kbFiles.length === 0 ? (
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    No knowledge files yet. AGENTS.md is injected into every session's system
                    prompt; docs/vendor-index.toml is the hardware knowledge index; cheatsheets
                    live under .firment/cheatsheets/.
                  </Text>
                ) : (
                  <>
                    <Select
                      size="small"
                      style={{ width: '100%', marginBottom: 8, fontFamily: 'Consolas, monospace' }}
                      value={kbKey ?? undefined}
                      onChange={selectKbFile}
                      options={kbFiles.map((f) => ({
                        value: f.key,
                        label: `${f.key}${f.exists ? '' : ' (new)'}`,
                      }))}
                    />
                    <Input.TextArea
                      value={kbDraft}
                      onChange={(e) => {
                        setKbDraft(e.target.value);
                        setKbDirty(true);
                      }}
                      rows={10}
                      styles={{ textarea: { fontFamily: 'Consolas, monospace', fontSize: 12 } }}
                      placeholder={
                        kbKey === 'AGENTS.md'
                          ? 'Project memory for every session: coding rules, hardware notes, gotchas…'
                          : undefined
                      }
                    />
                    <Space style={{ marginTop: 6 }}>
                      <Button
                        size="small"
                        type="primary"
                        disabled={busy || !kbDirty}
                        onClick={saveKbFile}
                      >
                        save{kbDirty ? ' •' : ''}
                      </Button>
                      {kbKey?.startsWith('cheatsheet:') && (
                        <Button size="small" danger disabled={busy} onClick={deleteKbFile}>
                          delete cheatsheet
                        </Button>
                      )}
                    </Space>
                  </>
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
                    <Tooltip title="Reload sessions from disk">
                      <Button size="small" type="text" loading={busy} onClick={load} icon={<ReloadOutlined />} />
                    </Tooltip>
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
