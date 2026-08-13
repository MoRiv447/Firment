import { Alert, Button, Card, Input, InputNumber, Space, Typography } from 'antd';
import { PlayCircleOutlined, RocketOutlined } from '@ant-design/icons';
import { useEffect, useState } from 'react';
import { api, onHardwareExit } from '../lib/api';
import type { HardwareExit } from '../types';

const { Text } = Typography;

export function FlashView() {
  const [file, setFile] = useState('');
  const [cwd, setCwd] = useState('');
  const [chip, setChip] = useState('');
  const [probe, setProbe] = useState('');
  const [timeoutSecs, setTimeoutSecs] = useState(60);
  const [busy, setBusy] = useState<'flash' | 'run' | null>(null);
  const [result, setResult] = useState<HardwareExit | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    void onHardwareExit((e) => setResult(e)).then((u) => {
      unlisten = u;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  const doAction = async (kind: 'flash' | 'run') => {
    setResult(null);
    setBusy(kind);
    try {
      const workDir = cwd.trim() || null;
      if (kind === 'flash') {
        await api.flash(file, chip.trim() || null, probe.trim() || null, workDir);
      } else {
        await api.firmRun(file, chip.trim() || null, probe.trim() || null, workDir, timeoutSecs);
      }
    } catch (err) {
      // If Rust emits the hardware-exit event before returning Err, the
      // listener already set a full result (real stdout/stderr). Don't
      // overwrite it with a stripped synthetic entry — fall back only
      // when no emit fired (e.g. spawn failure).
      setResult((prev) =>
        prev
          ? prev
          : {
              kind,
              code: -1,
              stdout: '',
              stderr: String(err),
            },
      );
    } finally {
      setBusy(null);
    }
  };

  return (
    <div style={{ padding: 20, height: '100%', overflowY: 'auto' }}>
      <Card size="small" title="Flash / Run (probe-rs)">
        <Space direction="vertical" size={12} style={{ width: '100%' }}>
          <Input
            placeholder="working dir (firm sandbox root; e.g. D:\...\Debug)"
            style={{ width: '100%' }}
            value={cwd}
            onChange={(e) => setCwd(e.target.value)}
          />
          <Space wrap>
            <Input
              placeholder="path to .elf/.bin (absolute)"
              style={{ width: 420 }}
              value={file}
              onChange={(e) => setFile(e.target.value)}
            />
            <Input
              placeholder="chip (e.g. nrf52840)"
              style={{ width: 160 }}
              value={chip}
              onChange={(e) => setChip(e.target.value)}
            />
            <Input
              placeholder="probe id (optional)"
              style={{ width: 160 }}
              value={probe}
              onChange={(e) => setProbe(e.target.value)}
            />
          </Space>
          <Space>
            <Button
              type="primary"
              icon={<RocketOutlined />}
              loading={busy === 'flash'}
              disabled={!file.trim() || busy !== null}
              onClick={() => doAction('flash')}
            >
              Flash
            </Button>
            <Button
              icon={<PlayCircleOutlined />}
              loading={busy === 'run'}
              disabled={!file.trim() || busy !== null}
              onClick={() => doAction('run')}
            >
              Run
            </Button>
            <InputNumber
              addonBefore="timeout (s)"
              value={timeoutSecs}
              onChange={(v) => setTimeoutSecs(v ?? 60)}
              min={5}
              max={3600}
            />
          </Space>
          <Text type="secondary" style={{ fontSize: 12 }}>
            These buttons invoke the same code path as <Text code>firm flash</Text> /{' '}
            <Text code>firm run</Text>, streaming RTT logs back into the result box below.
          </Text>
          {result && (
            <Alert
              type={result.code === 0 ? 'success' : 'error'}
              showIcon
              message={`${result.kind} exited with code ${result.code}`}
              description={
                <Text style={{ fontSize: 12, whiteSpace: 'pre-wrap' }}>
                  {result.stdout}
                  {result.stderr}
                </Text>
              }
            />
          )}
        </Space>
      </Card>
    </div>
  );
}