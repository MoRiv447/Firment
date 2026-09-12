/**
 * The actions an edit offers, from the workspace mockup's "IN CONTEXT" panel.
 *
 * These send a **request to the agent**, they do not run anything themselves.
 * That is the product's model -- the agent owns the toolchain, and a build has
 * to be visible in the transcript with its own card -- and it is why the prompts
 * are written as instructions rather than as commands.
 *
 * The mockup's third action, "View diff ›", is deliberately absent: the diff is
 * already rendered inline directly above the row, so a control that reveals it
 * would open something already open. The slant signature is not worth a button
 * that does nothing.
 */

export interface QuickAction {
  /** Stable key, for tests and for keying the row. */
  key: 'build' | 'test';
  label: string;
  tier: 'primary' | 'secondary';
  /** What gets sent. Kept in one place so the label and the request cannot drift. */
  prompt: string;
}

const BUILD: QuickAction = {
  key: 'build',
  label: 'Build & flash',
  // The device is named as "the board" rather than by chip: this action must not
  // repeat the target, because the agent reads the configured one and a prompt
  // that disagreed with it would be the worse kind of specific.
  prompt: 'Build this change and flash it to the board.',
  tier: 'primary',
};

const TEST: QuickAction = {
  key: 'test',
  label: 'Run tests',
  prompt: 'Run the tests that cover this change.',
  tier: 'secondary',
};

/**
 * The actions for a tool card, empty for everything that is not an edit.
 *
 * Only edits: a build button under a `read_file` card offers to re-do work that
 * has not been done, and a row that appears everywhere stops meaning anything.
 */
export function quickActionsFor(toolName: string): QuickAction[] {
  switch (toolName) {
    case 'edit_file':
    case 'write_file':
    case 'write':
    case 'edit':
      return [BUILD, TEST];
    default:
      return [];
  }
}
