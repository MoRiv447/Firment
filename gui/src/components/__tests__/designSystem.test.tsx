import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { StepProgress } from '../StepProgress';
import { SlantButton, slantClip } from '../SlantButton';
import { paletteFor, setActivePalette } from '../../styles/tokens';

/**
 * The two components the design language is made of.
 *
 * Nothing here asserts pixel positions -- those are in the screenshot the
 * components were built from and would rot. What is asserted is the set of
 * rules that a future edit could break silently: which edge is cut, that the
 * label is never sheared with the shape, that the optical correction is
 * actually applied, and that an unmeasured step never renders as a failure.
 */

/** jsdom normalises inline colours to `rgb(r, g, b)`; compare in that space. */
function rgb(hex: string): string {
  const value = hex.replace('#', '');
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(value.slice(i, i + 2), 16));
  return `rgb(${r}, ${g}, ${b})`;
}

/** The last aria-hidden layer is the fill; the label is the other span. */
function fillLayer(button: HTMLElement): HTMLElement {
  const layers = button.querySelectorAll<HTMLElement>('span[aria-hidden]');
  return layers[layers.length - 1];
}

function labelSpan(button: HTMLElement): HTMLElement {
  const span = button.querySelector<HTMLElement>('span:not([aria-hidden])');
  if (!span) throw new Error('label span missing');
  return span;
}

describe('SlantButton', () => {
  // The colour assertions below read the *active* palette, so the scheme has to
  // be pinned before rendering: the components read `color.x` at render time,
  // exactly as the views do.
  beforeEach(() => setActivePalette('light'));

  describe('weight comes from colour, not size', () => {
    it('fills the primary tier with the brand acid and labels it in onAcid', () => {
      const light = paletteFor('light');
      render(
        <SlantButton tier="primary" onClick={() => {}}>
          Build &amp; flash
        </SlantButton>,
      );
      const button = screen.getByRole('button', { name: 'Build & flash' });
      expect(fillLayer(button).style.background).toBe(rgb(light.brandAcid));
      expect(labelSpan(button).style.color).toBe(rgb(light.onAcid));
    });

    it('outlines the secondary tier instead of filling it', () => {
      const light = paletteFor('light');
      render(<SlantButton tier="secondary">Run tests</SlantButton>);
      const button = screen.getByRole('button', { name: 'Run tests' });
      // jsdom does not expand the shorthand here, so check the fill layer is
      // the surface colour rather than asserting on `border`.
      expect(fillLayer(button).style.background).toBe(rgb(light.surface));
      expect(button.style.borderTopWidth).toBe('1px');
      expect(slantClip('none')).toBeUndefined();
      expect(fillLayer(button).style.clipPath).toBe('');
    });

    it('gives every tier the same height', () => {
      render(
        <div>
          <SlantButton tier="primary">a</SlantButton>
          <SlantButton tier="secondary">b</SlantButton>
          <SlantButton tier="tertiary">c</SlantButton>
        </div>,
      );
      const heights = ['a', 'b', 'c'].map(
        (name) => screen.getByRole('button', { name }).style.height,
      );
      // Same height on a row: colour carries the hierarchy, not scale.
      expect(new Set(heights).size).toBe(1);
      expect(heights[0]).toBe('40px');
    });

    it('renders the tertiary tier as a label and a chevron, with no chrome', () => {
      render(
        <SlantButton tier="tertiary" chevron>
          View diff
        </SlantButton>,
      );
      const button = screen.getByRole('button', { name: 'View diff ›' });
      expect(fillLayer(button).style.background).toBe('transparent');
      expect(button.style.borderTopWidth).toBe('1px');
    });
  });

  describe('the slant is a cut, never a shear', () => {
    it('clips the fill layer by the documented run', () => {
      const { container } = render(
        <SlantButton tier="primary" onClick={() => {}}>
          Go
        </SlantButton>,
      );
      const button = container.querySelector('button') as HTMLElement;
      expect(fillLayer(button).style.clipPath).toContain('12px');
      expect(fillLayer(button).style.clipPath).toContain('polygon(');
    });

    it('never transforms the label', () => {
      const { container } = render(
        <SlantButton tier="primary" onClick={() => {}}>
          Go
        </SlantButton>,
      );
      const button = container.querySelector('button') as HTMLElement;
      // tokens.md: a skewX would shear the type with the shape and Latin text
      // leaning is a decal, not a design.
      expect(labelSpan(button).style.transform).toBe('');
      expect(button.style.transform).toBe('');
    });

    it('adds the optical correction on the cut side only', () => {
      const light = paletteFor('light');
      render(
        <SlantButton tier="primary" edge="left" onClick={() => {}}>
          Go
        </SlantButton>,
      );
      const label = labelSpan(screen.getByRole('button', { name: 'Go' }));
      // The removed triangle sat on the left, so the label gets the padding
      // back or it reads as off-centre.
      expect(label.style.paddingLeft).toBe('5px');
      expect(light.brandAcid).toBe('#B4F779');
    });

    it('drops the correction when nothing is cut', () => {
      render(<SlantButton tier="primary" edge="none" onClick={() => {}}>Go</SlantButton>);
      expect(labelSpan(screen.getByRole('button', { name: 'Go' })).style.paddingLeft).toBe('0px');
    });

    it('keeps rounded corners off the cut shape so the diagonal survives', () => {
      render(
        <SlantButton tier="primary" onClick={() => {}}>
          Go
        </SlantButton>,
      );
      expect(screen.getByRole('button', { name: 'Go' }).style.borderRadius).toBe('0px');
    });
  });

  it('does not fire when disabled', () => {
    const onClick = vi.fn();
    render(
      <SlantButton tier="primary" disabled onClick={onClick}>
        Go
      </SlantButton>,
    );
    const button = screen.getByRole('button', { name: 'Go' });
    fireEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
    expect(button).toBeDisabled();
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
});

describe('slantClip', () => {
  it('cuts one edge by the documented run', () => {
    expect(slantClip('left')).toBe('polygon(0 0, 100% 0, 100% 100%, 12px 100%)');
  });

  it('cuts both edges by the same run for the parallelogram', () => {
    expect(slantClip('both')).toContain('calc(100% - 12px)');
  });

  it('draws no clip at all when the edge is square', () => {
    // Undefined rather than an identity polygon: a stray `polygon(0 0, 100% 0,
    // 100% 100%, 0 100%)` would still clip, and would take the focus ring with
    // it on some engines.
    expect(slantClip('none')).toBeUndefined();
  });
});
