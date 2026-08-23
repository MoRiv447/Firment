import {
  Alert,
  Button,
  Card,
  Empty,
  Input,
  Modal,
  Space,
  Statistic,
  Tag,
  Typography,
} from 'antd';
import { useEffect, useState } from 'react';
import { api } from '../lib/api';
import type { SessionSummaryDto, WorkbenchStateDto } from '../types';

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

  const refresh = async (dir: string) => {    setError(null);
    try {
      const [wb, sessions] = await Promise.all([
        api.workbenchState(dir),
        api.listSessions(),
      ]);
      setState(wb);
      // Only show sessions belonging to this project root.
      const root = dir.replace(/\\/g, '/').replace(/\/$/, '').toLowerCase();
      setSessions(
        sessions.filter((s) => s.cwd.replace(/\\/g, '/').toLowerCase().startsWith(root)),
      );
      setCurrentSessionId(null);
    } catch (err) {
      setError(String(err));
    }
  };

  // v1: the project path is entered manually; W2 will remember the last
  // opened project per machine.
  useEffect(() => {}, []);

  const load = async () => {
    if (!cwd.trim()) return;
    setBusy(true);
    await refresh(cwd.trim());
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
      await refresh(state.root);
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

  const tree = sessions.map((s) => ({
    ...s,
    isMainline: state?.config.mainline_session === s.id,
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

              <Card type="inner" title="Session tree" size="small">
                {tree.length === 0 && (
                  <Space direction="vertical" size={8} style={{ width: '100%' }}>
                    <Empty description="No sessions under this path yet" image={Empty.PRESENTED_IMAGE_SIMPLE} />
                    <Button type="primary" loading={busy} onClick={createMainline}>
                      New mainline chat here
                    </Button>
                  </Space>
                )}
                <Space direction="vertical" size={6} style={{ width: '100%' }}>
                  {tree.map((s) => (
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
                      <Tag color={s.kind === 'main' ? 'gold' : 'blue'}>
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
                  ))}
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
