import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { FlashHistory } from '../FlashHistory';
import type { FlashHistoryDto } from '../../../types';

/**
 * The burn log.
 *
 * The reason this has a test of its own rather than riding on a test of
 * `WorkbenchView`: the section was extracted from that 1600-line file, and every
 * extraction is meant to arrive with its own proof. It is also the only place
 * that answers "did it ever reach the board?", so the failure row matters as
 * much as the success row.
 */

function burn(over: Partial<FlashHistoryDto> = {}): FlashHistoryDto {
  return {
    ts: 1_700_000_000,
    chip: 'stm32f407vetx',
    file: 'build/fw.elf',
    probe: 'ST-Link/V2',
    ok: true,
    error: null,
    ...over,
  };
}

/** The chip a row's word sits in: the fill is chosen by CSS from its status. */
const chipOf = (word: string) => screen.getByText(word).closest('[data-ui="chip"]');

describe('FlashHistory', () => {
  it('says where the record lives and what it means when empty', () => {
    render(<FlashHistory history={[]} />);
    expect(screen.getByText('.firment/work/flash-history.jsonl')).toBeInTheDocument();
    // The empty copy explains why the list is worth watching, rather than
    // just reporting nothing.
    expect(screen.getByText(/No flashes recorded yet/)).toBeInTheDocument();
  });

  it('shows the chip and the image for each burn', () => {
    render(<FlashHistory history={[burn()]} />);
    expect(screen.getByText('stm32f407vetx')).toBeInTheDocument();
    expect(screen.getByText('build/fw.elf')).toBeInTheDocument();
    expect(screen.getByText('OK')).toBeInTheDocument();
  });

  it('keeps a failed burn in the list, marked as one', () => {
    // The whole point of the log: "it never flashed" and "it flashed and the
    // board did nothing" are different problems, and only the record tells
    // them apart.
    render(
      <FlashHistory
        history={[burn(), burn({ ts: 1_700_000_100, ok: false, error: 'no probe found' })]}
      />,
    );
    expect(screen.getByText('OK')).toBeInTheDocument();
    expect(screen.getByText('FAIL')).toBeInTheDocument();
    expect(screen.getAllByText('stm32f407vetx')).toHaveLength(2);
  });

  it('writes the reason on the row that failed', () => {
    // The record always carried `error` and the card dropped it, so a burn could
    // be marked FAIL and say nothing about why while the answer sat in the DTO.
    render(<FlashHistory history={[burn({ ok: false, error: 'no probe found' })]} />);
    expect(screen.getByText('no probe found')).toBeInTheDocument();
  });

  it('carries the reason on the badge too, for a row read by its chip', () => {
    render(<FlashHistory history={[burn({ ok: false, error: 'no probe found' })]} />);
    expect(chipOf('FAIL')).toHaveAttribute('title', 'no probe found');
  });

  it('marks the outcome with a status, not with a colour in the markup', () => {
    // The fill and the ink are a CSS pair keyed on `data-status`; asserting the
    // attribute is what survives the numbers being re-scaled. A hardcoded
    // `borderRadius` assertion was already wrong once, on the day the tiers moved.
    render(<FlashHistory history={[burn(), burn({ ts: 1_700_000_100, ok: false, error: 'x' })]} />);
    expect(chipOf('OK')).toHaveAttribute('data-status', 'ok');
    expect(chipOf('FAIL')).toHaveAttribute('data-status', 'failed');
  });

  it('renders every burn, in the order given', () => {
    render(
      <FlashHistory
        history={[
          burn({ ts: 1, file: 'first.elf' }),
          burn({ ts: 2, file: 'second.elf' }),
          burn({ ts: 3, file: 'third.elf' }),
        ]}
      />,
    );
    const files = screen.getAllByText(/\.elf$/).map((el) => el.textContent);
    expect(files).toEqual(['first.elf', 'second.elf', 'third.elf']);
  });

  it('leaves the styling to the stylesheet', () => {
    // Nothing on this card is decided per row, so nothing needs an inline style --
    // and the moment one appears, the row has a second source of truth for what
    // the tokens say.
    const { container } = render(<FlashHistory history={[burn(), burn({ ts: 2, ok: false })]} />);
    expect(container.querySelectorAll('[style]')).toHaveLength(0);
  });
});
