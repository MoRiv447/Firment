// Read the palette out of the stylesheet, for the tests.
//
// `styles/tokens.ts` mirrored `tokens.css` and nothing in the app imported it -- the
// only readers were two test files and one bridge test comparing the copy to the
// original. This reads the original instead, which is one source of truth rather
// than two that agree because a test says so.
//
// Not a `.test.` file: it exports a helper and has no assertions, so the runner
// collects nothing from it.
import sheet from '../tokens.css?raw';

/** Strip comments, and normalise a value for comparison. */
export const withoutComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '');

export function scheme(name: 'dark' | 'light'): Record<string, string> {
  const source = withoutComments(sheet);
  const body = block(name, source);
  const out: Record<string, string> = {};
  for (const m of body.matchAll(/([-\w]+)\s*:\s*([^;]+);/g)) out[m[1]] = m[2].trim();
  return out;
}

/**
 * One scheme block exactly as it is written, comments included.
 *
 * `scheme()` throws the comments away because a value compared to a value wants no prose in
 * the way. A test that reads the reasons written *beside* the values -- the ratios the file
 * quotes to justify them -- needs the other half, and it needs to know which declaration
 * each comment belongs to, so it gets the block itself rather than a stripped copy.
 */
export function body(name: 'dark' | 'light'): string {
  return block(name, sheet);
}

function block(name: 'dark' | 'light', source: string): string {
  const at = source.indexOf(`:root[data-scheme="${name}"]`);
  if (at < 0) throw new Error(`no :root[data-scheme="${name}"] block in tokens.css`);
  const found = new RegExp(
    `:root\\[data-scheme="${name}"\\]\\s*\\{([\\s\\S]*?)\\n\\}`,
  ).exec(source.slice(at));
  if (!found) throw new Error(`the ${name} block has no body`);
  return found[1];
}

/** The camel-cased name a test would use, so `paletteFor('dark').ink` reads the same. */
export function named(name: 'dark' | 'light'): Record<string, string> {
  const raw = scheme(name);
  const camel = (key: string) => key.replace(/^--/, '').replace(/-([a-z0-9])/g, (_, c: string) => c.toUpperCase());
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(raw)) out[camel(key)] = value;
  return out;
}
