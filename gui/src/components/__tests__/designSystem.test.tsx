import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { StepProgress } from '../StepProgress';
import { ActionButton } from '../ActionButton';
import { paletteFor, setActivePalette } from '../../styles/tokens';

/**
 * The two components the design language is made of.
 *
 * Nothing here asserts pixel positions -- those are in the screenshot the
 * components were built from and would rot. What is asserted is the set of
 * rules that a future edit could break silently: that the button tiers are a
 * `type` mapping carrying no chrome of their own, and that an unmeasured step
 * never renders as a failure.
 *
 * The `fillLayer`/`labelSpan` helpers left with `SlantButton`: they existed to
 * reach inside its two hand-drawn `clip-path` layers, and there are no such
 * layers to reach into now.
 */

/** jsdom normalises inline colours to `rgb(r, g, b)`; compare in that space. */
function rgb(hex: string): string {
  const value = hex.replace('#', '');
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(value.slice(i, i + 2), 16));
  return `rgb(${r}, ${g}, ${b})`;
}

describe('ActionButton', () => {
  // Weight is a `type` mapping now, not a rendering of its own.
  //
  // SlantButton drew two absolutely-positioned `clip-path` layers by hand, the
  // outer one filled with `color.outline` so the diagonal had an edge. That was
  // right while `outline` was pure black; once it became an ordinary border
  // grey, a primary button rendered as an acid fill inside a grey ring -- a fill
  // that looks outlined by mistake. These tests pin the replacement.
  describe('weight comes from colour, not size', () => {
    it('maps the three tiers onto antd button types', () => {
      render(
        <div>
          <ActionButton tier="primary">a</ActionButton>
          <ActionButton tier="secondary">b</ActionButton>
          <ActionButton tier="tertiary">c</ActionButton>
        </div>,
      );
      const cls = (name: string) => screen.getByRole('button', { name }).className;
      expect(cls('a')).toContain('ant-btn-primary');
      expect(cls('b')).not.toContain('ant-btn-primary');
      expect(cls('b')).not.toContain('ant-btn-text');
      expect(cls('c')).toContain('ant-btn-text');
    });

    it('gives every tier the same height', () => {
      render(
        <div>
          <ActionButton tier="primary">a</ActionButton>
          <ActionButton tier="secondary">b</ActionButton>
          <ActionButton tier="tertiary">c</ActionButton>
        </div>,
      );
      const cls = (name: string) => screen.getByRole('button', { name }).className;
      // antd sizes all three from one control height, so a row reads as one row
      // rather than as a big button next to two small ones.
      expect(cls('a')).toContain('ant-btn');
      expect(cls('b')).toContain('ant-btn');
      expect(cls('c')).toContain('ant-btn');
      for (const name of ['a', 'b', 'c']) {
        expect(screen.getByRole('button', { name }).style.height).toBe('');
      }
    });

    it('leaves the primary tier unfilled by any inline chrome', () => {
      // The regression in one assertion: no inline border, no inline shadow and
      // no hand-drawn layer behind the label. antd paints the fill from
      // `colorPrimary` and labels it from `Button.primaryColor`; anything added
      // here would be a second source for a colour the theme already owns.
      render(<ActionButton tier="primary">Go</ActionButton>);
      const button = screen.getByRole('button', { name: 'Go' });
      expect(button.style.border).toBe('');
      expect(button.style.borderWidth).toBe('');
      expect(button.style.boxShadow).toBe('');
      expect(button.style.background).toBe('');
      expect(button.querySelectorAll('span[aria-hidden]')).toHaveLength(0);
    });

    it('renders the tertiary tier as a label and a chevron, with no chrome', () => {
      render(
        <ActionButton tier="tertiary" chevron>
          View diff
        </ActionButton>,
      );
      const button = screen.getByRole('button', { name: /View diff/ });
      expect(button).toHaveClass('ant-btn-text');
      expect(button.style.border).toBe('');
      expect(button.textContent).toContain('›');
    });
  });

  it('does not fire when disabled', () => {
    const onClick = vi.fn();
    render(
      <ActionButton tier="primary" disabled onClick={onClick}>
        Go
      </ActionButton>,
    );
    const button = screen.getByRole('button', { name: 'Go' });
    fireEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
    expect(button).toBeDisabled();
  });

  it('does not fire while loading, and says so', () => {
    const onClick = vi.fn();
    render(
      <ActionButton tier="primary" loading onClick={onClick}>
        Build
      </ActionButton>,
    );
    const button = screen.getByRole('button', { name: /Build/ });
    fireEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
    expect(button).toBeDisabled();
    // The spinner replaces the icon, so the label keeps its meaning and the
    // button does not silently look idle while work is in flight.
    expect(button.querySelector('.anticon-loading')).not.toBeNull();
  });

  it('takes a leading icon', () => {
    render(
      <ActionButton tier="primary" icon={<span data-testid="lead" />} onClick={() => {}}>
        Send
      </ActionButton>,
    );
    expect(screen.getByTestId('lead')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Send/ })).toBeInTheDocument();
  });
});

describe('StepProgress', () => {
  const steps = [
    { key: 'build', state: 'done' as const },
    { key: 'flash', state: 'current' as const },
    { key: 'monitor', state: 'pending' as const },
  ];

  it('marks the current step for assistive tech', () => {
    render(<StepProgress steps={steps} />);
    const current = screen.getByRole('listitem', { name: /flash/ });
    expect(current).toHaveAttribute('aria-current', 'step');
    expect(screen.getByRole('listitem', { name: /build/ })).not.toHaveAttribute('aria-current');
  });

  it('fills only the finished step', () => {
    setActivePalette('light');
    render(<StepProgress steps={steps} />);
    const light = paletteFor('light');
    expect(screen.getByRole('listitem', { name: /build/ }).style.background).toBe(
      rgb(light.stepDoneBg),
    );
    expect(screen.getByRole('listitem', { name: /flash/ }).style.background).toBe('transparent');
    expect(screen.getByRole('listitem', { name: /monitor/ }).style.background).toBe('transparent');
  });

  it('underlines the current step with the brand rule', () => {
    setActivePalette('light');
    render(<StepProgress steps={steps} />);
    const style = screen.getByRole('listitem', { name: /flash/ }).style;
    expect(style.borderBottomWidth).toBe('2px');
    expect(style.borderBottomColor).toBe(rgb(paletteFor('light').stepRule));
  });

  it('keeps a pending step legible rather than greying it to disabled', () => {
    setActivePalette('light');
    render(<StepProgress steps={steps} />);
    const pending = screen.getByRole('listitem', { name: /monitor/ }).style.color;
    expect(pending).toBe(rgb(paletteFor('light').stepPendingInk));
    // 4.51:1 -- readable "not yet", not "unavailable".
    expect(pending).not.toBe(rgb(paletteFor('light').muted));
  });

  it('is not a row of buttons', () => {
    render(<StepProgress steps={steps} />);
    // A progress row that looks pressable becomes a control that does nothing.
    expect(screen.queryAllByRole('button')).toHaveLength(0);
    for (const item of screen.getAllByRole('listitem')) {
      expect(item.style.cursor).toBe('default');
    }
  });

  it('renders an unknown step as o, never as a cross', () => {
    render(
      <StepProgress
        steps={[
          { key: 'build', state: 'done' },
          { key: 'flash', state: 'unknown' },
        ]}
      />,
    );
    const unknown = screen.getByRole('listitem', { name: /flash/ });
    // unknown is not failed: red means a real error happened.
    expect(unknown.textContent).toContain('○');
    expect(unknown.textContent).not.toContain('✗');
    expect(unknown.textContent).not.toContain('✕');
    expect(unknown.getAttribute('aria-label')).toContain('not yet measured');
  });

  it('keeps the label when a step carries an explicit one', () => {
    render(<StepProgress steps={[{ key: 'build', label: 'Build', state: 'done' }]} />);
    expect(screen.getByText('Build')).toBeInTheDocument();
  });

  it('fills a failed step, because that is a real error', () => {
    setActivePalette('light');
    render(<StepProgress steps={[{ key: 'flash', state: 'failed' }]} />);
    const light = paletteFor('light');
    const failed = screen.getByRole('listitem', { name: /flash/ });
    expect(failed.style.background).toBe(rgb(light.stepFailedBg));
    expect(failed.style.color).toBe(rgb(light.stepFailedInk));
    expect(failed.textContent).toContain('✕');
    expect(failed.getAttribute('aria-label')).toContain('failed');
  });

  it('keeps a failed step distinct from the brand and success greens', () => {
    setActivePalette('light');
    const light = paletteFor('light');
    // Three outcomes, three colours: a failure must not be able to read as
    // "passed", and it must not borrow the brand green either.
    expect(light.stepFailedInk).not.toBe(light.stepDoneInk);
    expect(light.stepFailedInk).not.toBe(light.brandAcid);
    expect(light.stepFailedBg).not.toBe(light.stepDoneBg);
  });
});

