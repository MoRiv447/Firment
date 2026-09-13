/**
 * Which colour scheme the GUI draws in.
 *
 * `ui.theme` in config.toml (see `crates/firment-core/src/config.rs`) is the
 * setting; this module turns it into the concrete `ThemeMode` the token layer
 * wants, and keeps the answer live when the setting is `auto`.
 *
 * The whole point of `auto` being a *setting* rather than the only behaviour:
 * on a dark machine `auto` resolves to dark, so a user who wants to check the
 * light scheme has to be able to pin it. `resolveTheme` is therefore total --
 * it never falls back to the system preference behind the user's back.
 */

import { createContext, useContext, useEffect, useState } from 'react';

/**
 * The two colour schemes.
 *
 * Declared here rather than in `styles/tokens.ts`, where it used to live: the
 * token layer is deleted with antd, and a type that outlives it -- the theme
 * setting, the scheme attribute on `<html>`, every context that carries a mode --
 * cannot be allowed to go with it.
 */
export type ThemeMode = 'dark' | 'light';

/** Mirrors `UiTheme` in firment-core. `system` is accepted as an alias. */
export type ThemeSetting = 'auto' | 'light' | 'dark';

/**
 * The scheme for a setting.
 *
 * Pure and total by design: an unknown value (a hand-edited config, a newer
 * core that grew a fourth option) behaves as `auto` rather than throwing or
 * silently pinning a scheme the user did not ask for.
 */
export function resolveTheme(setting: string, systemIsDark: boolean): ThemeMode {
  switch (setting.trim().toLowerCase()) {
    case 'light':
      return 'light';
    case 'dark':
      return 'dark';
    default:
      return systemIsDark ? 'dark' : 'light';
  }
}

/** The OS preference, or `false` where `matchMedia` does not exist (jsdom). */
export function systemPrefersDark(): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return false;
  }
  return window.matchMedia('(prefers-color-scheme: dark)').matches;
}

/**
 * Track the OS preference, but only while it can matter.
 *
 * Subscribing unconditionally would re-render on every OS theme flip even when
 * the user pinned `light` or `dark`; callers pass `auto`.
 */
export function useSystemPrefersDark(enabled: boolean): boolean {
  const [isDark, setIsDark] = useState(systemPrefersDark);

  useEffect(() => {
    if (!enabled) return;
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const query = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = (event: MediaQueryListEvent) => setIsDark(event.matches);
    setIsDark(query.matches);
    // `addEventListener` is the modern API; jsdom implements it, and the
    // legacy `addListener` path is not worth carrying for a Tauri webview.
    query.addEventListener('change', onChange);
    return () => query.removeEventListener('change', onChange);
  }, [enabled]);

  return isDark;
}

/**
 * The active scheme, for components that must re-render when it changes.
 *
 * Most components do not need this: they read `color.x` during render, and a
 * re-render of the tree picks up the new palette by itself. The exception is a
 * `React.memo` component -- memoisation compares props, so a theme change
 * leaves it showing the previous scheme's colours. Subscribing to this context
 * is what makes memoised subtrees follow the theme.
 */
export const ThemeModeContext = createContext<ThemeMode>('dark');

/** The active scheme; subscribing also invalidates `React.memo`. */
export function useThemeMode(): ThemeMode {
  return useContext(ThemeModeContext);
}

// ---------------------------------------------------------------------------
// The `ui.theme` setting itself.
//
// A module-level store rather than props: the setting is read at the top of
// `App` (which paints the shell) but written from `SettingsView`, which is
// rendered several levels down with no props of its own. Threading it through
// would mean the settings view had to know where the shell lives.
// ---------------------------------------------------------------------------

let currentSetting: ThemeSetting = 'auto';
const settingListeners = new Set<(setting: ThemeSetting) => void>();

/** Publish a new setting. Unknown values fall back to `auto`. */
export function setThemeSetting(next: string): void {
  const normalised: ThemeSetting =
    next.trim().toLowerCase() === 'light'
      ? 'light'
      : next.trim().toLowerCase() === 'dark'
        ? 'dark'
        : 'auto';
  if (normalised === currentSetting) return;
  currentSetting = normalised;
  for (const listener of settingListeners) listener(normalised);
}

/** The setting as it stands, for non-React callers. */
export function currentThemeSetting(): ThemeSetting {
  return currentSetting;
}

/** The setting, re-rendering the caller when it changes. */
export function useThemeSetting(): ThemeSetting {
  const [setting, setSetting] = useState(currentSetting);
  useEffect(() => {
    settingListeners.add(setSetting);
    // The store can outlive a mount (settings view unmounts), so resync on
    // subscribe instead of trusting the initial `useState` value.
    setSetting(currentSetting);
    return () => {
      settingListeners.delete(setSetting);
    };
  }, []);
  return setting;
}

// ---------------------------------------------------------------------------
// The pre-paint contract.
//
// `index.html` cannot await `get_settings`, so it decides the first frame from
// these two keys and the `matchMedia` fallback. `publishScheme` is the write
// side: without it the cache freezes at whatever the very first boot guessed,
// and a later `ui.theme = light` would keep flashing dark.
// ---------------------------------------------------------------------------

/** The `ui.theme` value itself: `auto` must survive, or the OS stops mattering. */
export const THEME_SETTING_KEY = 'firment.ui.theme';
/** The resolved `dark`/`light`, so the next boot needs no `matchMedia` guess. */
export const RESOLVED_SCHEME_KEY = 'firment.scheme.resolved';

/** Paint the scheme and leave it cached for the next cold start. */
export function publishScheme(setting: ThemeSetting, mode: ThemeMode): void {
  if (typeof document === 'undefined') return;
  document.documentElement.dataset.scheme = mode;
  try {
    localStorage.setItem(THEME_SETTING_KEY, setting);
    if (localStorage.getItem(RESOLVED_SCHEME_KEY) !== mode) {
      localStorage.setItem(RESOLVED_SCHEME_KEY, mode);
    }
  } catch {
    // A blocked store costs the next boot its cache and nothing else: the
    // scheme is already painted by the line above.
  }
}
