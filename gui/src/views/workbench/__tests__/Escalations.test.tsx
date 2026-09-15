import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';

import { Escalations } from '../Escalations';
import type { EscalationEntry } from '../../../types';

/**
 * What the guard decided is worth a diagnosis.
 *
 * The card owns no state, so what is pinned here is its two judgements: whether a
 * finding can be acted on at all (no mainline, no diagnosis) and how loudly each
 * row is allowed to look. Both used to be spread across the 1500-line parent, and
 * the one that mattered most -- the diagnose button staying dead without a
 * mainline session -- was checkable only by reading the handler.
 */

type Props = Parameters<typeof Escalations>[0];

/** Mid-morning today: `formatStamp` should print it as a bare clock time. */
const at = new Date();
at.setHours(9, 5, 0, 0);

const entry = (over: Partial<EscalationEntry> = {}): EscalationEntry => ({
  id: 'pump-1-over_temp',
  ts: at.getTime(),
  node: 'pump-1',
  sev: 'warn',
  rule: 'over_temp',
  summary: 'probe 2 above 80C',
  payload: '{"sev":"warn","rule":"over_temp"}',
  ...over,
});

function setup(over: Partial<Props> = {}): Props & { container: HTMLElement } {
  const props: Props = {
    entries: [entry()],
    threshold: 'warn',
    mainline: 'mainline/session-1',
    busy: false,
    autoRun: false,
    onAutoRun: vi.fn(),
    onDiagnose: vi.fn(),
    onDismiss: vi.fn(),
    ...over,
  };
  const { container } = render(<Escalations {...props} />);
  return { ...props, container };
}

const switchControl = () => screen.getByRole('switch', { name: 'auto' });
const diagnoseButton = () => screen.getByRole('button', { name: 'diagnose' });

describe('Escalations', () => {
  it('says which sev the list is filtered at', () => {
    const { container } = setup({ threshold: 'error' });
    expect(container.textContent).toContain('sev ≥ error');
  });

  it('is quiet about nothing, but says where the findings come from', () => {
    const { container } = setup({ entries: [] });
    expect(container.textContent).toContain('No pending escalations');
    expect(screen.queryByRole('button', { name: 'diagnose' })).not.toBeInTheDocument();
  });

  it('leaves the auto switch unexplained while it is off', () => {
    const { container } = setup();
    expect(container.textContent).not.toMatch(/handed to the mainline session/);
  });

  it('explains what auto does before it does it', () => {
    // A new row starts an agent in a project that is not on screen. That is not
    // something a tooltip should be the only record of.
    setup({ autoRun: true });
    expect(screen.getByRole('button', { name: 'diagnose' })).toBeInTheDocument();
    expect(screen.getByText(/handed to the mainline session/)).toBeInTheDocument();
  });

  it('reports the switch the way a switch is read', () => {
    const { onAutoRun } = setup({ autoRun: false });
    expect(switchControl()).toHaveAttribute('aria-checked', 'false');
    fireEvent.click(switchControl());
    expect(onAutoRun).toHaveBeenCalledWith(true);
  });

  it('refuses to diagnose without a mainline, and says why', () => {
    const { container } = setup({ mainline: '' });
    expect(diagnoseButton()).toBeDisabled();
    expect(container.textContent).toContain('No mainline session registered');
  });

  it('goes dead while a write is in flight', () => {
    setup({ busy: true });
    expect(diagnoseButton()).toBeDisabled();
    expect(screen.getByRole('button', { name: /Dismiss escalation/ })).toBeDisabled();
  });

  it('hands diagnose the whole row and dismiss only its id', () => {
    const found = entry({ id: 'probe-b-9', node: 'probe-b', rule: 'watchdog' });
    const { onDiagnose, onDismiss } = setup({ entries: [found] });
    fireEvent.click(diagnoseButton());
    expect(onDiagnose).toHaveBeenCalledWith(found);
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss escalation: probe-b watchdog' }));
    expect(onDismiss).toHaveBeenCalledWith('probe-b-9');
  });

  it('ranks the row: only an error is painted like a failure', () => {
    setup({ entries: [entry({ id: 'a', sev: 'error' }), entry({ id: 'b', sev: 'info' })] });
    const status = (text: string) =>
      screen.getByText(text).closest('[data-ui="chip"]')?.getAttribute('data-status');
    expect(status('error')).toBe('failed');
    expect(status('info')).toBe('attention');
  });

  it('reads the stamp as seconds, and names the rule that fired', () => {
    const { container } = setup();
    expect(container.textContent).toContain('09:05');
    expect(container.textContent).toContain('over_temp');
  });

  it('still says something when a row carries no rule', () => {
    const { container } = setup({ entries: [entry({ rule: '', summary: '' })] });
    expect(container.textContent).toContain('no rule');
    // With no summary the payload is the row's only prose. `title` is the one
    // thing that tells the summary line apart from the meta line below it.
    expect(screen.getByTitle('{"sev":"warn","rule":"over_temp"}')).toBeInTheDocument();
  });

  it('cuts a long payload short rather than wrapping the card', () => {
    const { container } = setup({ entries: [entry({ payload: 'x'.repeat(400) })] });
    expect(container.textContent).not.toContain(`${'x'.repeat(121)}`);
    expect(container.textContent).toContain('x'.repeat(120));
  });
});
