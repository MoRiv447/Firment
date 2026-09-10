import { describe, expect, it } from 'vitest';
import { resolveTheme, setThemeSetting, currentThemeSetting } from '../theme';

/**
 * `ui.theme` resolution.
 *
 * The rule this file exists to protect: `auto` is the default, and on a dark
 * machine `auto` resolves to dark -- so a user who wants to check the light
 * scheme must be able to pin `light`. A "sensible" fallback that quietly
 * prefers the system preference would make the light scheme unreachable on
 * exactly the machines the author works on.
 */

describe('resolveTheme', () => {
  it('lets an explicit choice override the system', () => {
    expect(resolveTheme('light', true)).toBe('light');
    expect(resolveTheme('dark', false)).toBe('dark');
  });

  it('follows the system only for auto', () => {
    expect(resolveTheme('auto', true)).toBe('dark');
    expect(resolveTheme('auto', false)).toBe('light');
  });

  it('tolerates the shapes a hand-edited config produces', () => {
    expect(resolveTheme('  LIGHT ', true)).toBe('light');
    expect(resolveTheme('Dark', false)).toBe('dark');
    // `system` is the word people reach for; firment-core accepts it as auto.
    expect(resolveTheme('system', true)).toBe('dark');
    expect(resolveTheme('', true)).toBe('dark');
    // An unknown word is not a licence to pin a scheme: follow the system.
    expect(resolveTheme('sepia', true)).toBe('dark');
    expect(resolveTheme('sepia', false)).toBe('light');
  });
});

describe('the theme setting store', () => {
  it('round-trips the three values verbatim', () => {
    for (const value of ['light', 'dark', 'auto'] as const) {
      setThemeSetting(value);
      expect(currentThemeSetting()).toBe(value);
    }
  });

  it('normalises anything it does not recognise to auto', () => {
    setThemeSetting('light');
    setThemeSetting('ultraviolet');
    expect(currentThemeSetting()).toBe('auto');
  });

  it('defaults to auto on a fresh module', () => {
    // `auto` is the documented default, and it is what a machine with no
    // config.toml entry must get.
    setThemeSetting('auto');
    expect(currentThemeSetting()).toBe('auto');
  });
});
