import { RotateCw } from 'lucide-react';

import { formatBytes, formatStamp } from '../../lib/format';
import type { ElfCardDto, GateThresholdsDto, QualityItemDto, TimelineEntryDto } from '../../types';
import { Button, Callout, Card, Chip, Stat } from '../../ui';
import styles from './insights.module.css';

/**
 * The read-only report cards under "Insights", and the card that holds them.
 *
 * The three report cards were three anonymous blocks inside the 1600-line
 * `WorkbenchView`. All three take data and render it -- no handlers, no drafts,
 * nothing to lift -- which is what made them a seam rather than a cut. The
 * wrapper above them came last, and it is here rather than in a file of its own
 * because on a case-insensitive filesystem `Insights.tsx` *is* this file.
 *
 * Each report renders nothing when it has nothing (a caller guards on the array
 * being non-empty, an ELF card on `elf` being present). That is the design
 * system's "unconfigured -> hidden entirely" rule: an empty card that says "no
 * data" is worse than no card, because it looks like a failure.
 *
 * The sizes here go through `formatBytes` rather than `(n / 1024).toFixed(1)`
 * with a separate `KiB` span next to it. The arithmetic was right up to the
 * first 1 MiB binary, where it printed "1024.0 KiB"; the formatter ends the same
 * ladder at GiB and says which unit it landed on.
 */

/**
 * The container, plus the one control that reloads all three.
 *
 * The refresh is gated on a mainline session existing rather than on anything in
 * here: without a mainline there is nothing to ask about, and a refresh button
 * that can only fail is worse than a disabled one.
 */
export function Insights({
  elf,
  elfError,
  quality,
  timeline,
  hasMainline,
  busy,
  onRefresh,
}: {
  elf: ElfCardDto | null;
  elfError: string | null;
  quality: QualityItemDto[];
  timeline: TimelineEntryDto[];
  /** Whether a mainline session exists — without one there is nothing to load. */
  hasMainline: boolean;
  busy: boolean;
  onRefresh: () => void;
}) {
  return (
    <Card
      title="Insights"
      extra={
        <Button size="sm" icon={RotateCw} disabled={busy || !hasMainline} onClick={onRefresh}>
          refresh
        </Button>
      }
    >
      {elfError && (
        <div className={styles.notice}>
          <Callout tone="warn" title="ELF budget card unavailable">
            {elfError}
          </Callout>
        </div>
      )}
      {elf && <ElfBudget elf={elf} />}
      {quality.length > 0 && <VerificationBadges quality={quality} />}
      {timeline.length > 0 && <ChangeTimeline timeline={timeline} />}
    </Card>
  );
}

/** The gate is a promise about the next change, so it says by how much it allows. */
function gateNote(gate: GateThresholdsDto): string {
  const limits = [
    `stack +${gate.stack_threshold}B`,
    `flash +${gate.flash_threshold_kib}KiB`,
    `ram +${gate.ram_threshold_kib}KiB`,
  ].join(' · ');
  return gate.strict ? `gate thresholds: ${limits} · strict` : `gate thresholds: ${limits}`;
}

/** The firmware's flash/RAM budget, and the gate the change is measured against. */
export function ElfBudget({ elf }: { elf: ElfCardDto }) {
  return (
    <Card title="ELF budget">
      <div className={styles.facts}>
        <Stat value={formatBytes(elf.flash_bytes)} label="flash" />
        <Stat value={formatBytes(elf.ram_bytes)} label="RAM (data+bss)" />
        <Stat value={elf.functions} label="functions" />
      </div>
      {elf.gate && <p className={styles.note}>{gateNote(elf.gate)}</p>}
      <p className={styles.note}>{elf.file}</p>
    </Card>
  );
}

/**
 * The last verification run, one badge per tool.
 *
 * PASS/FAIL rather than a green tick: the tool name is what makes the badge
 * readable, and a badge that says only "green" tells you nothing about what
 * passed.
 */
export function VerificationBadges({ quality }: { quality: QualityItemDto[] }) {
  return (
    <Card title="Verification badges (mainline)">
      <div className={styles.badges}>
        {quality.map((q) => (
          // The snippet is what the tool printed. It does not fit on a badge and
          // it is the first thing someone wants when one says FAIL, so it is the
          // badge's own description rather than a second line under it.
          <Chip key={q.tool} status={q.ok ? 'ok' : 'failed'} mono title={q.snippet}>
            {q.tool}: {q.ok ? 'PASS' : 'FAIL'}
          </Chip>
        ))}
      </div>
    </Card>
  );
}

/**
 * What the mainline changed, and how much it grew.
 *
 * `old -> new` per file, because the number that matters for firmware is the
 * delta, not the new size on its own.
 */
export function ChangeTimeline({ timeline }: { timeline: TimelineEntryDto[] }) {
  return (
    <Card title="Change timeline (mainline)">
      {timeline.map((entry) => (
        <div key={entry.seq} className={styles.entry}>
          <p className={styles.stamp} title={new Date(entry.created_at * 1000).toLocaleString()}>
            #{entry.seq} · {formatStamp(entry.created_at)}
          </p>
          {entry.files.map((f) => (
            <p key={f.path} className={styles.file}>
              <code className={styles.path} title={f.path}>
                {f.path}
              </code>
              <span className={styles.delta}>
                {f.old_lines} → {f.new_lines}
              </span>
            </p>
          ))}
        </div>
      ))}
    </Card>
  );
}
