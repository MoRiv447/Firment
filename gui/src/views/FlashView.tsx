import { Play, Rocket } from 'lucide-react';
import { useEffect, useState } from 'react';

import { api, onHardwareExit } from '../lib/api';
import type { HardwareExit } from '../types';
import { Button, Card, Field, NumberField, TextInput } from '../ui';
import styles from './FlashView.module.css';

/**
 * Flash and run, off the same code path as the CLI.
 *
 * The result panel is permanent rather than conditional. There used to be a note
 * pointing at "the result box below" while the component rendered nothing at all
 * until a result arrived -- so during a flash you watched a blank pane, which is
 * the one moment you most want to see something.
 */
export function FlashView() {
  const [file, setFile] = useState('');
  const [cwd, setCwd] = useState('');
  const [chip, setChip] = useState('');
  const [probe, setProbe] = useState('');
  const [timeoutSecs, setTimeoutSecs] = useState(60);
  const [busy, setBusy] = useState<'flash' | 'run' | null>(null);
  const [result, setResult] = useState<HardwareExit | null>(null);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void onHardwareExit((e) => setResult(e)).then((u) => {
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
      // If Rust emits the hardware-exit event before returning Err, the listener
      // already set a full result (real stdout/stderr). Don't overwrite it with a
      // stripped synthetic entry -- fall back only when no emit fired (e.g. spawn
      // failure).
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

  const ready = file.trim() !== '' && busy === null;

  return (
    // No padding: this view lives in the inspector, which already pads, and the
    // two added up to a third of the pane's width on a laptop.
    <div className={styles.page}>
      <Card title="Flash / Run (probe-rs)">
        <div className={styles.fields}>
          <TextInput
            placeholder="working dir (firm sandbox root; e.g. D:\…\Debug)"
            value={cwd}
            onChange={(e) => setCwd(e.target.value)}
          />
          {/* A wrapping row rather than fixed widths: a 420px field in a 360px
              column was unreachable, not merely cramped -- the pane scrolls
              vertically only. */}
          <div className={styles.target}>
            <div className={styles.fileField}>
              <TextInput
                mono
                placeholder="path to .elf/.bin (absolute)"
                value={file}
                onChange={(e) => setFile(e.target.value)}
              />
            </div>
            <div className={styles.smallField}>
              <TextInput
                mono
                placeholder="chip (e.g. nrf52840)"
                value={chip}
                onChange={(e) => setChip(e.target.value)}
              />
            </div>
            <div className={styles.smallField}>
              <TextInput
                mono
                placeholder="probe id (optional)"
                value={probe}
                onChange={(e) => setProbe(e.target.value)}
              />
            </div>
          </div>

          <div className={styles.actions}>
            <Button
              tier="primary"
              icon={Rocket}
              disabled={!ready}
              onClick={() => void doAction('flash')}
            >
              {busy === 'flash' ? 'flashing…' : 'Flash'}
            </Button>
            <Button icon={Play} disabled={!ready} onClick={() => void doAction('run')}>
              {busy === 'run' ? 'running…' : 'Run'}
            </Button>
            <div className={styles.timeout}>
              <Field label="timeout (s)">
                <NumberField
                  min={5}
                  max={3600}
                  value={timeoutSecs}
                  onValueChange={(n) => setTimeoutSecs(n ?? 60)}
                />
              </Field>
            </div>
          </div>
        </div>

        <p className={styles.note}>
          These buttons invoke the same code path as <code className={styles.cmd}>firm flash</code> /{' '}
          <code className={styles.cmd}>firm run</code>. The backend collects the run's output and emits it in one piece on
          exit, so it appears below when the run finishes rather than line by line.
        </p>

        <div className={styles.result} data-ok={result && result.code === 0 ? true : undefined}>
          {result ? (
            <>
              <p className={styles.head}>
                {result.kind} exited with code {result.code}
              </p>
              <pre className={styles.out}>
                {result.stdout}
                {result.stderr}
              </pre>
            </>
          ) : (
            <p className={styles.head}>
              {busy
                ? `${busy} in progress…`
                : 'No run yet — the output of the last flash or run lands here.'}
            </p>
          )}
        </div>
      </Card>
    </div>
  );
}
