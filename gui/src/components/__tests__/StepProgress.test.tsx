import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import stepSheetRaw from '../StepProgress.module.css?raw';
import tokensRaw from '../../styles/tokens.css?raw';
import { StepProgress } from '../StepProgress';

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

/**
 * The custom properties of one scheme, read out of the stylesheet.
 *
 * These assertions used to compare values from the JS palette module
 * `styles/tokens.ts`. The palette is CSS now, so the same question is asked of
 * the same declarations the browser reads -- which is the version that cannot
 * drift from what is rendered.
 */
const schemeTokens = (scheme: 'dark' | 'light'): Map<string, string> => {
  const css = tokensRaw.replace(/\/\*[\s\S]*?\*\//g, '');
  const block = new RegExp(`\\[data-scheme="${scheme}"\\]\\s*\\{([^}]*)\\}`).exec(css);
  return new Map(
    [...(block?.[1] ?? '').matchAll(/(--[\w-]+)\s*:\s*([^;]+)/g)].map(([, name, value]) => [
      name,
      value.trim(),
    ]),
  );
};

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
    const light = schemeTokens('light');
    expect(light.get('--step-pending-ink')).not.toBe(light.get('--muted'));
    expect(light.get('--step-pending-ink')).toBeTruthy();
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
    const light = schemeTokens('light');
    // Three outcomes, three colours: a failure must not be able to read as
    // "passed", and it must not borrow the brand green either.
    expect(light.get('--step-failed-ink')).not.toBe(light.get('--step-done-ink'));
    expect(light.get('--step-failed-ink')).not.toBe(light.get('--brand-acid'));
    expect(light.get('--step-failed-bg')).not.toBe(light.get('--step-done-bg'));
  });
});

/**
 * The numbers the row is allowed to print.
 *
 * The rule they are held to (`lib/timing.ts`): a duration is measured or absent, and
 * an estimate appears only with real history behind it. So the interesting cases
 * here are the ones that must render *nothing* -- a step that has not started, and a
 * card reopened from a transcript with no clock on it.
 */
describe('the numbers on a step row', () => {
  it('reports what a finished step took', () => {
    render(
      <StepProgress steps={[{ key: 'build', label: 'Build', state: 'done', elapsedMs: 4_234 }]} />,
    );
    expect(screen.getByText('4.2s')).toBeInTheDocument();
    expect(screen.getByRole('listitem', { name: /build/i })).toHaveAccessibleName(
      'Build: done, took 4.2s',
    );
  });

  it('counts a running step up, and shows the estimate beside it when there is one', () => {
    render(
      <StepProgress
        steps={[{ key: 'flash', label: 'Flash', state: 'current', elapsedMs: 3_100, estimateMs: 4_000 }]}
      />,
    );
    expect(screen.getByText('3.1s · ~4.0s')).toBeInTheDocument();
    // Said out loud it is an estimate, not a promise.
    expect(screen.getByRole('listitem')).toHaveAccessibleName(
      'Flash: in progress for 3.1s, about 4.0s expected',
    );
  });

  it('shows the elapsed count alone when the tool has no history', () => {
    render(
      <StepProgress
        steps={[{ key: 'flash', label: 'Flash', state: 'current', elapsedMs: 3_100, estimateMs: null }]}
      />,
    );
    expect(screen.getByText('3.1s')).toBeInTheDocument();
  });

  it('prints no number at all for a step that has not started', () => {
    render(<StepProgress steps={[{ key: 'monitor', label: 'Monitor', state: 'pending' }]} />);
    // Not `0.0s`, not a forecast: the row reports, and there is nothing to report.
    expect(screen.getByRole('listitem').textContent).not.toMatch(/\d/);
  });
});


/**
 * The part of a card's life spent on a dialog rather than on the tool.
 *
 * `elapsedMs` is already the tool's own time by the time it reaches here — `lib/timing`
 * takes the wait off it, and off the sample the next estimate learns from — so the row
 * has two jobs: keep the tool's number in first place, and account for the missing
 * minutes next to it. Under a second there is nothing to account for, and printing
 * `+0.4s waiting` would make the one that matters harder to spot.
 */
describe('the waiting time on a step row', () => {
  it('shows what the tool took and names what the reader took', () => {
    render(
      <StepProgress
        steps={[
          { key: 'build', label: 'Build', state: 'done', elapsedMs: 8_000, waitedMs: 120_000 },
        ]}
      />,
    );
    expect(screen.getByText('8.0s +2m 00s waiting')).toBeInTheDocument();
    expect(screen.getByRole('listitem', { name: /build/i })).toHaveAccessibleName(
      'Build: done, took 8.0s, after 2m 00s waiting',
    );
  });

  it('says nothing about an answer that came back at once', () => {
    render(
      <StepProgress
        steps={[
          { key: 'flash', label: 'Flash', state: 'done', elapsedMs: 8_000, waitedMs: 400 },
        ]}
      />,
    );
    expect(screen.getByText('8.0s')).toBeInTheDocument();
    expect(screen.getByRole('listitem')).toHaveAccessibleName('Flash: done, took 8.0s');
  });
});
