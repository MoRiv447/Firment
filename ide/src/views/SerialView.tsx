import { Button, Card, Checkbox, Input, InputNumber, Select, Space, Tag, Typography } from 'antd';
import { CaretRightOutlined, SendOutlined, StopOutlined } from '@ant-design/icons';
import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../lib/api';
import type { MonitorLine } from '../types';

const { Text } = Typography;

export function SerialView({ lines }: { lines: Record<string, MonitorLine[]> }) {
  const [ports, setPorts] = useState<string[]>([]);
  const [port, setPort] = useState('COM1');
  const [baud, setBaud] = useState(115200);
  const [elf, setElf] = useState('');
  const [active, setActive] = useState<string[]>([]);
  const [starting, setStarting] = useState(false);
  const [sendText, setSendText] = useState('');
  const [appendCrLf, setAppendCrLf] = useState(true);
  const [sendMsg, setSendMsg] = useState('');
  const scrollRef = useRef<HTMLDivElement>(null);

  const refreshPorts = async () => {
    try {
      const list = await api.listPorts();
      setPorts(list);
      if (list.length > 0 && !list.includes(port)) setPort(list[0]);
    } catch (err) {
      console.error(err);
    }
  };

  useEffect(() => {
    void refreshPorts();
    void api.activeMonitors().then(setActive).catch(console.error);
  }, []);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [lines]);

  const start = async () => {
    setStarting(true);
    try {
      await api.monitorStart(port, baud, elf.trim() || null);
      setActive(await api.activeMonitors());
    } catch (err) {
      setSendMsg(`⚠ failed to start: ${err}`);
      console.error(err);
    } finally {
      setStarting(false);
    }
  };

  const stop = async (p: string) => {
    await api.monitorStop(p);
    setActive(await api.activeMonitors());
  };

  const send = async () => {
    const trimmed = sendText;
    if (!trimmed || !active.includes(port)) {
      setSendMsg('Start the monitor first, then type something to send.');
      return;
    }
    const data = appendCrLf ? `${trimmed}\r\n` : trimmed;
    try {
      await api.monitorSend(port, data);
      setSendMsg(`→ sent ${data.length} bytes`);
    } catch (err) {
      setSendMsg(`⚠ send failed: ${err}`);
      console.error(err);
    }
  };

  const activeText = active.length > 0 ? active.join(', ') : 'none';
  const currentLines = lines[port] ?? [];

  // Render as a continuous stream: chunks that don't end with a newline are
  // glued onto the previous chunk instead of starting a fresh line, exactly
  // like a real serial terminal. stderr chunks keep their own colour.
  const stream = useMemo(() => {
    const blocks: { text: string; stderr: boolean }[] = [];
    let last: { text: string; stderr: boolean } | null = null;
    for (const l of currentLines) {
      const stderr = l.kind === 'stderr';
      const endsWithNl = l.line.endsWith('\n');
      const text = endsWithNl ? l.line.slice(0, -1) : l.line;
      if (last && last.stderr === stderr && !last.text.endsWith('\n')) {
        // continuation of the previous chunk on the same terminal line
        last.text += text;
      } else {
        last = { text, stderr };
        blocks.push(last);
      }
      // keep track of the newline boundary so the next chunk doesn't glue
      // onto a completed line
      if (endsWithNl) last = null;
    }
    return blocks;
  }, [currentLines]);

  return (
    <div style={{ padding: 20, display: 'flex', flexDirection: 'column', height: '100%', gap: 12 }}>
      <Card size="small" title="Serial monitor (read + write UART)">
        <Space wrap>
          <Select
            style={{ width: 150 }}
            value={port}
            onChange={setPort}
            options={ports.map((p) => ({ label: p, value: p }))}
            placeholder="COM port"
          />
          <Button size="small" onClick={refreshPorts}>
            Refresh
          </Button>
          <InputNumber
            value={baud}
            onChange={(v) => setBaud(v ?? 115200)}
            min={1200}
            max={3000000}
          />
          <Input
            placeholder="ELF for symbol decoding (optional)"
            style={{ width: 260 }}
            value={elf}
            onChange={(e) => setElf(e.target.value)}
          />
          {active.includes(port) ? (
            <Button danger icon={<StopOutlined />} onClick={() => stop(port)}>
              Stop on {port}
            </Button>
          ) : (
            <Button
              type="primary"
              icon={<CaretRightOutlined />}
              onClick={start}
              loading={starting}
            >
              Start
            </Button>
          )}
          <Tag>active: {activeText}</Tag>
        </Space>
      </Card>
      <div
        ref={scrollRef}
        style={{
          flex: 1,
          overflow: 'auto',
          background: '#0d0d0d',
          border: '1px solid #303030',
          borderRadius: 8,
          padding: 10,
          fontFamily: 'Consolas, monospace',
          fontSize: 12.5,
          lineHeight: 1.5,
        }}
      >
        {stream.length === 0 && (
          <Text type="secondary">No output yet — start the monitor above.</Text>
        )}
        {stream.map((b, i) => (
          <div key={i} style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>
            {b.stderr ? <span style={{ color: '#e8b339' }}>{b.text}</span> : <span>{b.text}</span>}
          </div>
        ))}
      </div>
      <Card size="small" title="Send to device">
        <Space.Compact style={{ width: '100%' }}>
          <Input
            value={sendText}
            onChange={(e) => setSendText(e.target.value)}
            onPressEnter={send}
            placeholder="Type data to send… (Enter to send)"
            style={{ fontFamily: 'Consolas, monospace' }}
            disabled={!active.includes(port)}
          />
          <Button
            type="primary"
            icon={<SendOutlined />}
            onClick={send}
            disabled={!sendText || !active.includes(port)}
          >
            Send
          </Button>
        </Space.Compact>
        <Space size={12} style={{ marginTop: 8 }}>
          <Checkbox checked={appendCrLf} onChange={(e) => setAppendCrLf(e.target.checked)}>
            append \r\n
          </Checkbox>
          {sendMsg && (
            <Text type="secondary" style={{ fontSize: 12 }}>
              {sendMsg}
            </Text>
          )}
        </Space>
      </Card>
    </div>
  );
}
