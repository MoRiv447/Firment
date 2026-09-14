import { pathKey } from '../../lib/paths';
import { Button, Callout, TextInput } from '../../ui';
import styles from './ProjectBar.module.css';

/**
 * Which project this workbench is showing, and how to change it.
 *
 * The path is a text field rather than only a picker because the useful thing is
 * to type the three letters of a folder you keep typing. The recent list drops the
 * project that is already open: it is the one string on screen that cannot be
 * mistaken for another, and a row that repeats it is a row that asks "which of
 * these is current?" without answering.
 */
export function ProjectBar({
  cwd,
  projects,
  busy,
  error,
  onCwd,
  onOpen,
  onPick,
}: {
  cwd: string;
  projects: string[];
  /** A load is in flight; the button spins and the list is still clickable. */
  busy: boolean;
  error: string | null;
  onCwd: (value: string) => void;
  onOpen: () => void;
  onPick: (dir: string) => void;
}) {
  const open = cwd.trim();
  const known = open ? projects.filter((p) => pathKey(p) !== pathKey(open)) : projects;

  return (
    <div className={styles.bar}>
      <div className={styles.pick}>
        <TextInput
          mono
          size="lg"
          placeholder="project path (e.g. D:\fw\thermostat)"
          value={cwd}
          onChange={(e) => onCwd(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') onOpen();
          }}
        />
        <Button tier="primary" size="lg" loading={busy} onClick={onOpen}>
          Open project
        </Button>
      </div>
      {known.length > 0 && (
        <div className={styles.known}>
          <span className={styles.label}>known projects:</span>
          {known.map((p) => (
            <Button
              key={p}
              tier="ghost"
              size="sm"
              className={styles.path}
              title={p}
              onClick={() => onPick(p)}
            >
              {p}
            </Button>
          ))}
        </div>
      )}
      {error && (
        <Callout tone="failed" mono>
          {error}
        </Callout>
      )}
    </div>
  );
}
