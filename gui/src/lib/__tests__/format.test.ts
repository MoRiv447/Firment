import { describe, expect, it } from 'vitest';

import { formatBytes, formatDuration } from '../format';

/**
 * The two formatters, on the numbers the app actually shows.
 *
 * The edges worth pinning are the ones a caller cannot see from a screenshot: the
 * step where 1023 becomes 1.0 KiB, the rounding that turns 59.6s into a minute,
 * and the unpadded second that makes a ticking timer one character wider on some
 * ticks than on others.
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
