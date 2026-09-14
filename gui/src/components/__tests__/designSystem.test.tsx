import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import stepSheetRaw from '../StepProgress.module.css?raw';
import { StepProgress } from '../StepProgress';
import { ActionButton } from '../ActionButton';
import { paletteFor, setActivePalette, slant } from '../../styles/tokens';

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
    it('draws only the primary tier by hand', () => {
      // The split is the point: antd owns every tier it can, and the primary is
      // built here only because a clipped fill cannot be a `border`.
      render(
        <div>
          <ActionButton tier="primary">a</ActionButton>
          <ActionButton tier="secondary">b</ActionButton>
          <ActionButton tier="tertiary">c</ActionButton>
        </div>,
      );
      const cls = (name: string) => screen.getByRole('button', { name }).className;
      const layers = (name: string) =>
        screen.getByRole('button', { name }).querySelectorAll('span[aria-hidden]').length;
      expect(cls('b')).toContain('ant-btn');
      expect(cls('c')).toContain('ant-btn-text');
      // primary is a plain <button> with two aria-hidden layers: edge + fill.
      expect(layers('a')).toBe(2);
      expect(layers('b')).toBe(0);
      expect(layers('c')).toBe(0);
    });

    it('gives every tier the same height', () => {
      render(
        <div>
          <ActionButton tier="primary">a</ActionButton>
          <ActionButton tier="secondary">b</ActionButton>
          <ActionButton tier="tertiary">c</ActionButton>
        </div>,
      );
      // Same height on a row: colour carries the hierarchy, not scale.
      expect(screen.getByRole('button', { name: 'a' }).style.height).toBe('40px');
      expect(screen.getByRole('button', { name: 'b' })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'c' })).toBeInTheDocument();
    });

    it('edges the primary tier in the brand edge, never in the generic border', () => {
      // This is the whole regression. The edge layer used to be filled with
      // `color.outline`, which was pure black until it became an ordinary
      // border grey -- at which point the primary button rendered as an acid
      // fill inside a grey ring, which reads as a mistake rather than a mark.
      for (const mode of ['dark', 'light'] as const) {
        setActivePalette(mode);
        const palette = paletteFor(mode);
        const { container, unmount } = render(<ActionButton tier="primary">Go</ActionButton>);
        const layers = container.querySelectorAll<HTMLElement>('span[aria-hidden]');
        expect(layers[0].style.background).toBe(rgb(palette.brandEdge));
        expect(layers[0].style.background).not.toBe(rgb(palette.outline));
        expect(layers[1].style.background).toBe(rgb(palette.brandAcid));
        unmount();
      }
      setActivePalette('light');
    });

    it('renders the tertiary tier as a label and a chevron, with no chrome', () => {
      render(
        <ActionButton tier="tertiary" chevron>
          View diff
        </ActionButton>,
      );
      const button = screen.getByRole('button', { name: /View diff/ });
      expect(button).toHaveClass('ant-btn-text');
      expect(button.textContent).toContain('›');
    });
  });

  describe('the slant is a cut, never a shear', () => {
    it('clips the fill by the documented run', () => {
      const { container } = render(<ActionButton tier="primary">Go</ActionButton>);
      const layers = container.querySelectorAll<HTMLElement>('span[aria-hidden]');
      expect(layers[1].style.clipPath).toContain('polygon(');
      expect(layers[1].style.clipPath).toContain(`${slant.cut}px`);
    });

    it('never transforms the label', () => {
      const { container } = render(<ActionButton tier="primary">Go</ActionButton>);
      const label = container.querySelector<HTMLElement>('button > span:not([aria-hidden])');
      // tokens.md: a `skewX` would shear the type with the shape, and Latin text
      // leaning is a decal, not a design.
      expect(label?.style.transform).toBe('');
      expect(container.querySelector('button')?.style.transform).toBe('');
    });

    it('adds the optical correction on the cut side only', () => {
      const { container } = render(<ActionButton tier="primary">Go</ActionButton>);
      const label = container.querySelector<HTMLElement>('button > span:not([aria-hidden])');
      // The removed triangle sat on the left, so the label gets the padding back
      // or it reads as off-centre.
      expect(label?.style.paddingLeft).toBe(`${slant.opticalPadLeft}px`);
    });

    it('drops the correction when nothing is cut', () => {
      const { container } = render(
        <ActionButton tier="primary" edge="none">
          Go
        </ActionButton>,
      );
      const label = container.querySelector<HTMLElement>('button > span:not([aria-hidden])');
      expect(label?.style.paddingLeft).toBe('0px');
    });

    it('keeps a radius off the cut shape so the diagonal survives', () => {
      const { container } = render(<ActionButton tier="primary">Go</ActionButton>);
      expect(container.querySelector('button')?.style.borderRadius).toBe('0px');
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

/**
 * The step sheet, read as text.
 *
 * `StepProgress` used to compute a colour per row in JS from `styles/tokens.ts`;
 * it now puts `data-state` on the row and a stylesheet paints it. jsdom does not
 * substitute `var()`, so a computed-style assertion here would compare two empty
 * strings and pass for the wrong reason -- which leaves exactly one thing worth
 * checking, and it is the thing that broke before: a fill and its ink have to be
 * declared in the same rule, because the two token families are not readable
 * against each other's ground.
 */
const stepSheet = stepSheetRaw.replace(/\/\*[\s\S]*?\*\//g, '');

const stepRules = new Map<string, Record<string, string>>(
  [...stepSheet.matchAll(/([^{}]+)\{([^}]*)\}/g)].map(([, selector, body]) => [
    selector.trim(),
    Object.fromEntries(
      [...body.matchAll(/([\w-]+)\s*:\s*([^;]+)/g)].map(([, prop, value]) => [
        prop.trim(),
        value.trim(),
      ]),
    ),
  ]),
);

const stepRule = (state: string) => stepRules.get(`.step[data-state='${state}']`) ?? {};

describe('StepProgress', () => {
  const steps = [
    { key: 'build', state: 'done' as const },
    { key: 'flash', state: 'current' as const },
    { key: 'monitor', state: 'pending' as const },
  ];

  it('hands the row its state for the sheet to key on', () => {
    render(<StepProgress steps={steps} />);
    for (const [key, state] of [
      ['build', 'done'],
      ['flash', 'current'],
      ['monitor', 'pending'],
    ] as const) {
      expect(screen.getByRole('listitem', { name: new RegExp(key) })).toHaveAttribute(
        'data-state',
        state,
      );
    }
  });

  it('marks the current step for assistive tech', () => {
    render(<StepProgress steps={steps} />);
    const current = screen.getByRole('listitem', { name: /flash/ });
    expect(current).toHaveAttribute('aria-current', 'step');
    expect(screen.getByRole('listitem', { name: /build/ })).not.toHaveAttribute('aria-current');
  });

  it('fills only the finished step', () => {
    expect(stepRule('done')).toMatchObject({
      background: 'var(--step-done-bg)',
      color: 'var(--step-done-ink)',
    });
    // Two states have a fill; a rule that added a third would make the row read
    // as a set of chips rather than as progress.
    const filled = [...stepRules]
      .filter(([selector, decls]) => selector.includes('[data-state=') && decls.background)
      .map(([selector]) => selector);
    expect(filled.sort()).toEqual([
      ".step[data-state='done']",
      ".step[data-state='failed']",
    ]);
  });

  it('underlines the current step with the brand rule', () => {
    // The base rule reserves the space so the row does not jump when the
    // underline arrives; only the colour is the state's own.
    expect(stepRules.get('.step')?.['border-bottom']).toBe('2px solid transparent');
    expect(stepRule('current')['border-bottom-color']).toBe('var(--step-rule)');
    expect(stepRule('current').background).toBeUndefined();
  });

  it('keeps a pending step legible rather than greying it to disabled', () => {
    // `--step-pending-ink` is the readable "not yet"; `--muted` is the control
    // grey that reads "unavailable", and the two are different values on purpose.
    expect(stepRules.get('.step')?.color).toBe('var(--step-pending-ink)');
    expect(stepRule('pending').color).toBeUndefined();
    const light = paletteFor('light');
    expect(light.stepPendingInk).not.toBe(light.muted);
    // Only the glyph dims, so the label never loses its weight.
    expect(stepRules.get(".step[data-state='pending'] .mark")?.opacity).toBe('0.7');
  });

  it('is not a row of buttons', () => {
    render(<StepProgress steps={steps} />);
    // A progress row that looks pressable becomes a control that does nothing.
    expect(screen.queryAllByRole('button')).toHaveLength(0);
    expect(stepRules.get('.step')?.cursor).toBe('default');
    expect([...stepRules.values()].some((decls) => decls.cursor === 'pointer')).toBe(false);
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
    render(<StepProgress steps={[{ key: 'flash', state: 'failed' }]} />);
    const failed = screen.getByRole('listitem', { name: /flash/ });
    expect(failed).toHaveAttribute('data-state', 'failed');
    expect(failed.textContent).toContain('✕');
    expect(failed.getAttribute('aria-label')).toContain('failed');
    expect(stepRule('failed')).toMatchObject({
      background: 'var(--step-failed-bg)',
      color: 'var(--step-failed-ink)',
    });
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

