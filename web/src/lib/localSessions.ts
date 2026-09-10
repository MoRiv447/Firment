import { ChatMessage } from './types';

export interface LocalSession {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  messages: ChatMessage[];
}

const SESSIONS_KEY = 'firment-sessions-v2';
const CORRUPT_KEY = 'firment-sessions-v2.corrupt';
const CURRENT_KEY = 'firment-current-v2';

/**
 * Session persistence result types.
 *
 * A swallowed storage error here is destructive, not cosmetic: callers read
 * the list at mount and write the whole list back on every change, so a
 * failed/blank read followed by a save replaces the store with a single
 * session. Both functions therefore report failure to the caller instead of
 * returning an empty list or `void`.
 */
export interface LoadSessionsResult {
  sessions: LocalSession[];
  /** Set when the stored blob could not be used; it has been backed up. */
  error?: string;
}

function genId(): string {
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    const v = c === 'x' ? r : (r & 0x3) | 0x8;
    return v.toString(16);
  });
}

function isSessionLike(value: any): value is LocalSession {
  return !!value && typeof value === 'object' && typeof value.id === 'string' && Array.isArray(value.messages);
}

/** Keep an unreadable blob under a sibling key so a user can still recover it. */
function quarantine(raw: string): void {
  try {
    // Never clobber an earlier backup: after a bad load the app immediately
    // writes a fresh store, so a *later* corrupt read is a different blob while
    // the first backup is still the user's only copy of their real sessions.
    for (let slot = 0; slot < 5; slot++) {
      const key = slot === 0 ? CORRUPT_KEY : `${CORRUPT_KEY}.${slot}`;
      const existing = localStorage.getItem(key);
      if (existing === raw) return;
      if (existing === null) {
        localStorage.setItem(key, raw);
        return;
      }
    }
  } catch {
    /* backup is best-effort; the returned error already says data is unusable */
  }
}

export function loadSessions(): LoadSessionsResult {
  let raw: string | null;
  try {
    raw = localStorage.getItem(SESSIONS_KEY);
  } catch (err: any) {
    return { sessions: [], error: `Local session storage is unreadable (${err?.message || err}).` };
  }
  if (!raw) return { sessions: [] };

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    quarantine(raw);
    return {
      sessions: [],
      error: 'Saved sessions could not be parsed and were backed up to a ".corrupt" key; starting from an empty list.',
    };
  }
  if (!Array.isArray(parsed) || !parsed.every(isSessionLike)) {
    quarantine(raw);
    return {
      sessions: [],
      error: 'Saved sessions have an unexpected structure and were backed up to a ".corrupt" key; starting from an empty list.',
    };
  }
  return { sessions: parsed };
}

/** Returns `null` on success, or a human-readable reason the write failed. */
export function saveSessions(sessions: LocalSession[]): string | null {
  try {
    localStorage.setItem(SESSIONS_KEY, JSON.stringify(sessions));
    return null;
  } catch (err: any) {
    const quota =
      err?.name === 'QuotaExceededError' ||
      err?.code === 22 ||
      err?.code === 1014 ||
      /quota/i.test(err?.message || '');
    return quota
      ? 'Browser storage is full — this chat was not saved. Delete older sessions to free space.'
      : `Could not save sessions (${err?.message || err}).`;
  }
}

export function loadCurrentId(): string | null {
  try {
    return localStorage.getItem(CURRENT_KEY);
  } catch {
    return null;
  }
}

export function saveCurrentId(id: string | null): void {
  try {
    if (id) localStorage.setItem(CURRENT_KEY, id);
    else localStorage.removeItem(CURRENT_KEY);
  } catch {
    /* ignore */
  }
}

export function createSession(): LocalSession {
  const now = Date.now();
  return {
    id: genId(),
    title: 'New chat',
    createdAt: now,
    updatedAt: now,
    messages: [],
  };
}
