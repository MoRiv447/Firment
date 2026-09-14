import { describe, expect, it } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { ChangesPane } from '../ChangesPane';
import type { FileChange } from '../../../lib/changes';
import sheetRaw from '../ChangesPane.module.css?raw';

/**
 * The Changes pane, against the shapes the transcript really produces.
 *
 * Three claims matter here, and all three are checkable without a browser: the
 * list is one row per file (a file edited three times is not three rows), the
 * numbers on the row are the numbers of the body the row opens, and a row with no
 * body does not pretend to have one. The diff itself is the card's renderer, so
 * the transcript and this pane cannot drift into two opinions of a `+` line.
 *
 * Colours are asserted against the sheet rather than the computed style: jsdom
 * never substitutes `var()`, so a computed check here would compare two empty
 * strings and pass for the wrong reason.
 */

const sheet = sheetRaw.replace(/\/\*[\s\S]*?\*\//g, '');

/** The declarations of one rule, looked up by its literal selector. */
function declarations(selector: string): Record<string, string> {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const body = new RegExp(`${escaped}\\s*\\{([^}]*)\\}`).exec(sheet)?.[1] ?? '';
  return Object.fromEntries(
    [...body.matchAll(/([\w-]+)\s*:\s*([^;]+)/g)].map(([, prop, value]) => [
      prop.trim(),
      value.trim(),
    ]),
  );
}

const changed = (over: Partial<FileChange> & { path: string }): FileChange => ({
  created: false,
  added: 1,
  removed: 1,
  diff: '@@ -1,2 +1,2 @@\n-a\n+b',
  truncated: false,
  edits: 1,
  ...over,
});

describe('ChangesPane', () => {
  it('says what is missing when the list is empty', () => {
    render(<ChangesPane changes={[]} />);
    expect(screen.getByText('No files changed')).toBeInTheDocument();
    // The blind spot is named rather than left as an empty box: a `shell` call
    // that rewrote a file never prints a diff, so it can never appear here.
    expect(screen.getByText(/shell/)).toBeInTheDocument();
  });

  it('leads with the newest touch and shows the tail of the path', () => {
    const { container } = render(
      <ChangesPane
        changes={[
          changed({ path: 'D:\\OldStudy66\\Firment\\src\\foc\\current.c' }),
          changed({ path: 'src/main.c' }),
        ]}
      />,
    );
    const paths = [...container.querySelectorAll('span[title]')];
    expect(paths.map((p) => p.textContent)).toEqual(['…/foc/current.c', 'src/main.c']);
    // The short form is for a 240px column; the whole path is still on the row.
    expect(paths[0]).toHaveAttribute('title', 'D:\\OldStudy66\\Firment\\src\\foc\\current.c');
  });

  it('summarises the column in one line, new files included', () => {
    render(
      <ChangesPane
        changes={[
          changed({ path: 'a.c' }),
          changed({ path: 'b.c', added: 4, removed: 2 }),
          changed({ path: 'c.c', created: true, added: null, removed: null, diff: null }),
        ]}
      />,
    );
    expect(screen.getByText('+5')).toBeInTheDocument();
    expect(screen.getByText('-3')).toBeInTheDocument();
    expect(screen.getByText('3 files')).toBeInTheDocument();
    // `+5 -3` is a count of two of the three rows, so the third is said as a
    // number rather than quietly dropped from the totals.
    expect(screen.getByText('1 new')).toBeInTheDocument();
  });

  it('says nothing about counts when there are none to give', () => {
    // A session that only wrote new files has no lines added or removed on
    // record; a `+0 -0` header would read as "it changed nothing".
    render(
      <ChangesPane
        changes={[changed({ path: 'c.c', created: true, added: null, removed: null, diff: null })]}
      />,
    );
    expect(screen.queryByText(/^\+\d+$/)).toBeNull();
    expect(screen.getByText('1 new')).toBeInTheDocument();
  });

  it('opens the diff the counts describe', () => {
    render(<ChangesPane changes={[changed({ path: 'src/main.c' })]} />);
    const row = screen.getByRole('button', { name: /src\/main.c/ });
    expect(row).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(row);
    expect(row).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('+b')).toBeInTheDocument();
  });

  it('keeps a row with no diff closed, because there is nothing to open', () => {
    render(
      <ChangesPane
        changes={[
          changed({ path: 'src/new.c', created: true, added: null, removed: null, diff: null }),
        ]}
      />,
    );
    expect(screen.queryByRole('button')).toBeNull();
    expect(screen.getByText('new file')).toBeInTheDocument();
  });

  it('says how many times a file was touched, since the counts cover only the last', () => {
    render(<ChangesPane changes={[changed({ path: 'src/main.c', edits: 3 })]} />);
    expect(screen.getByText('×3')).toBeInTheDocument();
  });

  it('marks a body the parser had to cut', () => {
    render(<ChangesPane changes={[changed({ path: 'src/main.c', truncated: true })]} />);
    expect(screen.getByText('part')).toBeInTheDocument();
  });

  it('paints the counts from the diff family, as the card does', () => {
    expect(declarations(".counts [data-kind='added']")).toMatchObject({
      color: 'var(--diff-added-ink)',
    });
    expect(declarations(".counts [data-kind='removed']")).toMatchObject({
      color: 'var(--diff-removed-ink)',
    });
  });

  it('invites a click only on a row that folds', () => {
    // The `<div>` and the `<button>` share `.row`, so the affordance has to come
    // from the attribute the button carries, not from the element name.
    expect(declarations('.row')).not.toHaveProperty('cursor');
    expect(declarations(".row[data-fold='true']")).toMatchObject({ cursor: 'pointer' });
  });
});
