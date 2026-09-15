import { describe, expect, it } from 'vitest';

/**
 * The rules that hold the layer together, checked instead of remembered.
 *
 * "The layer" is `src/ui` plus `src/shell`: the primitives and the four regions
 * built out of them. Stage 8 tightens these into CI, so they live as tests rather
 * than as prose in a document nobody opens. The files are read as text through
 * Vite's `?raw` import rather than `node:fs` -- the GUI has deliberately no Node
 * types (see the `@ts-expect-error` in `vite.config.ts`), and a gate that needs a
 * new dependency to run is a gate that gets deleted.
 *
 * Each rule exists because the old tree broke it:
 *
 * * `styles/tokens.ts` is a JS object, so a colour read from it was frozen at
 *   whichever scheme the cache had picked when it was first asked. That is the
 *   specific reason the light scheme never followed a theme flip: the shell asked
 *   `color.surface` for a string and got one.
 * * `data-ui` is the hash-proof handle. CSS Modules rewrite every class name, so
 *   `[class*="chip"]` is the only other way to find a component from a test, and
 *   that breaks the moment the file is renamed. The names are a closed set: adding
 *   one means editing the list below, which is a cheaper conversation than a test
 *   that silently stops matching.
 * * Element selectors belong to `base.css` alone. A `.module.css` that says
 *   `button { … }` styles every button in the app through its hashed class, and
 *   the reset races with the component.
 * * Inline `style` carries custom properties and nothing else. A component that
 *   sets `style.top` in JS opts out of the cascade for the whole subtree beneath
 *   it -- no media query, no theme rule, no override can reach it. `usePopover`
 *   and `Slider` are the two places that put a number on the screen, and both do
 *   it by writing `--pop-top` / `--slider-fill` for CSS to consume.
 * * `className` is opt-in per primitive, and every opt-in has to be on this list.
 *   A design system where any component accepts a class string is a design system
 *   where callers override spacing from two files away.
 */

const SOURCES = import.meta.glob(
  [
    '../*.ts',
    '../*.tsx',
    '../*.css',
    '../../shell/*.tsx',
    '../../shell/*.css',
    '../../shell/panes/*.tsx',
    '../../shell/panes/*.css',
    // Stage 3 moved the transcript onto the layer and stage 5 moved the session
    // rail and the two kernel dialogs, so all three are gated like the layer.
    // Named one file at a time rather than `../../components/*.tsx`: the one
    // missing from the list -- `ActionButton.tsx` -- is an antd leftover that
    // stage 7 deletes, and a wildcard would fail the suite over code that is
    // already scheduled to die.
    '../../components/Dialogs.tsx',
    '../../components/Dialogs.module.css',
    '../../components/LiveRun.tsx',
    '../../components/LiveRun.module.css',
    '../../components/Markdown.tsx',
    '../../components/Markdown.module.css',
    '../../components/MessageList.tsx',
    '../../components/MessageList.module.css',
    '../../components/StepProgress.tsx',
    '../../components/StepProgress.module.css',
    '../../components/ToolCard.tsx',
    '../../components/ToolCard.module.css',
    '../../views/ChatView.tsx',
    '../../views/ChatView.module.css',
    // Stage 6f emptied the workbench's shell: the last antd in the view went
    // with it, so the file is gated like the layer it is now written against.
    // Until this line existed, "no antd in WorkbenchView" was a promise in a
    // handoff document rather than something that could fail.
    '../../views/WorkbenchView.tsx',
    '../../views/WorkbenchView.module.css',
    '../../views/SessionSidebar.tsx',
    '../../views/SessionSidebar.module.css',
    // Stage 6 split the workbench into panes under `views/workbench/`, and every
    // file in that directory is on the layer, so it is gated as a directory. `*`
    // does not cross `/`, which leaves its `__tests__` to the suite that runs them.
    '../../views/workbench/*.tsx',
    '../../views/workbench/*.ts',
    '../../views/workbench/*.css',
    '../../dev/*.tsx',
    '../../dev/*.css',
  ],
  { query: '?raw', import: 'default', eager: true },
) as Record<string, string>;

const paths = Object.keys(SOURCES);
// Both `../../dev/…` and `../../shell/…` start with `../`, so a prefix test would
// read the gallery as part of the layer it displays and demand barrel exports for
// the shell. This means "sitting in `src/ui` itself".
const inUi = (path: string) => /^..\//.test(path) && !path.slice(3).includes('/');
const withExtension = (extension: string) => paths.filter((path) => path.endsWith(extension));

const tsxFiles = withExtension('.tsx');
const tsFiles = withExtension('.ts');
const cssFiles = withExtension('.css');
const codeFiles = [...tsxFiles, ...tsFiles];

const read = (path: string) => SOURCES[path] ?? '';
const nameOf = (path: string) => path.slice(path.lastIndexOf('/') + 1, -'.tsx'.length);

/** Selectors that only make sense in a global sheet, since they have no class. */
const GLOBAL_SELECTORS = new Set([
  '*',
  ':root',
  'html',
  'body',
  'a',
  'aside',
  'b',
  'button',
  'code',
  'div',
  'em',
  'figure',
  'footer',
  'form',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'header',
  'i',
  'img',
  'input',
  'label',
  'li',
  'main',
  'nav',
  'ol',
  'p',
  'pre',
  'section',
  'select',
  'small',
  'span',
  'strong',
  'svg',
  'table',
  'td',
  'textarea',
  'th',
  'tr',
  'ul',
]);

describe('Primitive layer and shell conventions', () => {
  it('imports nothing from antd, which is the point of the layer', () => {
    expect(codeFiles.filter((path) => /from ['"](antd|@ant-design)/.test(read(path)))).toEqual([]);
  });

  it('leaves the JS token module out of the layer and out of the shell', () => {
    // Matching the import rather than the words: `ui/types.ts` names the file in a
    // comment explaining exactly why it does not import it.
    const offenders = codeFiles.filter((path) =>
      /from ['"][^'"]*styles\/tokens['"]/.test(read(path)),
    );
    expect(offenders).toEqual([]);
  });

  it('names every root with a data-ui anchor from a closed list', () => {
    const anchors = new Set<string>();
    for (const path of tsxFiles) {
      for (const [, anchor] of read(path).matchAll(/data-ui="([a-z-]+)"/g)) anchors.add(anchor);
    }
    expect([...anchors].sort()).toEqual([
      'agents-pane',
      'button',
      'callout',
      'card',
      'change-row',
      'changes-pane',
      'chat',
      'checkbox',
      'chip',
      'empty-state',
      'field',
      'hardware-pane',
      'icon',
      'input',
      'inspector',
      'inspector-body',
      'inspector-rail',
      'key-value',
      'live-run',
      'markdown',
      'menu',
      'menu-item',
      'menu-separator',
      'notifications-panel',
      'option',
      'popover',
      'radio',
      'radio-group',
      'scrim',
      'segmented',
      'select',
      'session-rail',
      'session-row',
      'skeleton',
      'slider',
      'splitter',
      'stat',
      'status-bar',
      'status-dot',
      'status-item',
      'step-item',
      'step-progress',
      'switch',
      'tab',
      'tabs',
      'textarea',
      'title-bar',
      'toast',
      'toast-stack',
      'todos-pane',
      'tool-card',
      'tool-card-head',
      'tool-diff',
      'tool-output',
      'tool-result',
      'tool-run',
      'transcript',
      'user-bubble',
      'wordmark',
    ]);
  });

  it('keeps element and global selectors out of the modules', () => {
    const offenders: string[] = [];
    for (const path of cssFiles) {
      // Comments first: a line of prose starting with `*` is indistinguishable
      // from the universal selector, and "a tooltip should wrap" is `a`.
      const source = read(path).replace(/\/\*[\s\S]*?\*\//g, '');
      for (const block of source.split('{')) {
        // What sits between the previous `}` and this `{` is the selector. Reading
        // lines instead would take `display: table` for a rule and miss a rule
        // written on one line.
        const selector = block.slice(block.lastIndexOf('}') + 1).trim();
        if (!selector || selector.startsWith('@') || selector.endsWith(';')) continue;
        // A selector list is only as local as its loosest part, and so is a
        // descendant: `.x button` still styles every button under `.x`.
        for (const part of selector.split(',')) {
          for (const compound of part.trim().split(/[\s>+~]+/)) {
            const head = compound.split(/[:.[{#]/)[0] ?? '';
            if (GLOBAL_SELECTORS.has(head)) offenders.push(`${path}: ${selector}`);
          }
        }
      }
    }
    expect(offenders).toEqual([]);
  });

  it('writes numbers through custom properties, never through an inline style', () => {
    const offenders: string[] = [];
    for (const path of codeFiles) {
      for (const [, body] of read(path).matchAll(/style=\{\{([^}]*)\}\}/g)) {
        for (const [, key] of body.matchAll(/(['"]?)([A-Za-z_$-][\w$-]*)\1\s*:/g)) {
          if (!key.startsWith('--')) offenders.push(`${path}: ${key}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });

  it('opens className to exactly the five components that need it', () => {
    const optedIn = tsxFiles
      .filter((path) => /className\?:\s*string/.test(read(path)))
      .map(nameOf)
      .sort();
    expect(optedIn).toEqual(['Field', 'Icon', 'KeyValue', 'Overlay', 'Popover']);
    // `Button` takes one too, through the native attribute bag -- worth pinning,
    // since it is the door a caller is most likely to walk through.
    expect(read('../Button.tsx')).toContain('NativeButton');
  });

  it('exports every component from the barrel, so a file can be split without a diff', () => {
    const barrel = read('../index.ts');
    /** Not a primitive a view is meant to reach for; it is a dialog's backdrop. */
    const internals = ['Overlay'];
    const missing = tsxFiles
      .filter(inUi)
      .map(nameOf)
      .filter((name) => /^[A-Z]/.test(name) && !internals.includes(name))
      .filter((name) => !barrel.includes(`./${name}'`));
    expect(missing).toEqual([]);
  });
});
