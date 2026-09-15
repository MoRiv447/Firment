import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Hardware } from '../Hardware';
import type { HardwareInfoDto } from '../../../types';

/**
 * The hardware card.
 *
 * The behaviour worth pinning is the chip editor's: it is the one control here
 * that writes, it writes to *global* config rather than the project, and the
 * editor must only close when the backend said it landed. Everything else on the
 * card is a read-out.
 */

const info: HardwareInfoDto = {
  default_chip: 'stm32g431rb',
  probe_rs_available: true,
  serial_ports: ['COM4', 'COM7'],
  probes: ['ST-Link/V2-1 (0483:374b)'],
};

function setup(over: Partial<Parameters<typeof Hardware>[0]> = {}) {
  const onRefresh = vi.fn().mockResolvedValue(undefined);
  const onSaveChip = vi.fn().mockResolvedValue(true);
  const view = render(
    <Hardware hardware={info} busy={false} onRefresh={onRefresh} onSaveChip={onSaveChip} {...over} />,
  );
  const chipButton = () => screen.getByRole('button', { name: /^chip:/ });
  return { onRefresh, onSaveChip, view, chipButton };
}

describe('Hardware', () => {
  it('says what to do before anything is enumerated', () => {
    // Enumeration touches hardware, so it does not happen on mount -- the empty
    // state has to say how to get out of it.
    setup({ hardware: null });
    expect(screen.getByText(/Not loaded yet/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^chip:/ })).toBeNull();
  });

  it('shows the target chip, the ports and the probes', () => {
    setup();
    expect(screen.getByRole('button', { name: /stm32g431rb/ })).toBeInTheDocument();
    expect(screen.getByText('COM4')).toBeInTheDocument();
    expect(screen.getByText('COM7')).toBeInTheDocument();
    expect(screen.getByText(/ST-Link/)).toBeInTheDocument();
  });

  it('says so when there are no ports, rather than showing nothing', () => {
    setup({ hardware: { ...info, serial_ports: [] } });
    expect(screen.getByText('No serial ports found.')).toBeInTheDocument();
  });

  it('reports probe-rs as missing when it is', () => {
    setup({ hardware: { ...info, probe_rs_available: false } });
    expect(screen.getByText(/not installed/)).toBeInTheDocument();
  });

  it('re-enumerates when refresh is clicked', async () => {
    const { onRefresh } = setup();
    fireEvent.click(screen.getByRole('button', { name: /refresh/ }));
    await waitFor(() => expect(onRefresh).toHaveBeenCalledTimes(1));
  });

  it('opens the editor seeded with the current chip', () => {
    const { chipButton } = setup();
    fireEvent.click(chipButton());
    expect((screen.getByRole('textbox') as HTMLInputElement).value).toBe('stm32g431rb');
  });

  it('saves the trimmed chip and closes once it landed', async () => {
    const { onSaveChip, chipButton } = setup();
    fireEvent.click(chipButton());
    fireEvent.change(screen.getByRole('textbox'), { target: { value: '  stm32f407vetx  ' } });
    fireEvent.click(screen.getByRole('button', { name: 'save' }));

    await waitFor(() => expect(onSaveChip).toHaveBeenCalledWith('stm32f407vetx'));
    await waitFor(() => expect(screen.queryByRole('textbox')).toBeNull());
  });

  it('keeps the editor and the draft when the global write failed', async () => {
    // The write goes to global config. Closing on a failed write would leave the
    // card reading "(unset)" while the user believes they set a chip.
    const { chipButton } = setup({ onSaveChip: vi.fn().mockResolvedValue(false) });
    fireEvent.click(chipButton());
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'stm32f407vetx' } });
    fireEvent.click(screen.getByRole('button', { name: 'save' }));

    await waitFor(() => expect(screen.getByRole('textbox')).toBeInTheDocument());
    expect((screen.getByRole('textbox') as HTMLInputElement).value).toBe('stm32f407vetx');
  });

  it('saves from Enter', async () => {
    const { onSaveChip, chipButton } = setup();
    fireEvent.click(chipButton());
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'stm32g0b1re' } });
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' });
    await waitFor(() => expect(onSaveChip).toHaveBeenCalledWith('stm32g0b1re'));
  });

  it('an empty draft is a real value — it unsets the chip', async () => {
    const { onSaveChip, chipButton } = setup();
    fireEvent.click(chipButton());
    fireEvent.change(screen.getByRole('textbox'), { target: { value: '  ' } });
    fireEvent.click(screen.getByRole('button', { name: 'save' }));
    // Not "disabled when empty": clearing the field is how the chip is unset.
    await waitFor(() => expect(onSaveChip).toHaveBeenCalledWith(''));
  });

  it('will not queue a second write from Enter while one is in flight', () => {
    const { onSaveChip, chipButton } = setup({ busy: true });
    fireEvent.click(chipButton());
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' });
    expect(onSaveChip).not.toHaveBeenCalled();
  });

  it('abandons the edit without writing anything', () => {
    const { onSaveChip, chipButton } = setup();
    fireEvent.click(chipButton());
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'nonsense' } });
    fireEvent.click(screen.getByRole('button', { name: 'cancel' }));
    expect(screen.queryByRole('textbox')).toBeNull();
    expect(onSaveChip).not.toHaveBeenCalled();
  });
});
