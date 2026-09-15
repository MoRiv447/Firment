import { Play, RefreshCw, Send, Square } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';

import { api, onMonitorExited } from '../lib/api';
import type { MonitorLine } from '../types';
import { Button, Card, Checkbox, Chip, Field, NumberField, Select, TextInput } from '../ui';
import { toStream } from './serial/stream';
import styles from './SerialView.module.css';

/**
 * The serial monitor: open a port, watch it, type into it.
 *
 * The layout is deliberate about one thing -- the controls wrap and can shrink.
 * They used to be a `Space`, which cannot shrink a 260px field, so on the narrow
 * inspector pane the ELF box and the Start button were cut off with no way to
 * scroll to them. The pane is 360px at its roomiest.
 */
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

  // A monitor can exit on its own (port unplugged, reader error). Refresh the
  // active list so Stop/Send buttons don't keep claiming the port is live.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void onMonitorExited(() => {
      void api.activeMonitors().then(setActive).catch(console.error);
    }).then((u) => {
      // Unmounted before the listen() IPC resolved → drop it immediately,
      // otherwise this view leaks one listener per fast tab switch.
      if (cancelled) u();
      else unlisten = u;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
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
    if (!sendText || !open) {
      setSendMsg('Start the monitor first, then type something to send.');
      return;
    }
    const data = appendCrLf ? `${sendText}\r\n` : sendText;
    try {
      await api.monitorSend(port, data);
      setSendMsg(`→ sent ${data.length} bytes`);
    } catch (err) {
      setSendMsg(`⚠ send failed: ${err}`);
      console.error(err);
    }
  };

  const open = active.includes(port);
  const activeText = active.length > 0 ? active.join(', ') : 'none';
  const currentLines = lines[port] ?? [];
  const stream = useMemo(() => toStream(currentLines), [currentLines]);

  return (
    <div className={styles.page}>
      <Card title="Serial monitor (read + write UART)">
        <div className={styles.controls}>
          <div className={styles.portField}>
            <Select
              ariaLabel="Port"
              value={port}
              onChange={setPort}
              options={ports.map((p) => ({ label: p, value: p }))}
              placeholder="COM port"
            />
          </div>
          <Button size="sm" icon={RefreshCw} onClick={() => void refreshPorts()}>
            Refresh
          </Button>
          <div className={styles.baudField}>
            <Field label="Baud">
              <NumberField
                min={1200}
                max={3000000}
                value={baud}
                onValueChange={(n) => setBaud(n ?? 115200)}
              />
            </Field>
          </div>
          <div className={styles.elfField}>
            <Field label="ELF for symbol decoding (optional)">
              <TextInput
                mono
                value={elf}
                onChange={(e) => setElf(e.target.value)}
                placeholder="firmware.elf"
              />
            </Field>
          </div>
          {open ? (
            <Button tier="danger" icon={Square} onClick={() => void stop(port)}>
              Stop on {port}
            </Button>
          ) : (
            <Button tier="primary" icon={Play} disabled={starting} onClick={() => void start()}>
              Start
            </Button>
          )}
          <Chip size="sm" mono>
            active: {activeText}
          </Chip>
        </div>
      </Card>

      <div ref={scrollRef} className={styles.output}>
        {stream.length === 0 && <p className={styles.waiting}>No output yet — start the monitor above.</p>}
        {stream.map((block, i) => (
          <div key={i} className={styles.block}>
            {block.stderr ? <span className={styles.stderr}>{block.text}</span> : <span>{block.text}</span>}
          </div>
        ))}
      </div>

      <Card title="Send to device">
        <div className={styles.sendRow}>
          <TextInput
            mono
            value={sendText}
            onChange={(e) => setSendText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void send();
            }}
            placeholder="Type data to send… (Enter to send)"
            disabled={!open}
          />
          <Button
            tier="primary"
            icon={Send}
            disabled={!sendText || !open}
            onClick={() => void send()}
          >
            Send
          </Button>
        </div>
        <div className={styles.sendFoot}>
          <Checkbox checked={appendCrLf} onChange={setAppendCrLf} label={'append \\r\\n'} />
          {sendMsg && <p className={styles.sendMsg}>{sendMsg}</p>}
        </div>
      </Card>
    </div>
  );
}
