import { describe, expect, it } from 'vitest';
import { executeTool, validateArgs } from '../tools';

/**
 * Argument-contract tests (FIR-003). A tool call whose required arguments are
 * missing or empty used to run anyway, which produced a silently WRONG success
 * rather than an error: `grep` with an empty pattern matches every line, and
 * `list_dir` with no path lists the whole sandbox.
 */

describe('validateArgs', () => {
  it('rejects a missing required argument', () => {
    expect(validateArgs('grep', {})).toMatch(/missing required argument "pattern"/);
    expect(validateArgs('read_file', {})).toMatch(/missing required argument "path"/);
    expect(validateArgs('web_fetch', {})).toMatch(/missing required argument "url"/);
  });

  it('rejects an empty (whitespace-only) required string', () => {
    expect(validateArgs('grep', { pattern: '' })).toMatch(/must not be empty/);
    expect(validateArgs('glob', { pattern: '   ' })).toMatch(/must not be empty/);
    expect(validateArgs('web_search', { query: '' })).toMatch(/must not be empty/);
  });

  it('rejects a required argument of the wrong type', () => {
    expect(validateArgs('read_file', { path: 42 })).toMatch(/must be a string/);
    expect(validateArgs('grep', { pattern: null })).toMatch(/is null/);
  });

  it('accepts a well-formed call', () => {
    expect(validateArgs('grep', { pattern: 'dma_start' })).toBeNull();
    expect(validateArgs('read_file', { path: 'src/main.c', limit: 50 })).toBeNull();
  });

  it('still allows a call that legitimately takes no arguments', () => {
    // list_dir declares `required: []` — defaulting to the sandbox root is the
    // documented behavior, so validation must not block it.
    expect(validateArgs('list_dir', {})).toBeNull();
  });

  it('leaves unknown tools to the dispatcher', () => {
    expect(validateArgs('no_such_tool', {})).toBeNull();
  });

  it('rejects a non-object argument bag', () => {
    expect(validateArgs('grep', 'pattern' as unknown as Record<string, any>)).toMatch(/must be a JSON object/);
  });
});

describe('executeTool argument gate', () => {
  it('refuses to run a tool whose arguments are unusable', async () => {
    const result = await executeTool('grep', {}, process.cwd(), {});
    expect(result.success).toBe(false);
    expect(result.error).toMatch(/missing required argument "pattern"/);
  });

  it('refuses to run a required-string tool with an empty pattern', async () => {
    const result = await executeTool('grep', { pattern: '' }, process.cwd(), {});
    expect(result.success).toBe(false);
    expect(result.output).toBe('');
  });
});
