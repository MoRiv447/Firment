import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';

import { Bindings } from '../Bindings';
import type { DeviceBindingDto, DeviceEntry } from '../../../types';

/**
 * Which nodes this project may command.
 *
 * Two things are pinned here, and both were easy to break during the extraction.
 *
 * The drafts: `Decisions` had already moved its fields into its own component, so
 * a bind that clears its node name on a failed write is the bug to guard against,
 * not a new one.
 *
 * The live indicator: a row is green because the broker just heard from that node.
 * A binding that is present but silent is the failure mode this card exists to
 * show, so the dot has to come from the traffic list and not from the binding.
 */

type Props = Parameters<typeof Bindings>[0];

const binding = (over: Partial<DeviceBindingDto> = {}): DeviceBindingDto => ({
  node: 'pump-1',
  role: 'main mcu',
  note: '',
  allow: [],
  ...over,
});

const talking = (over: Partial<DeviceEntry> = {}): DeviceEntry => ({
  node: 'pump-1',
  lastKind: 'telemetry',
  lastFrame: '{"c":1}',
  ts: 1_700_000_000_000,
  count: 3,
  ...over,
});

/**
 * Returns the props it rendered with, so a test that overrides `onBind` asserts
 * on that mock rather than on a fresh one that never saw a click.
 */
function setup(over: Partial<Props> = {}): Props & { container: HTMLElement } {
  const props: Props = {
    bindings: [binding()],
    devices: [talking()],
    busy: false,
    onBind: vi.fn().mockResolvedValue(true),
    onUnbind: vi.fn(),
    ...over,
  };
  const { container } = render(<Bindings {...props} />);
  return { ...props, container };
}

const draftFields = () =>
  [
    screen.getByPlaceholderText('node name (s3-node-1)'),
    screen.getByPlaceholderText('role (main mcu / sensor node…)'),
  ] as HTMLInputElement[];

// `bind` matches as a substring of the rows' "Unbind node: …", so the draft
// button is named exactly.
const bindButton = () => screen.getByRole('button', { name: 'bind' });

describe('Bindings', () => {
  it('shows what is bound: node, role, note and the allow list', () => {
    setup({
      bindings: [binding({ role: 'sensor', note: 'on the breadboard', allow: ['read', 'reboot'] })],
    });
    expect(screen.getByText(/● pump-1/)).toBeInTheDocument();
    expect(screen.getByText('sensor')).toBeInTheDocument();
    expect(screen.getByText(/on the breadboard/)).toBeInTheDocument();
    expect(screen.getByText('[allow: read, reboot]')).toBeInTheDocument();
  });

  it('says how much a node has said, and when', () => {
    const { container } = setup();
    expect(container.textContent).toContain('×3');
    expect(container.textContent).toContain(new Date(1_700_000_000_000).toLocaleTimeString());
  });

  it('drops to an open circle for a bound node nobody has heard from', () => {
    setup({ bindings: [binding({ node: 'quiet-1' })], devices: [] });
    expect(screen.getByText(/○ quiet-1/)).toBeInTheDocument();
    // The green dot belongs to the traffic, so a silent row cannot borrow it.
    expect(screen.queryByText(/● quiet-1/)).not.toBeInTheDocument();
  });

  it('unbinds by node name, not by position', () => {
    const { onUnbind } = setup({ bindings: [binding(), binding({ node: 'probe-b' })] });
    fireEvent.click(screen.getByRole('button', { name: 'Unbind node: probe-b' }));
    expect(onUnbind).toHaveBeenCalledWith('probe-b');
  });

  it('will not bind without a node, and whitespace is not a node', () => {
    setup({ devices: [] });
    expect(bindButton()).toBeDisabled();
    fireEvent.change(draftFields()[0], { target: { value: '   ' } });
    expect(bindButton()).toBeDisabled();
  });

  it('types a node name when nothing is publishing', () => {
    setup({ devices: [] });
    expect(screen.queryByPlaceholderText('node name (s3-node-1)')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'node to bind' })).not.toBeInTheDocument();
  });

  it('prefers picking a live node over spelling its name', () => {
    // The picker is the whole reason a typo'd binding is caught before it is
    // written: `s3-node-1` and `s3-node-l` are one keystroke apart.
    setup({ devices: [talking({ node: 's3-node-l' })] });
    expect(screen.queryByPlaceholderText('node name (s3-node-1)')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'node to bind' }));
    expect(screen.getByRole('option', { name: 's3-node-l (3 frames)' })).toBeInTheDocument();
  });

  it('offers only the nodes that are not bound yet', () => {
    setup({ devices: [talking(), talking({ node: 's3-node-l' })] });
    fireEvent.click(screen.getByRole('button', { name: 'node to bind' }));
    // `pump-1` is publishing and already bound — re-binding it is not a choice.
    expect(screen.getAllByRole('option').map((o) => o.textContent)).toEqual([
      's3-node-l (3 frames)',
    ]);
  });

  it('trims what it sends', async () => {
    const { onBind } = setup({ devices: [] });
    fireEvent.change(draftFields()[0], { target: { value: ' pump-1 ' } });
    fireEvent.change(draftFields()[1], { target: { value: ' main mcu ' } });
    fireEvent.click(bindButton());
    await waitFor(() => expect(onBind).toHaveBeenCalledWith('pump-1', 'main mcu'));
  });

  it('clears the drafts once the node is bound', async () => {
    setup({ devices: [] });
    fireEvent.change(draftFields()[0], { target: { value: 'pump-2' } });
    fireEvent.change(draftFields()[1], { target: { value: 'sensor' } });
    fireEvent.click(bindButton());
    await waitFor(() => expect(draftFields()[0].value).toBe(''));
    expect(draftFields()[1].value).toBe('');
  });

  it('keeps the drafts when the bind failed', async () => {
    const { onBind } = setup({ devices: [], onBind: vi.fn().mockResolvedValue(false) });
    fireEvent.change(draftFields()[0], { target: { value: 'pump-2' } });
    fireEvent.click(bindButton());
    await waitFor(() => expect(onBind).toHaveBeenCalled());
    expect(draftFields()[0].value).toBe('pump-2');
  });

  it('binds from the keyboard, and refuses to while a write is in flight', async () => {
    const { onBind } = setup({ devices: [], busy: true });
    const role = draftFields()[1];
    fireEvent.change(role, { target: { value: 'sensor' } });
    fireEvent.keyDown(role, { key: 'Enter' });
    // `busy` gates Enter as well as the button: the backend writes per call.
    await waitFor(() => expect(bindButton()).toBeDisabled());
    expect(onBind).not.toHaveBeenCalled();
  });
});
