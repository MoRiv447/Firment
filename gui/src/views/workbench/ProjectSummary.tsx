import type { WorkbenchStateDto } from '../../types';
import { Card, KeyValue } from '../../ui';
import styles from './ProjectSummary.module.css';

/**
 * What the folder on disk actually is: a git state, a mainline, and the file the
 * workbench was read from.
 *
 * The values are stacked in `KeyValue` rather than laid out across the card. The
 * old version of this used antd's `Statistic`, which sets its value at display
 * size -- 24px -- and that is right for a count and wrong for everything else
 * that was in here: a branch name, a session id and the words "not a git
 * repository" were all being set as if they were headline numbers, each one a tiny
 * grey label floating over a huge grey string.
 *
 * `root` is the path the state was read from, which is not always what the field
 * above currently says -- typing a new path does not move the cards until the load
 * that path triggers has finished.
 */
export function ProjectSummary({ state }: { state: WorkbenchStateDto }) {
  const { config, git } = state;
  return (
    <Card title={`Project: ${config.project_name || '(unnamed)'}`}>
      <KeyValue label="root" value={state.root} mono />
      {git ? (
        <>
          <KeyValue label="branch" value={git.branch || '(none)'} mono />
          <KeyValue label="dirty files" value={git.dirty_files} />
        </>
      ) : (
        <KeyValue label="git" value="not a repository" />
      )}
      <KeyValue
        label="mainline"
        value={config.mainline_session ? config.mainline_session.slice(0, 8) : '(unset)'}
        mono
      />
      {config.toml_raw ? (
        <details className={styles.toml}>
          <summary className={styles.summary}>.firment/workbench.toml</summary>
          <pre className={styles.raw}>{config.toml_raw}</pre>
        </details>
      ) : (
        <p className={styles.none}>
          No .firment/workbench.toml yet — creating a branch will generate it.
        </p>
      )}
    </Card>
  );
}
