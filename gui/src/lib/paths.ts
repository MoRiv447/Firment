/**
 * Comparing two Windows paths without comparing their spelling.
 *
 * Every project directory here can arrive in more shapes than one: the sidebar
 * and the path field hold whatever the user typed or the picker returned, the
 * session records hold what the process was started with, and `localStorage`
 * holds whichever of those was saved first. A `\` and a `/` are the same
 * directory, `D:\fw\` and `d:/fw` are the same directory, and a comparison that
 * misses that shows the same project twice -- or, one function further on, hides
 * a session that belongs to the folder that is open.
 *
 * So these two are the only comparisons the workbench makes. They are here rather
 * than inline because the same normalisation is needed by the recent-projects
 * list, by the session filter, and by whatever reads those two next.
 */

/** Case, separator style and a trailing slash are spelling, not identity. */
export function pathKey(dir: string): string {
  return dir.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
}

/**
 * Is `dir` `root` itself or below it?
 *
 * The boundary matters: a plain `startsWith` makes `D:/fw/thermo` swallow every
 * session under `D:/fw/thermostat`, which reads as a project containing another
 * project's history. The separator is appended to the root rather than stripped
 * from the candidate, so a drive root (`D:`) still matches `D:/firmware`.
 */
export function isUnder(root: string, dir: string): boolean {
  const base = pathKey(root);
  const candidate = pathKey(dir);
  return candidate === base || candidate.startsWith(`${base}/`);
}
