import { describe, expect, it } from 'vitest';

import { formatBytes, formatDuration, formatStamp } from '../format';

/**
 * The three formatters, on the numbers the app actually shows.
 *
 * The edges worth pinning are the ones a caller cannot see from a screenshot: the
 * step where 1023 becomes 1.0 KiB, the rounding that turns 59.6s into a minute,
 * the unpadded second that makes a ticking timer one character wider on some
 * ticks than on others, and the day and year boundaries `formatStamp` changes its
 * format at.
 */

describe('formatBytes', () => {
  it('stays in bytes below the first unit', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(1023)).toBe('1023 B');
  });

  it('uses binary units, because these numbers come from a linker', () => {
    expect(formatBytes(1024)).toBe('1.0 KiB');
    expect(formatBytes(131072)).toBe('128.0 KiB');
    expect(formatBytes(1024 * 1024)).toBe('1.0 MiB');
    expect(formatBytes(2 * 1024 * 1024 + 512 * 1024)).toBe('2.5 MiB');
    expect(formatBytes(4 * 1024 * 1024 * 1024)).toBe('4.0 GiB');
  });

  it('ends the ladder at GiB rather than inventing a unit', () => {
    // Nothing a firmware tool reports is terabyte-sized, so the last unit is the
    // one it stops on.
    expect(formatBytes(1024 * 1024 * 1024 * 1024)).toBe('1024.0 GiB');
  });
});

describe('formatDuration', () => {
  it('rounds to whole seconds under a minute', () => {
    expect(formatDuration(0)).toBe('0s');
    expect(formatDuration(999)).toBe('1s');
    expect(formatDuration(47_200)).toBe('47s');
    expect(formatDuration(59_400)).toBe('59s');
  });

  it('switches to minutes at the minute, with the seconds padded', () => {
    // Rounding gets there first: 59.6s is a minute, and it has to read as one.
    expect(formatDuration(59_600)).toBe('1m 00s');
    expect(formatDuration(60_000)).toBe('1m 00s');
    expect(formatDuration(266_000)).toBe('4m 26s');
    expect(formatDuration(3_599_000)).toBe('59m 59s');
  });

  it('drops the seconds past an hour', () => {
    expect(formatDuration(3_600_000)).toBe('1h 00m');
    expect(formatDuration(8_040_000)).toBe('2h 14m');
  });

  it('reads a clock that went backwards as zero, not as a negative', () => {
    expect(formatDuration(-5_000)).toBe('0s');
  });
});

/** The epoch seconds a `SessionSummaryDto.updated_at` would carry for a local moment. */
const at = (y: number, mo: number, d: number, h: number, mi: number) =>
  Math.floor(new Date(y, mo - 1, d, h, mi).getTime() / 1000);

const now = new Date(2026, 8, 14, 16, 30);

describe('formatStamp', () => {
  it('is a clock time for anything still today, including one minute past midnight', () => {
    expect(formatStamp(at(2026, 9, 14, 9, 5), now)).toBe('09:05');
    expect(formatStamp(at(2026, 9, 14, 0, 0), now)).toBe('00:00');
  });

  it('adds the day for the minute before today starts', () => {
    expect(formatStamp(at(2026, 9, 13, 23, 59), now)).toBe('09-13 23:59');
  });

  it('keeps the month as the discriminator across a new year', () => {
    // Same calendar month number, five months ago: the year is still this one, so
    // it is the day that has to carry the difference.
    expect(formatStamp(at(2026, 1, 5, 8, 0), now)).toBe('01-05 08:00');
  });

  it('spends the four digits only when the year is what differs', () => {
    expect(formatStamp(at(2025, 12, 31, 23, 59), now)).toBe('2025-12-31 23:59');
  });
});
