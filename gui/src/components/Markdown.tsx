import { openUrl } from '@tauri-apps/plugin-opener';
import type { AnchorHTMLAttributes, HTMLAttributes, ReactNode } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { Components } from 'react-markdown';

import styles from './Markdown.module.css';

/**
 * The markdown the agent writes, rendered as the transcript's prose.
 *
 * Extracted from `MessageList.tsx`, where it lived as a `components` map with an
 * inline style on every element -- 37 of them in that file. The styles are a
 * stylesheet now, and the map survives because react-markdown cannot put a class
 * on an element it invented.
 *
 * GFM is what makes pipe tables render as tables: react-markdown does not support
 * them by default, so a finished table used to collapse into one long line of raw
 * pipes in the middle of a reply.
 */

/** The schemes the window is allowed to hand to the OS, and the only ones the
 *  opener plugin is permitted to open (`opener:default`). */
const OPENABLE = /^(?:https?:|mailto:|tel:)/i;

/**
 * A link leaves the webview rather than navigating inside it.
 *
 * An ordinary `<a>` in a Tauri window replaces the app's own document -- the one
 * mistake in this file's history that no test caught, because jsdom happily
 * "navigates" a detached window. The scheme is checked before the URL is handed
 * over: `javascript:` and a relative path are not things to open externally, and
 * a link is agent-authored text, so nothing here trusts its shape.
 */
function Link({ href, children, ...rest }: AnchorHTMLAttributes<HTMLAnchorElement>) {
  if (!href || !OPENABLE.test(href)) {
    // Then it is not a link at all, because this app has nowhere for it to go. A relative path
    // or a `file:` URL replaces the app's own document, and a `javascript:` one runs inside the
    // webview -- the same document that holds the IPC bridge, so text the model wrote would be
    // handing out commands. Declining to *open* such an href while still rendering the anchor
    // left the click to the browser's default action and did the damage anyway, which is why
    // the check moved from the handler to the element.
    return <span>{children}</span>;
  }
  return (
    <a
      {...rest}
      href={href}
      className={styles.link}
      onClick={(event) => {
        // A modified or non-primary click is the reader asking for something other than "go
        // there" -- new tab, new window, copy link -- and this handler cannot serve any of
        // those, so the only honest move is to get out of the way. Taking them over would make
        // `Ctrl+click` open the URL inside the app's own window: the one thing this component
        // exists to prevent.
        if (
          event.ctrlKey ||
          event.metaKey ||
          event.shiftKey ||
          event.altKey ||
          event.button !== 0
        ) {
          return;
        }
        event.preventDefault();
        void openUrl(href).catch(() => {
          // No host (a browser tab in development): let the anchor do what an
          // anchor does.
          window.location.href = href;
        });
      }}
    >
      {children}
    </a>
  );
}

/** Fenced blocks arrive as `pre > code` with a `language-*` class; only inline
 *  code gets the pill, because the block already has a box around it. */
function Code({ className, children }: HTMLAttributes<HTMLElement> & { children?: ReactNode }) {
  if (className?.startsWith('language-')) return <code>{children}</code>;
  return <code className={styles.inlineCode}>{children}</code>;
}

const COMPONENTS: Components = {
  p: ({ children }) => <p className={styles.p}>{children}</p>,
  ul: ({ children }) => <ul className={styles.list}>{children}</ul>,
  ol: ({ children }) => <ol className={styles.list}>{children}</ol>,
  li: ({ children }) => <li className={styles.item}>{children}</li>,
  pre: ({ children }) => <pre className={styles.pre}>{children}</pre>,
  code: Code,
  a: Link,
  h1: ({ children }) => <h1 className={styles.h1}>{children}</h1>,
  h2: ({ children }) => <h2 className={styles.h2}>{children}</h2>,
  h3: ({ children }) => <h3 className={styles.h3}>{children}</h3>,
  blockquote: ({ children }) => <blockquote className={styles.quote}>{children}</blockquote>,
  hr: () => <hr className={styles.rule} />,
  table: ({ children }) => (
    <div className={styles.tableWrap}>
      <table className={styles.table}>{children}</table>
    </div>
  ),
  th: ({ children }) => <th className={styles.th}>{children}</th>,
  td: ({ children }) => <td className={styles.td}>{children}</td>,
};

const REMARK_PLUGINS = [remarkGfm];

export function Markdown({ children }: { children: string }) {
  return (
    <div data-ui="markdown" className={styles.root}>
      <ReactMarkdown remarkPlugins={REMARK_PLUGINS} components={COMPONENTS}>
        {children}
      </ReactMarkdown>
    </div>
  );
}
