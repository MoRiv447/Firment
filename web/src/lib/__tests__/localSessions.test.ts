import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { loadSessions, saveSessions } from '../localSessions';

/**
 * Persistence failure tests (FIR-004). The node test environment has no
 * localStorage, so a throwaway stub is installed per test — including the two
 * failure modes that used to be swallowed: an unreadable/corrupt blob and a
 * full quota.
 */

interface StorageStub {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
  dump(): Map<string, string>;
}

function installStorage(options: { throwOnGet?: Error; throwOnSet?: Error } = {}): StorageStub {
  const store = new Map<string, string>();
  const stub: StorageStub = {
    getItem: (key) => {
      if (options.throwOnGet) throw options.throwOnGet;
      return store.get(key) ?? null;
    },
    setItem: (key, value) => {
      if (options.throwOnSet) throw options.throwOnSet;
      store.set(key, value);
    },
    removeItem: (key) => {
      store.delete(key);
    },
    dump: () => store,
  };
  vi.stubGlobal('localStorage', stub);
  return stub;
}

const KEY = 'firment-sessions-v2';
const CORRUPT_KEY = 'firment-sessions-v2.corrupt';

function quotaError(): Error {
  const err = new Error('exceeded the quota');
  err.name = 'QuotaExceededError';
  return err;
}

beforeEach(() => installStorage());
afterEach(() => vi.unstubAllGlobals());

describe('loadSessions', () => {
  it('returns stored sessions when the blob is valid', () => {
    installStorage();
    localStorage.setItem(KEY, JSON.stringify([{ id: 'a', messages: [], title: 't', createdAt: 1, updatedAt: 1 }]));
    const result = loadSessions();
    expect(result.error).toBeUndefined();
    expect(result.sessions.map((s) => s.id)).toEqual(['a']);
  });

  it('treats an empty store as an empty list without an error', () => {
    expect(loadSessions()).toEqual({ sessions: [] });
  });

  it('reports unreadable storage instead of pretending the list is empty', () => {
    installStorage({ throwOnGet: new Error('SecurityError') });
    const result = loadSessions();
    expect(result.sessions).toEqual([]);
    expect(result.error).toMatch(/unreadable/i);
  });

  it('backs up unparseable JSON to a .corrupt key and reports it', () => {
    const stub = installStorage();
    stub.setItem(KEY, '{not json');
    const result = loadSessions();
    expect(result.error).toMatch(/corrupt/);
    expect(stub.getItem(CORRUPT_KEY)).toBe('{not json');
  });

  it('treats a non-array payload as corrupt rather than handing it to callers', () => {
    const stub = installStorage();
    stub.setItem(KEY, JSON.stringify({ a: 1 }));
    const result = loadSessions();
    expect(result.sessions).toEqual([]);
    expect(result.error).toMatch(/unexpected structure/);
    expect(stub.getItem(CORRUPT_KEY)).toContain('"a"');
  });

  it('treats array entries without the session shape as corrupt', () => {
    const stub = installStorage();
    stub.setItem(KEY, JSON.stringify([{ title: 'no id, no messages' }]));
    expect(loadSessions().error).toMatch(/unexpected structure/);
  });
});

describe('saveSessions', () => {
  it('returns null on success', () => {
    const stub = installStorage();
    expect(saveSessions([{ id: 'a', title: 't', createdAt: 1, updatedAt: 1, messages: [] }])).toBeNull();
    expect(stub.getItem(KEY)).toContain('"a"');
  });

  it('surfaces a quota failure instead of dropping the write', () => {
    installStorage({ throwOnSet: quotaError() });
    const err = saveSessions([{ id: 'a', title: 't', createdAt: 1, updatedAt: 1, messages: [] }]);
    expect(err).toMatch(/storage is full/i);
  });

  it('surfaces any other write failure with its message', () => {
    installStorage({ throwOnSet: new Error('disk gone') });
    expect(saveSessions([])).toMatch(/disk gone/);
  });
});
