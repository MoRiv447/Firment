import { describe, expect, it } from 'vitest';
import { isUnder, pathKey } from '../paths';

/**
 * The two comparisons the workbench makes.
 *
 * `pathKey` is the identity, `isUnder` is the question asked of it, and the
 * interesting cases are both the ones that look equal and the one that looks
 * equal and is not: a prefix that is a word rather than a directory.
 */

describe('pathKey', () => {
  it('reads a drive, a separator style and a trailing slash as one path', () => {
    expect(pathKey('D:\\fw\\thermostat')).toBe(pathKey('d:/fw/thermostat/'));
    expect(pathKey('D:/fw')).toBe(pathKey('D:\\fw\\'));
  });

  it('keeps a drive root comparable to what hangs off it', () => {
    // `D:` alone is the root; the session directories below it are `D:/…`, so a
    // rule that stripped the separator from both would make `D:` match `D:fw`.
    expect(pathKey('D:\\')).toBe('d:');
    expect(pathKey('D:/')).toBe('d:');
  });
});

describe('isUnder', () => {
  it('accepts the directory itself and anything below it', () => {
    expect(isUnder('D:\\fw', 'D:/fw')).toBe(true);
    expect(isUnder('D:\\fw', 'D:\\fw\\thermostat\\src')).toBe(true);
  });

  it('refuses a sibling whose name starts with the same letters', () => {
    // The bug this line exists for: a plain `startsWith` on the raw string makes
    // `D:/fw/thermo` swallow every session under `D:/fw/thermostat`.
    expect(isUnder('D:\\fw\\thermo', 'D:/fw/thermostat')).toBe(false);
  });

  it('refuses anything else', () => {
    expect(isUnder('D:\\fw', 'E:/fw')).toBe(false);
    expect(isUnder('D:\\fw', 'D:/')).toBe(false);
  });
});
