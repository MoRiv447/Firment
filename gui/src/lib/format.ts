/**
 * The two formatters the app was writing inline.
 *
 * They live here rather than in `ui/` because they are not visual: `formatBytes`
 * is a fact about a number, and a primitive that formatted its own contents could
 * not be told what unit to show.
 *
 * The duplication they replace is small but real -- `(bytes / 1024).toFixed(1)`
 * appeared twice in the workbench insights and `Math.round((now - start) / 1000)`
 * twice in the run timer -- and the two copies already disagreed: one said "KiB",
 * the other said "s" with no space, so the shell had two density rules for the
 * same kind of number.
 */

const UNITS = ['B', 'KiB', 'MiB', 'GiB'] as const;

/**
 * Binary units, one decimal above 1 KiB.
 *
 * KiB not KB: the values come from a linker (`flash_bytes`, `ram_bytes`), where
 * 1024 is what the number actually means, and a card that says "128 KB" for a
 * 131072-byte part is off by three percent in the one place a firmware user
 * checks the arithmetic.
 */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${UNITS[unit]}`;
}

/**
 * A span of time in the shortest form that still says something.
 *
 * Under a minute it is seconds -- a build that took 47s should not read as 0.8min.
 * Past an hour the minutes are what matters, so the seconds drop out rather than
 * making a 2h 14m run into a six-character number nobody parses at a glance.
 */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  if (total < 60) return `${total}s`;
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  if (minutes < 60) return `${minutes}m ${seconds.toString().padStart(2, '0')}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${(minutes % 60).toString().padStart(2, '0')}m`;
}
