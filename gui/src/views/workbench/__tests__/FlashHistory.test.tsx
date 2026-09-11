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

describe('FlashHistory', () => {
  it('says where the record lives and what it means when empty', () => {
    render(<FlashHistory history={[]} />);
    expect(screen.getByText('.firment/work/flash-history.jsonl')).toBeInTheDocument();
    // The empty copy explains why the list is worth watching, rather than
    // just reporting nothing.
    expect(screen.getByText(/暂无烧录记录/)).toBeInTheDocument();
  });

  it('shows the chip and the image for each burn', () => {
    render(<FlashHistory history={[burn()]} />);
    expect(screen.getByText('stm32f407vetx')).toBeInTheDocument();
    expect(screen.getByText('build/fw.elf')).toBeInTheDocument();
    expect(screen.getByText('✓')).toBeInTheDocument();
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
    expect(screen.getByText('✓')).toBeInTheDocument();
    expect(screen.getByText('✗')).toBeInTheDocument();
    expect(screen.getAllByText('stm32f407vetx')).toHaveLength(2);
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

  it('keeps the status chip a chip, not a hard-edged tile', () => {
    render(<FlashHistory history={[burn()]} />);
    // The extraction fixed a drift: this badge was `borderRadius: 0`, which is
    // the brand-tile radius. A badge is a chip (docs/design/tokens.md).
    const chip = screen.getByText('✓');
    expect(chip).toHaveStyle({ borderRadius: '2px' });
  });

  it('sets the code-ish columns in the mono stack', () => {
    render(<FlashHistory history={[burn()]} />);
    const chipName = screen.getByText('stm32f407vetx');
    expect(chipName.style.fontFamily).toContain('JetBrains Mono');
  });
});
