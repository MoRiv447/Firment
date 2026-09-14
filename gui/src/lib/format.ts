/**
 * The formatters the app was writing inline.
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
 *
 * `formatStamp` joined for the same reason: the session rail printed a full
 * `toLocaleString()` under every title, which is 19 characters of locale to say
 * something that only needs four or eleven.
 */

const UNITS = ['B', 'KiB', 'MiB', 'GiB'] as const;

/** Two digits, because every caller is printing a time or a day. */
const pad = (value: number) => value.toString().padStart(2, '0');

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
  if (minutes < 60) return `${minutes}m ${pad(seconds)}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${pad(minutes % 60)}m`;
}

/**
 * A moment in the shortest form that still says how recent it is.
 *
 * A list of sessions is scanned for recency, and everything a full localised
 * date prints -- the weekday, the year, the seconds -- is true of every row in
 * it at once. So today is a clock time, this year adds the day, and only a
 * date from another year spends the four digits that make it one. The caller
 * keeps `toLocaleString()` for the row's `title`, which is the one place the
 * full reading belongs.
 *
 * `epochSeconds`, not milliseconds, because that is the unit the DTO carries:
 * `updated_at` comes out of a Rust `SystemTime` as seconds, and a formatter
 * taking milliseconds would leave every call site multiplying.
 *
 * `now` is a `Date` rather than a `Date.now()` inside, so the "is this the same
 * day" branch can be tested on a fixed pair of dates instead of on whatever day
 * the suite happens to run on -- and so it is a `Date`, not a second number in
 * the wrong unit.
 */
export function formatStamp(epochSeconds: number, now: Date = new Date()): string {
  const at = new Date(epochSeconds * 1000);
  const clock = `${pad(at.getHours())}:${pad(at.getMinutes())}`;
  // `toDateString` is the one date string the language specifies rather than
  // localises, so "same calendar day" cannot depend on the host's locale.
  if (at.toDateString() === now.toDateString()) return clock;
  const day = `${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
  if (at.getFullYear() === now.getFullYear()) return `${day} ${clock}`;
  return `${at.getFullYear()}-${day} ${clock}`;
}
