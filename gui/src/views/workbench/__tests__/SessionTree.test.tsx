import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { SessionTree } from '../SessionTree';
import type { SessionSummaryDto } from '../../../types';

/**
 * The session tree.
 *
 * The filter is the part with behaviour in it: it decides which rows exist *and*
 * whether the "new mainline" offer appears, and those are different questions --
 * an empty filtered view is an answer, an empty project is a prompt.
 */

const sessions: SessionSummaryDto[] = [
  {
    id: 'aaaaaaaa-1111',
    updated_at: 0,
    model: 'm',
    cwd: '/w',
    preview: 'sensor drift hunt',
    kind: 'mainline',
    parent_session: null,
  },
  {
    id: 'bbbbbbbb-2222',
    updated_at: 0,
    model: 'm',
    cwd: '/w',
    preview: 'pwm breathing',
    kind: 'branch',
    parent_session: 'aaaaaaaa-1111',
  },
  {
    id: 'cccccccc-3333',
    updated_at: 0,
    model: 'm',
    cwd: '/w',
    preview: '',
    kind: 'normal',
    parent_session: null,
  },
];

function setup(over: Partial<Parameters<typeof SessionTree>[0]> = {}) {
  const handlers = {
    onReload: vi.fn(),
    onNewMainline: vi.fn(),
    onSetMainline: vi.fn(),
    onOpen: vi.fn(),
    onBranch: vi.fn(),
  };
  const view = render(
    <SessionTree
      sessions={sessions}
      mainlineSession="aaaaaaaa-1111"
      currentId={null}
      busy={false}
      {...handlers}
      {...over}
    />,
  );
  const filter = (name: string) => screen.getByRole('radio', { name });
  return { ...handlers, view, filter };
}

describe('SessionTree', () => {
  it('starts on every kind', () => {
    setup();
    expect(screen.getByRole('radio', { name: 'ALL' })).toBeChecked();
    expect(screen.getByText('sensor drift hunt')).toBeInTheDocument();
    expect(screen.getByText('pwm breathing')).toBeInTheDocument();
  });

  it('narrows to one kind', () => {
    const { filter } = setup();
    fireEvent.click(filter('BRANCH'));
    expect(screen.getByText('pwm breathing')).toBeInTheDocument();
    expect(screen.queryByText('sensor drift hunt')).toBeNull();
  });

  it('labels the mainline as such whatever its kind field says', () => {
    setup();
    // 'mainline' is also a filter label, so this asks the row, not the page. The
    // capitals are a CSS rule now (`data-upper`), and jsdom does not apply
    // `text-transform` -- so the text asserted here is the source text, and the
    // attribute is what proves the rule is on.
    const mainline = screen.getByText('sensor drift hunt').closest('[data-kind]');
    expect(mainline).toHaveTextContent('mainline');
    expect(mainline!.querySelector('[data-upper]')).not.toBeNull();
    const branch = screen.getByText('pwm breathing').closest('[data-kind]');
    expect(branch).toHaveTextContent('branch');
  });

  it('falls back to the id when a session has no preview yet', () => {
    setup();
    expect(screen.getByText('cccccccc')).toBeInTheDocument();
  });

  it('says which session a branch came from', () => {
    setup();
    expect(screen.getByText('of aaaaaaaa')).toBeInTheDocument();
  });

  it('marks the session being viewed', () => {
    setup({ currentId: 'bbbbbbbb-2222' });
    const row = screen.getByText('pwm breathing').closest('[data-current]');
    expect(row).toHaveAttribute('data-current', 'true');
  });

  it('offers to promote a session that is not the mainline', () => {
    const { onSetMainline } = setup();
    // The mainline row has nothing to promote.
    expect(screen.getAllByRole('button', { name: 'set mainline' })).toHaveLength(2);
    fireEvent.click(screen.getAllByRole('button', { name: 'set mainline' })[0]);
    expect(onSetMainline).toHaveBeenCalledWith('bbbbbbbb-2222');
  });

  it('opens a session and branches from one', () => {
    const { onOpen, onBranch } = setup();
    fireEvent.click(screen.getAllByRole('button', { name: 'open' })[0]);
    expect(onOpen).toHaveBeenCalledWith('aaaaaaaa-1111');
    fireEvent.click(screen.getByRole('button', { name: 'Branch from sensor drift hunt' }));
    expect(onBranch).toHaveBeenCalledWith('aaaaaaaa-1111');
  });

  it('reloads from disk', () => {
    const { onReload } = setup();
    fireEvent.click(screen.getByRole('button', { name: 'Reload sessions from disk' }));
    expect(onReload).toHaveBeenCalledTimes(1);
  });

  it('will not reload while a write is in flight', () => {
    // A second render in the same test would leave two trees on screen and the
    // first one's button enabled -- which is how this assertion was wrong once.
    setup({ busy: true });
    expect(screen.getByRole('button', { name: 'Reload sessions from disk' })).toBeDisabled();
  });

  it('offers to start a mainline when the project is empty', () => {
    const { onNewMainline } = setup({ sessions: [] });
    fireEvent.click(screen.getByRole('button', { name: 'New mainline chat here' }));
    expect(onNewMainline).toHaveBeenCalledTimes(1);
  });

  it('does NOT offer it when the emptiness is only the filter', () => {
    // "No sessions of this kind" is an answer, not a prompt to create one.
    setup({ sessions: sessions.filter((s) => s.kind !== 'branch') });
    fireEvent.click(screen.getByRole('radio', { name: 'BRANCH' }));
    expect(screen.getByText('No sessions under this path yet')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'New mainline chat here' })).toBeNull();
  });

  it('will not act on a row while a write is in flight', () => {
    setup({ busy: true });
    expect(screen.getAllByRole('button', { name: 'open' })[0]).toBeDisabled();
    expect(screen.getAllByRole('button', { name: 'set mainline' })[0]).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Branch from sensor drift hunt' })).toBeDisabled();
  });
});
