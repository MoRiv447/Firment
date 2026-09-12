import { Card, List, Space, Statistic, Tag, Typography } from 'antd';
import { radius, statusChip } from '../../styles/tokens';
import type { ElfCardDto, QualityItemDto, TimelineEntryDto } from '../../types';

const { Text } = Typography;

/**
 * The three read-only report cards under "Insights".
 *
 * They were three anonymous blocks inside the 1600-line `WorkbenchView`. All
 * three take data and render it -- no handlers, no drafts, nothing to lift --
 * which is what made them a seam rather than a cut.
 *
 * Each one renders nothing when it has nothing (a caller guards on the array
 * being non-empty, an ELF card on `elf` being present). That is the design
 * system's "unconfigured -> hidden entirely" rule: an empty card that says "no
 * data" is worse than no card, because it looks like a failure.
 */

/** The firmware's flash/RAM budget, and the gate the change is measured against. */
export function ElfBudget({ elf }: { elf: ElfCardDto }) {
  return (
    <Card type="inner" size="small" title="ELF budget" style={{ marginBottom: 12 }}>
      <Space wrap size={24}>
        <Statistic title="flash" value={(elf.flash_bytes / 1024).toFixed(1)} suffix="KiB" />
        <Statistic title="RAM (data+bss)" value={(elf.ram_bytes / 1024).toFixed(1)} suffix="KiB" />
        <Statistic title="functions" value={elf.functions} />
      </Space>
      {elf.gate && (
        <Text type="secondary" style={{ fontSize: 11, display: 'block', marginTop: 6 }}>
          gate thresholds: stack +{elf.gate.stack_threshold}B · flash +
          {elf.gate.flash_threshold_kib}KiB · ram +{elf.gate.ram_threshold_kib}KiB
          {elf.gate.strict ? ' · strict' : ''}
        </Text>
      )}
      <Text type="secondary" style={{ fontSize: 11, display: 'block' }}>
        {elf.file}
      </Text>
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
    <Card
      type="inner"
      size="small"
      title="Verification badges (mainline)"
      style={{ marginBottom: 12 }}
    >
      <Space wrap size={8}>
        {quality.map((q) => (
          <Tag key={q.tool} style={{ ...statusChip(q.ok ? 'ok' : 'failed'), borderRadius: radius.chip, fontSize: 12 }}>
            {q.tool}: {q.ok ? 'PASS' : 'FAIL'}
          </Tag>
        ))}
      </Space>
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
    <Card type="inner" size="small" title="Change timeline (mainline)">
      <List
        size="small"
        dataSource={timeline}
        renderItem={(entry) => (
          <List.Item style={{ padding: '4px 0' }}>
            <div style={{ width: '100%' }}>
              <Text type="secondary" style={{ fontSize: 11 }}>
                #{entry.seq} · {new Date(entry.created_at * 1000).toLocaleString()}
              </Text>
              {entry.files.map((f) => (
                <div key={f.path} style={{ fontSize: 12 }}>
                  <Text code>{f.path}</Text>{' '}
                  <Text type="secondary">
                    {f.old_lines} → {f.new_lines}
                  </Text>
                </div>
              ))}
            </div>
          </List.Item>
        )}
      />
    </Card>
  );
}
