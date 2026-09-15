import { useState } from 'react';

import type { SettingsDto } from '../../types';
import { Button, Callout, Card, Field, MultiSelect, NumberField, Select, TextInput } from '../../ui';
import styles from './AgentCard.module.css';

const THINKING = ['off', 'low', 'medium', 'high', 'xhigh', 'max'].map((t) => ({
  label: t,
  value: t,
}));

const THEMES = [
  { label: 'auto', value: 'auto' },
  { label: 'light', value: 'light' },
  { label: 'dark', value: 'dark' },
];

const SEARCH_PROVIDERS = [
  { label: 'bing (default — no key, CN-reachable)', value: 'bing' },
  { label: 'duckduckgo (no key)', value: 'duckduckgo' },
  { label: 'tavily (API key)', value: 'tavily' },
  { label: 'brave (API key)', value: 'brave' },
];

/**
 * The agent's settings, as a controlled form.
 *
 * It was an antd `Form`: twelve `Form.Item`s whose values lived inside the form
 * instance and were read back with `getFieldsValue()` at save time. The values are
 * a plain draft object now, which is what the rest of this layer already does --
 * and it is why `extra=` and `tooltip=` became `hint=`: a tooltip is text that
 * explains a field, and `Field` already has the line for it, where a screen reader
 * can reach it.
 *
 * The draft is re-seeded when the caller's settings change, which happens after
 * every save (the caller reloads) -- so a failed save leaves your edits alone and
 * a successful one shows you what was actually stored, rather than what you typed.
 *
 * Three of these fields are numbers and three are nullable strings. Both kinds
 * have exactly one rule each, and the layer's `NumberField` and the `?? ''` /
 * `|| null` pair here are where those rules live.
 */
export function AgentCard({
  settings,
  models,
  providers,
  saveMsg,
  saveErr,
  onFetchModels,
  onSave,
}: {
  settings: SettingsDto;
  models: string[];
  providers: { label: string; value: string }[];
  saveMsg: string;
  saveErr: string;
  onFetchModels: (provider: string) => void;
  /** Returns true when the settings were written. */
  onSave: (draft: SettingsDto) => Promise<boolean>;
}) {
  const [draft, setDraft] = useState<SettingsDto>(settings);
  const [saving, setSaving] = useState(false);
  const [seeded, setSeeded] = useState(settings);

  // Re-seed from the caller when it reloads (after a save). Not on every render:
  // that would fight whoever is typing.
  if (seeded !== settings) {
    setSeeded(settings);
    setDraft(settings);
  }

  const patch = (next: Partial<SettingsDto>) => setDraft((d) => ({ ...d, ...next }));

  const save = async () => {
    setSaving(true);
    try {
      // `providers` is carried over rather than edited here, so it always comes
      // from the loaded settings -- not from the draft.
      await onSave({ ...draft, providers: settings.providers });
    } finally {
      setSaving(false);
    }
  };

  return (
    <Card title="Agent">
      <div className={styles.grid}>
        <Field label="Default provider">
          <Select
            value={draft.default_provider}
            onChange={(v) => {
              patch({ default_provider: v });
              onFetchModels(v);
            }}
            options={providers}
          />
        </Field>
        <Field label="Model">
          <TextInput
            mono
            value={draft.default_model}
            onChange={(e) => patch({ default_model: e.target.value })}
          />
        </Field>
        <Field label="Thinking">
          <Select
            value={draft.thinking}
            onChange={(v) => patch({ thinking: v })}
            options={THINKING}
          />
        </Field>
        <Field label="Max iterations" hint="How many tool calls one turn may make.">
          <NumberField
            min={1}
            max={100}
            value={draft.max_iterations}
            onValueChange={(n) => patch({ max_iterations: n ?? 1 })}
          />
        </Field>
        <Field
          label="Theme"
          hint="auto follows the OS. Pinning light or dark overrides it, so on a dark machine pick light to see the light scheme."
        >
          <Select value={draft.theme} onChange={(v) => patch({ theme: v })} options={THEMES} />
        </Field>
      </div>

      <div className={styles.models}>
        <Button size="sm" onClick={() => onFetchModels(draft.default_provider)}>
          Fetch models
        </Button>
        {models.map((m) => (
          <Button
            key={m}
            size="sm"
            className={styles.model}
            onClick={() => patch({ default_model: m })}
          >
            {m}
          </Button>
        ))}
      </div>

      <div className={styles.grid}>
        <Field label="Build command">
          <TextInput
            mono
            placeholder="e.g. cargo build"
            value={draft.build_command ?? ''}
            onChange={(e) => patch({ build_command: e.target.value || null })}
          />
        </Field>
        <Field label="Default chip">
          <TextInput
            mono
            placeholder="nrf52840, stm32f103, …"
            value={draft.default_chip ?? ''}
            onChange={(e) => patch({ default_chip: e.target.value || null })}
          />
        </Field>
        <Field label="Monitor port">
          <TextInput
            mono
            placeholder="COM3"
            value={draft.monitor_port ?? ''}
            onChange={(e) => patch({ monitor_port: e.target.value || null })}
          />
        </Field>
        <Field label="Baud">
          <NumberField
            min={1200}
            max={3000000}
            suffix="baud"
            value={draft.monitor_baud}
            onValueChange={(n) => patch({ monitor_baud: n ?? 1200 })}
          />
        </Field>
      </div>

      <div className={styles.wide}>
        <Field
          label="Auto-approve tools"
          hint="Tools the agent may run without asking. Anything not listed here still asks."
        >
          <MultiSelect
            value={draft.auto_approve}
            onChange={(next) => patch({ auto_approve: next })}
            options={[]}
            emptyLabel="Type a tool name and press Enter"
            placeholder="tool names, e.g. read_file"
          />
        </Field>
        <Field label="Context budget (chars)" hint="Steps of 10000. The turn compacts once it reaches this.">
          <NumberField
            min={10000}
            suffix="chars"
            value={draft.context_budget_chars}
            onValueChange={(n) => patch({ context_budget_chars: n ?? 10000 })}
          />
        </Field>
        <Field
          label="Web search provider"
          hint="Default: bing (no key, CN-reachable). Set to duckduckgo for international."
        >
          <Select
            clearable
            placeholder="bing (default)"
            value={draft.web_search ?? ''}
            onChange={(v) => patch({ web_search: v || null })}
            options={SEARCH_PROVIDERS}
          />
        </Field>
      </div>

      <div className={styles.footer}>
        <Button tier="primary" disabled={saving} onClick={() => void save()}>
          Save settings
        </Button>
        {saveMsg && <span className={styles.ok}>{saveMsg}</span>}
        {saveErr && (
          <span className={styles.failed}>
            <Callout tone="failed" title="Could not save">
              {saveErr}
            </Callout>
          </span>
        )}
      </div>
    </Card>
  );
}
