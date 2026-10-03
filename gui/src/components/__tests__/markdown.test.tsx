import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * The one place agent-authored HTML reaches the person, and the one place it can reach the shell.
 *
 * `Markdown` renders text the model wrote — often a summary of a web page it fetched. An ordinary
 * anchor there is a click that either replaces the app's own document or runs a `javascript:` URL
 * in the webview that holds the IPC bridge, so the two decisions this file pins are the whole
 * security boundary of the transcript: which schemes become links at all, and which clicks the
 * component is allowed to take over.
 *
 * The opener plugin is the only thing here outside the app, and it is a real `@tauri-apps` module
 * with no jsdom implementation: what matters is whether the component decided to call it, not what
 * a desktop shell would do with the URL.
 */
const { openUrl } = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl }));

import { Markdown } from '../Markdown';

beforeEach(() => {
  openUrl.mockReset().mockResolvedValue(undefined);
});

const anchor = () => screen.queryByRole('link', { name: 'docs' });

describe('Markdown: a link leaves the webview', () => {
  it('hands an http link to the operating system and navigates nothing', () => {
    render(<Markdown>{'[docs](https://example.com/a)'}</Markdown>);
    // `fireEvent.click` returns false only when the handler called preventDefault: the app's own
    // document has to survive the click.
    expect(fireEvent.click(anchor()!)).toBe(false);
    expect(openUrl).toHaveBeenCalledWith('https://example.com/a');
  });

  it.each(['ctrlKey', 'metaKey', 'shiftKey', 'altKey'])(
    'leaves a %s click to whatever the reader meant by it',
    (modifier) => {
      render(<Markdown>{'[docs](https://example.com/a)'}</Markdown>);
      expect(fireEvent.click(anchor()!, { [modifier]: true })).toBe(true);
      expect(openUrl).not.toHaveBeenCalled();
    },
  );

  it('leaves a middle click alone, which is "open in a new tab" everywhere else', () => {
    render(<Markdown>{'[docs](https://example.com/a)'}</Markdown>);
    expect(fireEvent.click(anchor()!, { button: 1 })).toBe(true);
    expect(openUrl).not.toHaveBeenCalled();
  });
});

describe('Markdown: which hrefs are links at all', () => {
  // The scheme check used to sit in the click handler, which meant a refused href still arrived
  // as an `<a href="…">` and its click fell through to the browser's default action -- the very
  // navigation the guard was written against. These cases fail in that shape.
  it.each([
    ['a path into the project', './notes.md'],
    ['a local file', 'file:///C:/Windows/win.ini'],
    ['a script', 'javascript:alert(1)'],
  ])('renders %s as the text it is, not as a link', (_label, href) => {
    render(<Markdown>{`[docs](${href})`}</Markdown>);
    expect(anchor()).toBeNull();
    expect(screen.getByText('docs')).toBeInTheDocument();
    expect(openUrl).not.toHaveBeenCalled();
  });

  it('keeps the prose around a refused link intact', () => {
    const { container } = render(<Markdown>{`see [docs](./notes.md) for the wiring`}</Markdown>);
    // Refusing the href must not eat the word it was written on: the sentence is the reason the
    // agent wrote it, and a gap where the link was reads as a rendering failure.
    expect(container.textContent).toBe('see docs for the wiring');
    expect(anchor()).toBeNull();
  });
});
