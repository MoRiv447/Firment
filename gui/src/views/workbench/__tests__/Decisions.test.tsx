import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Decisions } from '../Decisions';
import type { DecisionEntryDto } from '../../../types';

/**
 * The decision log.
 *
 * The behaviour worth testing here is not the markup -- it is what happens to
 * the two draft fields. The section was extracted with its drafts, which is a
 * change in where the state lives, so the failure mode to guard against is a
 * draft that clears itself when the record did not actually happen.
 */

const entries: DecisionEntryDto[] = [
  { date: '2026-08-30', title: 'I2C bus at 400k', body: 'the sensor can do it' },
  { date: '2026-09-01', title: 'HSE 8 MHz', body: '' },
];

function setup(over: Partial<Parameters<typeof Decisions>[0]> = {}) {
  const onAdd = vi.fn().mockResolvedValue(true);
  const onRemove = vi.fn();
  const view = render(
    <Decisions decisions={entries} busy={false} onAdd={onAdd} onRemove={onRemove} {...over} />,
  );
  const drafts = () => screen.getAllByRole('textbox') as HTMLInputElement[];
  return { onAdd, onRemove, view, drafts };
}

describe('Decisions', () => {
  it('shows the decision, its date and its rationale', () => {
    setup();
    expect(screen.getByText('I2C bus at 400k')).toBeInTheDocument();
    expect(screen.getByText('the sensor can do it')).toBeInTheDocument();
    expect(screen.getByText('2026-08-30')).toBeInTheDocument();
  });

  it('explains the log when it is empty', () => {
    setup({ decisions: [] });
    // The copy is what tells a reader that the agent writes the same list.
    expect(screen.getByText(/No decisions recorded/)).toBeInTheDocument();
  });

  it('submits the headline and the rationale together', async () => {
    const { onAdd, drafts } = setup({ decisions: [] });
    fireEvent.change(drafts()[0], { target: { value: 'I2C bus at 400k' } });
    fireEvent.change(drafts()[1], { target: { value: 'the sensor can do it' } });
    fireEvent.click(screen.getByRole('button', { name: /record/ }));

    await waitFor(() => expect(onAdd).toHaveBeenCalledTimes(1));
    expect(onAdd).toHaveBeenCalledWith('I2C bus at 400k', 'the sensor can do it');
  });

  it('clears the drafts once the decision is recorded', async () => {
    const { drafts } = setup({ decisions: [] });
    fireEvent.change(drafts()[0], { target: { value: 'I2C bus at 400k' } });
    fireEvent.click(screen.getByRole('button', { name: /record/ }));

    await waitFor(() => expect(drafts()[0].value).toBe(''));
  });

  it('keeps the drafts when the decision was NOT recorded', async () => {
    // The caller returns false when the write failed. Losing what the user
    // typed because the backend was down is the whole reason the drafts are
    // cleared on a signal rather than on the click.
    const { drafts } = setup({ decisions: [], onAdd: vi.fn().mockResolvedValue(false) });
    fireEvent.change(drafts()[0], { target: { value: 'I2C bus at 400k' } });
    fireEvent.change(drafts()[1], { target: { value: 'the sensor can do it' } });
    fireEvent.click(screen.getByRole('button', { name: /record/ }));

    await waitFor(() => expect(drafts()[1].value).toBe('the sensor can do it'));
    expect(drafts()[0].value).toBe('I2C bus at 400k');
  });

  it('will not record a decision with no headline', () => {
    setup({ decisions: [] });
    expect(screen.getByRole('button', { name: /record/ })).toBeDisabled();
    fireEvent.change(screen.getAllByRole('textbox')[0], { target: { value: '   ' } });
    // Whitespace is not a headline.
    expect(screen.getByRole('button', { name: /record/ })).toBeDisabled();
  });

  it('removes by rendered position, not by date', () => {
    const { onRemove } = setup();
    // The caller adds one, because the backend list is 1-based while this list
    // is not; the component must hand over what it rendered.
    fireEvent.click(screen.getByRole('button', { name: 'Remove decision: HSE 8 MHz' }));
    expect(onRemove).toHaveBeenCalledWith(1);
  });

  it('disables removal while a write is in flight', () => {
    setup({ busy: true });
    expect(screen.getByRole('button', { name: 'Remove decision: I2C bus at 400k' })).toBeDisabled();
  });
});
