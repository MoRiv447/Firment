import {
  Alert,
  Button,
  Card,
  Empty,
  Input,
  Modal,
  Select,
  Space,
  Tag,
  Tooltip,
  Typography,
} from 'antd';
import { ReloadOutlined } from '@ant-design/icons';
import { useEffect, useRef, useState } from 'react';
import { api, notifySessionsChanged, onWorkbenchOpen } from '../lib/api';
import { isUnder, pathKey } from '../lib/paths';
import type {
  BoardPinmapDto,
  DecisionEntryDto,
  DeviceBindingDto,
  ElfCardDto,
  EscalationEntry,
  FlashHistoryDto,
  HardwareInfoDto,
  KbEntryDto,
  QualityItemDto,
  SessionSummaryDto,
  TimelineEntryDto,
  WorkbenchStateDto,
} from '../types';
import { color, font, radius, statusChip } from '../styles/tokens';
import type { StatusKind } from '../styles/tokens';
import { ActionButton } from '../components/ActionButton';
import { FlashHistory } from './workbench/FlashHistory';
import { ChangeTimeline, ElfBudget, VerificationBadges } from './workbench/insights';
import { Decisions } from './workbench/Decisions';
import { Bindings } from './workbench/Bindings';
import { Hardware } from './workbench/Hardware';
import { Pinmap } from './workbench/Pinmap';
import { Escalations } from './workbench/Escalations';
import { ProjectBar } from './workbench/ProjectBar';
import { ProjectSummary } from './workbench/ProjectSummary';
import { TrafficPane } from './workbench/TrafficPane';
import { alertFromFrame, foldEscalation } from './workbench/guard';
import { useDeviceTraffic } from './workbench/useDeviceTraffic';

const { Text } = Typography;

/**
 * Project workbench (W1): mainline + branch session tree over
 * .firment/workbench.toml, quick repo status, pins, devices, hardware, flash
 * history and the W1d insight cards.
 *
 * Still missing, named here rather than left as a row of "Coming next" tags that
 * outlived what they described: the ChangeRequest flow (the branch model carries
 * `status: open | merged | archived` and nothing sets it), session search and
 * archive, and a GUI table for the guard rules that live only in
 * `sbc-guard/rules.toml`. docs/gui-workbench.md lists exactly these.
 */
export function WorkbenchView() {
  // Device traffic (the unfiltered stream + the guard link) is subscribed by
  // `useDeviceTraffic` below rather than up here: the alert callback has to
  // close over `runEscalation`, which is defined after the loaders it calls.
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
  // Pin/resource registry ([pinmap.<board>] in workbench.toml), shared with
  // the agent's `pinmap` tool — GUI edits and agent claims land on one
  // table, scoped per board (board name == MQTT node name).
  const [pinmap, setPinmap] = useState<BoardPinmapDto[]>([]);
  const [pinBoard, setPinBoard] = useState<string | null>(null);
  // Per-project device bindings ([devices.<node>] in workbench.toml).
  const [bindings, setBindings] = useState<DeviceBindingDto[]>([]);
  // Hardware inventory: serial ports + probe-rs probes + default chip.
  // Refreshed behind an explicit button (probe-rs enumeration takes ~1s).
  const [hardware, setHardware] = useState<HardwareInfoDto | null>(null);
  // Burn history (.firment/work/flash-history.jsonl) — 何日向哪块板烧了哪个镜像。
  const [flashHistory, setFlashHistory] = useState<FlashHistoryDto[]>([]);
  // Guard escalations: alerts at/above the project's escalate_sev for BOUND
  // nodes. Persisted per project so switching tabs doesn't drop them.
  const [escalations, setEscalations] = useState<EscalationEntry[]>([]);
  const [autoRun, setAutoRun] = useState(
    () => localStorage.getItem('escalation-auto-run') === '1',
  );
  // The pending list is mirrored in a ref as well as in state: two alerts can
  // land inside one React batch, and the second fold has to see the first.
  const escalRef = useRef<EscalationEntry[]>([]);

  /** The one way the pending list changes, so the per-project copy in
   * localStorage cannot drift from the list on screen. */
  const setPending = (next: EscalationEntry[]) => {
    escalRef.current = next;
    setEscalations(next);
    try {
      localStorage.setItem(`guard-pending-${cwd.trim()}`, JSON.stringify(next));
    } catch {
      /* storage full — the pending list just won't survive a restart */
    }
  };
  // ADR-lite decision log ([[decision]]); branches whose title matches a
  // decision inherit it automatically at creation.
  const [decisions, setDecisions] = useState<DecisionEntryDto[]>([]);
  // Project knowledge files (AGENTS.md / vendor index / private cheatsheets).
  const [kbFiles, setKbFiles] = useState<KbEntryDto[]>([]);
  const [kbKey, setKbKey] = useState<string | null>(null);
  const [kbDraft, setKbDraft] = useState('');
  const [kbDirty, setKbDirty] = useState(false);
  const [newCheatName, setNewCheatName] = useState('');
  // Save baselines (disk mtime per key, captured when the draft was loaded):
  // passed back to workbench_kb_save so an external edit (agent, other
  // editor) is refused instead of silently overwritten. undefined = the file
  // was never opened in the editor.
  const kbBaselineRef = useRef<Record<string, number | null>>({});

  const rememberProject = (dir: string) => {
    localStorage.setItem('workbench-last-cwd', dir);
    setProjects((prev) => {
      const next = [dir, ...prev.filter((p) => pathKey(p) !== pathKey(dir))];
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
      // Only show sessions belonging to this project root. `isUnder` respects
      // the directory boundary: without it D:\fw\thermo also swallows every
      // session under D:\fw\thermostat.
      setSessions(all.filter((s) => isUnder(dir, s.cwd)));
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
      // load() (not refresh()): restores the CARDS too — bindings, pinmap,
      // decisions, knowledge files and the persisted escalation list.
      void load(last);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The sidebar's folder button opens a DIFFERENT project. This view stays
  // mounted across tabs (its startup restore ran long ago), so App bridges
  // the request here instead of relying on localStorage alone. The []-deps
  // listener reaches the latest load() through a ref (stale-closure-proof,
  // same pattern as runEscalationRef below).
  const reloadRef = useRef<(dir: string) => void>(() => {});
  useEffect(() => onWorkbenchOpen((dir) => reloadRef.current(dir)), []);
  reloadRef.current = (dir: string) => {
    setCwd(dir);
    void load(dir);
  };

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

  /** dir override: the startup-restore path passes the persisted path
   * directly because the `cwd` state it just set is not visible yet. */
  /**
   * Enumeration and the global chip write stay here, not in Hardware: the
   * payload, the error slot and the busy flag all belong to this component.
   */
  const refreshHardware = async () => {
    if (!cwd.trim()) return;
    setBusy(true);
    try {
      setHardware(await api.workbenchHardwareList(cwd.trim()));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const saveChip = async (chip: string) => {
    setBusy(true);
    try {
      const saved = await api.setDefaultChip(chip);
      setHardware((prev) => (prev ? { ...prev, default_chip: saved } : prev));
      return true;
    } catch (err) {
      setError(String(err));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const load = async (dir?: string) => {
    const target = (dir ?? cwd).trim();
    if (!target) return;
    setBusy(true);
    const wb = await refresh(target);
    try {
      const boards = await api.workbenchPinmapList(target);
      setPinmap(boards);
      setPinBoard((prev) => {
        if (prev && boards.some((b) => b.board === prev)) return prev;
        return boards[0]?.board ?? null;
      });
    } catch {
      setPinmap([]);
    }
    try {
      setDecisions(await api.workbenchDecisionList(target));
    } catch {
      setDecisions([]);
    }
    try {
      setBindings(await api.workbenchDevicesList(target));
    } catch {
      setBindings([]);
    }
    try {
      setHardware(await api.workbenchHardwareList(target));
    } catch {
      setHardware(null);
    }
    try {
      setFlashHistory(await api.workbenchFlashHistory(target, 20));
    } catch {
      setFlashHistory([]);
    }
    try {
      const saved = JSON.parse(
        localStorage.getItem(`guard-pending-${target}`) || '[]',
      ) as EscalationEntry[];
      setEscalations(saved);
      escalRef.current = saved;
    } catch {
      setEscalations([]);
    }
    try {
      const files = await api.workbenchKbList(target);
      setKbFiles(files);
      // Seed baselines only for keys never opened in the editor: an existing
      // baseline belongs to the loaded DRAFT and must keep exposing disk
      // changes made after it was loaded.
      for (const f of files) {
        if (kbBaselineRef.current[f.key] === undefined) {
          kbBaselineRef.current[f.key] = f.mtimeMs;
        }
      }
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
      await refreshInsights(target, wb.config.mainline_session);
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

  const claimPin = async (pin: string, func: string) => {
    if (!cwd.trim() || !pinBoard) return false;
    setBusy(true);
    try {
      setPinmap(await api.workbenchPinmapSet(cwd.trim(), pinBoard, pin, func, 'user'));
      return true;
    } catch (err) {
      setError(String(err));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const removePin = async (pin: string) => {
    if (!cwd.trim() || !pinBoard) return;
    setBusy(true);
    try {
      setPinmap(await api.workbenchPinmapRemove(cwd.trim(), pinBoard, pin));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };


  const bindDevice = async (node: string, role: string): Promise<boolean> => {
    if (!cwd.trim() || !node.trim()) return false;
    setBusy(true);
    try {
      // note/allow omitted → backend PRESERVES existing values (an allow
      // whitelist is never wiped by a role-only rebind).
      setBindings(await api.workbenchDevicesSet(cwd.trim(), node.trim(), role.trim()));
      return true;
    } catch (err) {
      setError(String(err));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const unbindDevice = async (node: string) => {
    if (!cwd.trim()) return;
    setBusy(true);
    try {
      setBindings(await api.workbenchDevicesRemove(cwd.trim(), node));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const dropEscalation = (id: string) => {
    setPending(escalRef.current.filter((x) => x.id !== id));
  };

  /** Hand an escalation to the project's mainline session: synthesized
   * diagnosis prompt, turn started via the App-level event listener. */
  const runEscalation = (entry: EscalationEntry) => {
    const mainline = state?.config.mainline_session;
    if (!mainline) return;
    const prompt = [
      `[guard escalation] node ${entry.node} sev=${entry.sev} rule=${entry.rule}`,
      `summary: ${entry.summary}`,
      `payload: ${entry.payload}`,
      'Diagnose this device alert: start with device_log to find the root cause in recent frames.',
      'Use device_cmd if the device must be touched, and say why. End with a conclusion and next steps.',
    ].join('\n');
    window.dispatchEvent(
      new CustomEvent('firment:run-escalation', {
        detail: { cwd: cwd.trim(), sessionId: mainline, prompt },
      }),
    );
    dropEscalation(entry.id);
  };

  // The alert half of the stream is decided here and not in the hook: what
  // counts as an escalation belongs to the project, not to the transport.
  // `||` and not `??`: an empty threshold is an unset one, and a level the rank
  // table does not know would score as `info` while the card printed
  // "sev ≥ warn".
  const escalateSev = state?.config.guard_escalate_sev || 'warn';
  const traffic = useDeviceTraffic((frame, node, ts) => {
    const alert = alertFromFrame(frame, node, ts);
    const fold = foldEscalation(escalRef.current, alert, {
      threshold: escalateSev,
      bound: bindings.some((b) => b.node === alert.node),
    });
    if (fold.kind === 'none') return;
    setPending(fold.entries);
    // Same path as the manual button: it builds the mainline prompt and
    // dispatches the correctly-shaped event.
    if (fold.kind === 'escalated' && autoRun) runEscalation(fold.entry);
  });

  const toggleAutoRun = (on: boolean) => {
    setAutoRun(on);
    localStorage.setItem('escalation-auto-run', on ? '1' : '0');
  };

  const addDecision = async (title: string, body: string): Promise<boolean> => {
    if (!cwd.trim() || !title.trim()) return false;
    setBusy(true);
    try {
      setDecisions(await api.workbenchDecisionAdd(cwd.trim(), title, body));
      return true;
    } catch (err) {
      setError(String(err));
      return false;
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
    // Loading the draft freezes the baseline: any disk change from this
    // moment on is a conflict the save must surface.
    kbBaselineRef.current[key] = f?.mtimeMs ?? null;
    setKbDirty(false);
  };

  const saveKbFile = async () => {
    if (!cwd.trim() || !kbKey || !kbDirty) return;
    setBusy(true);
    try {
      await api.workbenchKbSave(
        cwd.trim(),
        kbKey,
        kbDraft,
        kbBaselineRef.current[kbKey] ?? null,
      );
      const files = await api.workbenchKbList(cwd.trim());
      setKbFiles(files);
      kbBaselineRef.current[kbKey] =
        files.find((f) => f.key === kbKey)?.mtimeMs ?? null;
      setKbDirty(false);
    } catch (err) {
      const msg = String(err);
      if (msg.includes('[ConcurrentChange]')) {
        // The file changed on disk while the draft was open. Offer a reload
        // instead of letting the user fight a silent last-writer-wins.
        Modal.confirm({
          title: 'The file changed on disk',
          content: 'The knowledge file was modified by the agent or another program while you were editing it. Discard your draft and reload from disk?',
          okText: 'Reload',
          cancelText: 'Keep draft',
          onOk: () => {
            if (kbKey) selectKbFile(kbKey);
          },
        });
      } else {
        setError(msg);
      }
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
      // Create empty, reload list, and jump straight into editing it. The
      // fresh-create baseline (0) refuses when the file appeared meanwhile.
      await api.workbenchKbSave(cwd.trim(), key, '# project cheatsheet\n', 0);
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
        <TrafficPane traffic={traffic} />
        <Space direction="vertical" size={12} style={{ width: '100%' }}>
          <ProjectBar
            cwd={cwd}
            projects={projects}
            busy={busy}
            error={error}
            onCwd={setCwd}
            onOpen={() => void load()}
            onPick={(dir) => {
              setCwd(dir);
              void load(dir);
            }}
          />

          {state && (
            <>
              <ProjectSummary state={state} />

              <Bindings
                bindings={bindings}
                devices={traffic.devices}
                busy={busy}
                onBind={bindDevice}
                onUnbind={unbindDevice}
              />

              <Escalations
                entries={escalations}
                threshold={escalateSev}
                mainline={state.config.mainline_session}
                busy={busy}
                autoRun={autoRun}
                onAutoRun={toggleAutoRun}
                onDiagnose={runEscalation}
                onDismiss={dropEscalation}
              />

              <Hardware
                hardware={hardware}
                busy={busy}
                onRefresh={refreshHardware}
                onSaveChip={saveChip}
              />

               <FlashHistory history={flashHistory} />

              <Pinmap
                boards={pinmap}
                selected={pinBoard}
                busy={busy}
                onSelectBoard={setPinBoard}
                onClaimPin={claimPin}
                onRemovePin={removePin}
              />

              <Decisions
                decisions={decisions}
                busy={busy}
                onAdd={addDecision}
                onRemove={removeDecision}
              />

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
                      style={{ width: 150, fontFamily: font.mono, fontSize: 11 }}
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
                      style={{ width: '100%', marginBottom: 8, fontFamily: font.mono }}
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
                      styles={{ textarea: { fontFamily: font.mono, fontSize: 12 } }}
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
                {elf && <ElfBudget elf={elf} />}
                {quality.length > 0 && <VerificationBadges quality={quality} />}
                {timeline.length > 0 && <ChangeTimeline timeline={timeline} />}
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
                      <Button size="small" type="text" loading={busy} onClick={() => void load()} icon={<ReloadOutlined />} />
                    </Tooltip>
                  </Space>
                }
              >
                {tree.length === 0 && (
                  <Space direction="vertical" size={8} style={{ width: '100%' }}>
                    <Empty description="No sessions under this path yet" image={Empty.PRESENTED_IMAGE_SIMPLE} />
                    {kindFilter === 'all' && (
                      <ActionButton tier="primary" loading={busy} onClick={createMainline}>
                        New mainline chat here
                      </ActionButton>
                    )}
                  </Space>
                )}
                <Space direction="vertical" size={6} style={{ width: '100%' }}>
                  {tree.map((s) => {
                    // Mainline / branch / plain session. A mainline is an
                    // emphasis, not a warning, so it gets `attention`'s pair
                    // rather than a gold preset.
                    const kindStatus: StatusKind =
                      s.kind === 'mainline' ? 'attention' : s.kind === 'branch' ? 'running' : 'ok';
                    return (
                    <div
                      key={s.id}
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        gap: 8,
                        padding: '4px 8px',
                        // `selection`, not the acid: a 1px acid border on a
                        // light ground is 1.27:1, so the selected card and its
                        // neighbours were the same colour.
                        border: `1px solid ${s.id === currentSessionId ? color.selection : color.line}`,
                        borderRadius: radius.control,
                      }}
                    >
                      <Tag style={{ ...statusChip(kindStatus), borderRadius: radius.chip }}>
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
