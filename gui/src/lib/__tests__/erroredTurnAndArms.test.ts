import { describe, expect, it } from 'vitest';

import { initialTurnState, turnReducer } from '../turnReducer';

/**
 * Two things that were each invisible in a diff, both about the last moment of a turn.
 *
 * The first half is behavioural: a turn that failed in the background used to lose its `⚠ …`
 * line the instant you opened that chat, because the open triggers a `turn_synced` and
 * `turn_synced` drops the retained live turn -- correct for a reply the refreshed transcript now
 * contains, wrong for a failure the transcript never held.
 *
 * The second half is a source gate over the event switch in `App.tsx` and the tab body in
 * `HardwarePane.tsx`. Both were checked at the source because the alternative is a mount test
 * that would spend more code on fakes than on the property, and because each hazard is a line
 * that has to be present, not a behaviour that emerges.
 */

const fold = (events: Parameters<typeof turnReducer>[1][]) =>
  events.reduce((state, event) => turnReducer(state, event), initialTurnState());

describe('an errored turn keeps the only copy of its failure', () => {
  it('survives the transcript sync that a finished turn does not', () => {
    const errored = fold([
      { type: 'turn_start' },
      { type: 'text_delta', text: 'thinking about the timer' },
      { type: 'error', message: 'provider error: 500 boom' },
    ]);
    expect(errored.turn?.text).toContain('⚠ provider error: 500 boom');
    expect(errored.running).toBe(false);

    // The bug: opening the chat fires `turn_synced`, and that arm dropped the turn wholesale.
    const reopened = turnReducer(errored, { type: 'turn_synced' });
    expect(reopened.turn?.text).toContain('⚠ provider error: 500 boom');

    // The negative control, so this is not "never drop": a turn that ENDED cleanly is dropped,
    // because the refreshed transcript really does contain its reply.
    const clean = fold([
      { type: 'turn_start' },
      { type: 'text_delta', text: 'here is the driver' },
      { type: 'turn_end', text: 'here is the driver' },
    ]);
    expect(turnReducer(clean, { type: 'turn_synced' }).turn).toBeNull();

    // And the mark does not leak into the next turn: a fresh start has no error to remember.
    const next = turnReducer(reopened, { type: 'turn_start' });
    expect(next.turn?.errored).toBeFalsy();
    expect(turnReducer({ ...next, running: false }, { type: 'turn_synced' }).turn).toBeNull();
  });
});

/**
 * `App.tsx`'s event switch flushes the delta buffer before handing an event to the reducer in
 * every arm that dispatches. The `default:` arm -- the one every kind not named above it falls
 * through to, including a `turn_end` -- used to dispatch without flushing, so text that arrived
 * in the same frame was discarded with the buffer.
 */
describe('the fall-through arm of the event switch behaves like the arms above it', () => {
  const sources = import.meta.glob(['../../App.tsx', '../../shell/panes/HardwarePane.tsx'], {
    query: '?raw',
    import: 'default',
    eager: true,
  }) as Record<string, string>;

  const read = (name: string): string => {
    const found = Object.entries(sources).find(([path]) => path.endsWith(`/${name}`));
    if (!found) throw new Error(`${name} was not read`);
    return found[1];
  };

  it('flushes before it dispatches', () => {
    const app = read('App.tsx');
    // Find the arm by what it does rather than by which `default:` it happens to be: the file
    // has other switches, and counting them would have this test fail for the wrong reason.
    const arm = app
      .split('default:')
      .slice(1)
      .map((chunk) => {
        const end = chunk.indexOf('break;');
        return end < 0 ? chunk : chunk.slice(0, end);
      })
      .find((body) => body.includes('dispatchTurn(e)'));
    expect(arm, 'no `default:` arm dispatches to the turn reducer any more').toBeTruthy();
    expect(arm).toContain('flushDeltas()');
    expect(
      arm!.indexOf('flushDeltas()'),
      'a flush after the dispatch would flush nothing: the reducer has already dropped the turn',
    ).toBeLessThan(arm!.indexOf('dispatchTurn(e)'));
    // The arms above it must still be there for this to be a parity rule rather than one check.
    expect(app.match(/flushDeltas\(\)/g)?.length).toBeGreaterThanOrEqual(4);
  });

  it('keeps both hardware views mounted across a tab switch', () => {
    const pane = read('HardwarePane.tsx');
    // The ternary that used to pick one view is the defect: choosing unmounts the other, and
    // the flash form plus its result panel reset on every switch.
    expect(pane).not.toMatch(/tab === 'serial' \? <SerialView/);
    expect(pane).toContain("hidden={tab !== 'serial'}");
    expect(pane).toContain("hidden={tab !== 'flash'}");
    expect(pane.match(/<(SerialView|FlashView)\b/g)).toHaveLength(2);
  });
});
