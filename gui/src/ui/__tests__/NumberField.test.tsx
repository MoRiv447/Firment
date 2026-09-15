import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { NumberField } from '../NumberField';
import { Field } from '../Field';

/**
 * The numeric field.
 *
 * Every assertion here is one of the two rules the primitive exists to hold --
 * out-of-range clamps, empty is `undefined` -- or one of the re-sync behaviours
 * that a controlled number field gets wrong in the same three ways every time.
 */

function setup(over: Partial<Parameters<typeof NumberField>[0]> = {}) {
  const onValueChange = vi.fn();
  const view = render(
    <NumberField value={10} onValueChange={onValueChange} aria-label="budget" {...over} />,
  );
  const box = () => screen.getByRole('textbox', { name: 'budget' }) as HTMLInputElement;
  return { onValueChange, view, box };
}

describe('NumberField', () => {
  it('shows the number it was given', () => {
    setup();
    expect(screen.getByRole('textbox', { name: 'budget' })).toHaveValue('10');
  });

  it('shows nothing for an unset value', () => {
    setup({ value: undefined });
    expect(screen.getByRole('textbox', { name: 'budget' })).toHaveValue('');
  });

  it('reports the number as it is typed', () => {
    const { onValueChange, box } = setup();
    fireEvent.change(box(), { target: { value: '256' } });
    expect(onValueChange).toHaveBeenCalledWith(256);
  });

  it('reports undefined when it is emptied, not zero', () => {
    // "no limit" and "the limit is zero" are different settings.
    const { onValueChange, box } = setup();
    fireEvent.change(box(), { target: { value: '' } });
    expect(onValueChange).toHaveBeenCalledWith(undefined);
  });

  it('clamps above the maximum', () => {
    const { onValueChange, box } = setup({ max: 100 });
    fireEvent.change(box(), { target: { value: '99999' } });
    expect(onValueChange).toHaveBeenCalledWith(100);
  });

  it('clamps below the minimum', () => {
    const { onValueChange, box } = setup({ min: 1 });
    fireEvent.change(box(), { target: { value: '0' } });
    expect(onValueChange).toHaveBeenCalledWith(1);
  });

  it('reports nothing for text that is not a number', () => {
    const { onValueChange, box } = setup();
    fireEvent.change(box(), { target: { value: 'twelve' } });
    expect(onValueChange).toHaveBeenCalledWith(undefined);
  });

  it('lets a half-typed number stay half-typed', () => {
    // `-` and `1.` mean nothing yet. A field that parsed on every keystroke and
    // then rewrote the box from its value would delete them as they were typed.
    const { box } = setup({ value: undefined });
    fireEvent.change(box(), { target: { value: '-' } });
    expect(box().value).toBe('-');
    fireEvent.change(box(), { target: { value: '-12' } });
    expect(box().value).toBe('-12');
  });

  it('does not rewrite the box while you are still typing past the maximum', () => {
    // The caller clamps to 100, but the box must keep showing 999 until blur.
    const { view, box } = setup({ max: 100, value: 50 });
    fireEvent.change(box(), { target: { value: '999' } });
    expect(box().value).toBe('999');

    // The caller echoing the clamped value back must not touch the text either.
    view.rerender(
      <NumberField value={100} onValueChange={() => {}} max={100} aria-label="budget" />,
    );
    expect(box().value).toBe('999');
  });

  it('normalises to the clamped value once you leave the field', () => {
    const { view, box } = setup({ max: 100, value: 50 });
    fireEvent.change(box(), { target: { value: '999' } });
    view.rerender(
      <NumberField value={100} onValueChange={() => {}} max={100} aria-label="budget" />,
    );
    fireEvent.blur(box());
    expect(box().value).toBe('100');
  });

  it('follows a value that came from outside', () => {
    const { view, box } = setup({ value: 10 });
    view.rerender(
      <NumberField value={42} onValueChange={() => {}} aria-label="budget" />,
    );
    expect(box().value).toBe('42');
  });

  it('carries a unit as part of the field', () => {
    setup({ suffix: 'baud' });
    expect(screen.getByText('baud')).toBeInTheDocument();
  });

  it('takes the label, the hint and the error from the Field around it', () => {
    render(
      <Field label="Baud" hint="the rate the monitor opens at" error="too fast">
        <NumberField value={undefined} onValueChange={() => {}} max={100} />
      </Field>,
    );
    const box = screen.getByRole('textbox', { name: /Baud/ });
    // The error replaces the hint, so the control describes the error.
    expect(box).toHaveAttribute('aria-invalid', 'true');
    expect(box).toHaveAccessibleDescription('too fast');
  });
});
