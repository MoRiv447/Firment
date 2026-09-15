import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Pinmap } from '../Pinmap';
import type { BoardPinmapDto } from '../../../types';

/**
 * The pin table.
 *
 * Two things here are easy to get wrong and are what these tests are for: a
 * failed claim must not eat the pin number the user read off a schematic, and a
 * claim must be issued against the **selected** board rather than against
 * whatever the free-text field last held.
 */

const boards: BoardPinmapDto[] = [
  {
    board: 's3-node-1',
    pins: [
      { pin: 'PA5', func: 'LED', owner: 'user' },
      { pin: 'PB6', func: 'USART1_TX', owner: '' },
    ],
  },
  { board: 's3-node-2', pins: [] },
];

function setup(over: Partial<Parameters<typeof Pinmap>[0]> = {}) {
  const onSelectBoard = vi.fn();
  const onClaimPin = vi.fn().mockResolvedValue(true);
  const onRemovePin = vi.fn();
  const view = render(
    <Pinmap
      boards={boards}
      selected={null}
      busy={false}
      onSelectBoard={onSelectBoard}
      onClaimPin={onClaimPin}
      onRemovePin={onRemovePin}
      {...over}
    />,
  );
  const pinField = () => screen.getByPlaceholderText('pin (PA5)') as HTMLInputElement;
  const funcField = () => screen.getByPlaceholderText(/function/) as HTMLInputElement;
  return { onSelectBoard, onClaimPin, onRemovePin, view, pinField, funcField };
}

describe('Pinmap', () => {
  it('explains itself when there is nothing yet', () => {
    setup({ boards: [] });
    expect(screen.getByText(/No boards\/pins yet/)).toBeInTheDocument();
  });

  it('shows every board with its pin count', () => {
    setup();
    expect(screen.getByRole('button', { name: 's3-node-1 (2)' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 's3-node-2 (0)' })).toBeInTheDocument();
  });

  it('selects a board by clicking it', () => {
    const { onSelectBoard } = setup();
    fireEvent.click(screen.getByRole('button', { name: 's3-node-1 (2)' }));
    expect(onSelectBoard).toHaveBeenCalledWith('s3-node-1');
  });

  it('marks which board is selected without relying on colour alone', () => {
    setup({ selected: 's3-node-1' });
    expect(screen.getByRole('button', { name: 's3-node-1 (2)' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    expect(screen.getByRole('button', { name: 's3-node-2 (0)' })).toHaveAttribute(
      'aria-pressed',
      'false',
    );
  });

  it('accepts a board name typed by hand', () => {
    const { onSelectBoard } = setup({ boards: [] });
    const field = screen.getByPlaceholderText('new board name');
    fireEvent.change(field, { target: { value: '  s3-node-9  ' } });
    fireEvent.click(screen.getByRole('button', { name: 'use board' }));
    expect(onSelectBoard).toHaveBeenCalledWith('s3-node-9');
    expect((field as HTMLInputElement).value).toBe('');
  });

  it('will not select an empty board name', () => {
    const { onSelectBoard } = setup({ boards: [] });
    fireEvent.change(screen.getByPlaceholderText('new board name'), { target: { value: '   ' } });
    expect(screen.getByRole('button', { name: 'use board' })).toBeDisabled();
    fireEvent.keyDown(screen.getByPlaceholderText('new board name'), { key: 'Enter' });
    expect(onSelectBoard).not.toHaveBeenCalled();
  });

  it('lists the claimed pins of the selected board', () => {
    setup({ selected: 's3-node-1' });
    expect(screen.getByText('PA5')).toBeInTheDocument();
    expect(screen.getByText('LED')).toBeInTheDocument();
    expect(screen.getByText('user')).toBeInTheDocument();
    expect(screen.getByText('USART1_TX')).toBeInTheDocument();
  });

  it('shows a dash where nobody claimed ownership', () => {
    setup({ selected: 's3-node-1' });
    // An empty owner is a fact about the table, not a missing value to hide.
    expect(screen.getByText('—')).toBeInTheDocument();
  });

  it('says so when the selected board has no pins', () => {
    setup({ selected: 's3-node-2' });
    expect(screen.getByText(/has no claimed pins yet/)).toBeInTheDocument();
  });

  it('claims against the selected board, and clears once it landed', async () => {
    const { onClaimPin, pinField, funcField } = setup({ selected: 's3-node-1' });
    fireEvent.change(pinField(), { target: { value: ' PA7 ' } });
    fireEvent.change(funcField(), { target: { value: ' TIM3_CH2 ' } });
    fireEvent.click(screen.getByRole('button', { name: /claim on s3-node-1/ }));

    await waitFor(() => expect(onClaimPin).toHaveBeenCalledWith('PA7', 'TIM3_CH2'));
    await waitFor(() => expect(pinField().value).toBe(''));
    expect(funcField().value).toBe('');
  });

  it('keeps the pin and the function when the claim failed', async () => {
    // The expensive part is reading the pin number off a schematic.
    const { pinField, funcField } = setup({
      selected: 's3-node-1',
      onClaimPin: vi.fn().mockResolvedValue(false),
    });
    fireEvent.change(pinField(), { target: { value: 'PA7' } });
    fireEvent.change(funcField(), { target: { value: 'TIM3_CH2' } });
    fireEvent.click(screen.getByRole('button', { name: /claim on s3-node-1/ }));

    await waitFor(() => expect(funcField().value).toBe('TIM3_CH2'));
    expect(pinField().value).toBe('PA7');
  });

  it('needs both halves before it will claim', () => {
    const { pinField } = setup({ selected: 's3-node-1' });
    expect(screen.getByRole('button', { name: /claim on/ })).toBeDisabled();
    fireEvent.change(pinField(), { target: { value: 'PA7' } });
    // A pin with no function is not a claim.
    expect(screen.getByRole('button', { name: /claim on/ })).toBeDisabled();
  });

  it('will not queue a second claim from Enter while one is in flight', () => {
    const { onClaimPin, pinField, funcField } = setup({ selected: 's3-node-1', busy: true });
    fireEvent.change(pinField(), { target: { value: 'PA7' } });
    fireEvent.change(funcField(), { target: { value: 'TIM3_CH2' } });
    fireEvent.keyDown(funcField(), { key: 'Enter' });
    expect(onClaimPin).not.toHaveBeenCalled();
  });

  it('releases a pin by name', () => {
    const { onRemovePin } = setup({ selected: 's3-node-1' });
    fireEvent.click(screen.getByRole('button', { name: 'Release pin: PA5' }));
    expect(onRemovePin).toHaveBeenCalledWith('PA5');
  });

  it('will not release while a write is in flight', () => {
    setup({ selected: 's3-node-1', busy: true });
    expect(screen.getByRole('button', { name: 'Release pin: PA5' })).toBeDisabled();
  });
});
