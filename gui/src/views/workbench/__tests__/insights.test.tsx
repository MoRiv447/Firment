import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { ChangeTimeline, ElfBudget, VerificationBadges } from '../insights';
import type { ElfCardDto, QualityItemDto, TimelineEntryDto } from '../../../types';

/**
 * The three report cards under Insights.
 *
 * They came out of the same 1600-line file as `FlashHistory`, so they carry
 * their own proof for the same reason. What they have in common is that they
 * report numbers nobody can re-derive by eye once the card is on screen: the
 * ELF budget decides whether a change is too big, and the badges decide whether
 * anyone checked.
 */

function elfCard(over: Partial<ElfCardDto> = {}): ElfCardDto {
  return {
    file: 'build/fw.elf',
    flash_bytes: 24_576,
    ram_bytes: 4_096,
    functions: 42,
    gate: {
      stack_threshold: 32,
      flash_threshold_kib: 1,
      ram_threshold_kib: 1,
      strict: false,
    },
    ...over,
  };
}

const chips: QualityItemDto[] = [
  { tool: 'cargo test', ok: true, snippet: 'ok. 42 passed' },
  { tool: 'clippy', ok: false, snippet: 'warning: unused import' },
];

const timeline: TimelineEntryDto[] = [
  {
    seq: 7,
    created_at: 1_700_000_000,
    files: [{ path: 'src/main.c', old_lines: 118, new_lines: 121 }],
  },
];

describe('ElfBudget', () => {
  it('reports the budget in KiB, not bytes', () => {
    // 24576 bytes is 24.0 KiB; a raw byte count is unreadable at a glance.
    render(<ElfBudget elf={elfCard()} />);
    expect(screen.getByText('24.0 KiB')).toBeInTheDocument();
    expect(screen.getByText('4.0 KiB')).toBeInTheDocument();
    expect(screen.getByText('42')).toBeInTheDocument();
    expect(screen.queryByText('24576')).toBeNull();
  });

  it('carries on past KiB instead of printing 1024.0 KiB', () => {
    // The hand-rolled `(n / 1024).toFixed(1)` this card used was right up to the
    // first 1 MiB binary, where it said "1024.0 KiB". A firmware image crosses
    // that line routinely, so the ladder is the thing being tested, not the
    // rounding.
    render(<ElfBudget elf={elfCard({ flash_bytes: 1_572_864 })} />);
    expect(screen.getByText('1.5 MiB')).toBeInTheDocument();
    expect(screen.queryByText('1024.0 KiB')).toBeNull();
  });

  it('names the gate thresholds the change is measured against', () => {
    render(<ElfBudget elf={elfCard()} />);
    const gate = screen.getByText(/gate thresholds:/);
    expect(gate.textContent).toContain('stack +32B');
    expect(gate.textContent).toContain('flash +1KiB');
    expect(gate.textContent).toContain('ram +1KiB');
    // A soft gate and a strict one are not the same promise.
    expect(gate.textContent).not.toContain('strict');
  });

  it('says when the gate is strict', () => {
    render(<ElfBudget elf={elfCard({ gate: { ...elfCard().gate!, strict: true } })} />);
    expect(screen.getByText(/gate thresholds:/).textContent).toContain('strict');
  });

  it('omits the threshold line entirely when there is no gate', () => {
    render(<ElfBudget elf={elfCard({ gate: null })} />);
    expect(screen.queryByText(/gate thresholds:/)).toBeNull();
    // The file it measured is still worth showing.
    expect(screen.getByText('build/fw.elf')).toBeInTheDocument();
  });
});

describe('VerificationBadges', () => {
  it('names the tool on every badge', () => {
    render(<VerificationBadges quality={chips} />);
    // A badge that only says "green" does not say what passed.
    expect(screen.getByText('cargo test: PASS')).toBeInTheDocument();
    expect(screen.getByText('clippy: FAIL')).toBeInTheDocument();
  });

  it('renders nothing without items rather than an empty card', () => {
    // The caller guards on length; this pins that the guard is what keeps an
    // empty card off the screen.
    render(<VerificationBadges quality={[]} />);
    expect(screen.queryByText('Verification badges (mainline)')).toBeInTheDocument();
    expect(screen.queryAllByText(/PASS|FAIL/)).toHaveLength(0);
  });
});

describe('ChangeTimeline', () => {
  it('shows the delta per file, not just the new size', () => {
    render(<ChangeTimeline timeline={timeline} />);
    // For firmware the delta is the number that matters.
    expect(screen.getByText('src/main.c')).toBeInTheDocument();
    expect(screen.getByText(/118\s*→\s*121/)).toBeInTheDocument();
  });

  it('marks the entry with its sequence number', () => {
    render(<ChangeTimeline timeline={timeline} />);
    expect(screen.getByText(/^#7 · /)).toBeInTheDocument();
  });

  it('lists every file in an entry', () => {
    render(
      <ChangeTimeline
        timeline={[
          {
            seq: 8,
            created_at: 1_700_000_100,
            files: [
              { path: 'a.c', old_lines: 1, new_lines: 2 },
              { path: 'b.c', old_lines: 3, new_lines: 4 },
            ],
          },
        ]}
      />,
    );
    expect(screen.getByText('a.c')).toBeInTheDocument();
    expect(screen.getByText('b.c')).toBeInTheDocument();
  });
});
