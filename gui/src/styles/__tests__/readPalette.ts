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
  const at = source.indexOf(`:root[data-scheme="${name}"]`);
  if (at < 0) throw new Error(`no :root[data-scheme="${name}"] block in tokens.css`);
  const body = source.match(new RegExp(`:root\\[data-scheme="${name}"\\]\\s*\\{([^}]*)\\}`));
  if (!body) throw new Error(`the ${name} block has no body`);
  const out: Record<string, string> = {};
  for (const m of body[1].matchAll(/([-\w]+)\s*:\s*([^;]+);/g)) out[m[1]] = m[2].trim();
  return out;
}

/** The camel-cased name a test would use, so `paletteFor('dark').ink` reads the same. */
export function named(name: 'dark' | 'light'): Record<string, string> {
  const raw = scheme(name);
  const camel = (key: string) => key.replace(/^--/, '').replace(/-([a-z0-9])/g, (_, c: string) => c.toUpperCase());
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(raw)) out[camel(key)] = value;
  // the JS palette merged lineStrong into `outline`; the CSS keeps both names
  out.outline = raw['--outline'];
  return out;
}
