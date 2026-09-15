import { Trash2 } from 'lucide-react';
import { useRef, useState } from 'react';

import type { ProviderEntryDto } from '../../types';
import { Button, Card, Chip, IconButton, PopConfirm, Select, TextInput } from '../../ui';
import styles from './ProvidersCard.module.css';

const TYPES = [
  { label: 'openai', value: 'openai' },
  { label: 'anthropic', value: 'anthropic' },
];

/**
 * The providers, and the one row that adds another.
 *
 * Every provider is its own little record with its own three write paths -- the
 * row persists on blur, the key has its own button (writing a key is a different
 * file from writing the config), and deleting asks first -- so a row is a
 * component. Editing updates the caller's copy on every keystroke and persists on
 * blur: one RPC plus a full reload per character is not worth the round trip, and
 * losing the edit on a failed write is worse than keeping it.
 *
 * The delete confirmation is anchored to the button it belongs to rather than
 * being a modal: it is a question about one row, and it should not take the whole
 * screen to ask it.
 */
export function ProvidersCard({
  providers,
  newMsg,
  keyMsg,
  onChange,
  onPersist,
  onSaveKey,
  onRemove,
  onAdd,
}: {
  providers: ProviderEntryDto[];
  /** Set by the caller's IO, shown here because this is where it happened. */
  newMsg: string;
  keyMsg: string;
  /** A local edit -- the caller holds the list, this holds the typing. */
  onChange: (provider: ProviderEntryDto, patch: Partial<ProviderEntryDto>) => void;
  /** Write the row out. Called on blur. */
  onPersist: (provider: ProviderEntryDto) => void;
  onSaveKey: (provider: ProviderEntryDto) => void;
  onRemove: (provider: ProviderEntryDto) => void;
  /** Returns true when the provider was stored. */
  onAdd: (fields: {
    name: string;
    type: string;
    baseUrl: string | null;
    model: string;
  }) => Promise<boolean>;
}) {
  const [name, setName] = useState('');
  const [type, setType] = useState('openai');
  const [baseUrl, setBaseUrl] = useState('');
  const [model, setModel] = useState('');
  // The card's own, because the original had none at all: the add button and the
  // per-row buttons were live during a write, so a second click queued a second
  // RPC. It is not the caller's flag -- the caller's `saving` means "the settings
  // form is being written", which is a different write.
  const [busy, setBusy] = useState(false);

  const wrap = async <T,>(fn: () => Promise<T> | T): Promise<T> => {
    setBusy(true);
    try {
      return await fn();
    } finally {
      setBusy(false);
    }
  };

  const complete = name.trim() !== '' && model.trim() !== '';

  const add = async () => {
    if (busy || !complete) return;
    const stored = await wrap(() =>
      onAdd({
        name: name.trim(),
        type,
        baseUrl: baseUrl.trim() || null,
        model: model.trim(),
      }),
    );
    if (stored) {
      setName('');
      setBaseUrl('');
      setModel('');
    }
  };

  return (
    <Card title="Providers">
      <p className={styles.hint}>
        Each provider has its own API key, base URL and model. The default provider is used for
        new sessions; delete any provider — the default automatically moves to the next one.
      </p>

      {providers.map((provider) => (
        <ProviderRow
          key={provider.name}
          provider={provider}
          busy={busy}
          onChange={onChange}
          onPersist={(p) => void wrap(() => onPersist(p))}
          onSaveKey={(p) => void wrap(() => onSaveKey(p))}
          onRemove={(p) => void wrap(() => onRemove(p))}
        />
      ))}

      <div className={styles.add}>
        <p className={styles.addTitle}>Add provider</p>
        <div className={styles.addRow}>
          {/* Every control on this row is the same height: the design system's
              control height. A tall CTA beside short inputs is the one thing that
              makes a row look assembled rather than designed. */}
          <TextInput
            size="lg"
            placeholder="name (e.g. deepseek)"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void add();
            }}
          />
          <Select
            ariaLabel="provider type"
            size="lg"
            value={type}
            onChange={setType}
            options={TYPES}
          />
          <TextInput
            size="lg"
            mono
            placeholder="base url (e.g. https://api.deepseek.com/v1)"
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
          />
          <TextInput
            size="lg"
            placeholder="model (e.g. deepseek-v4-flash)"
            value={model}
            onChange={(e) => setModel(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void add();
            }}
          />
          <Button tier="primary" disabled={busy || !complete} onClick={() => void add()}>
            Save provider
          </Button>
        </div>
        {/* The button says why it is off; a message would say it after the fact. */}
        {!complete && (name.trim() !== '' || model.trim() !== '') && (
          <p className={styles.hint}>A provider needs a name and a model.</p>
        )}
      </div>

      {newMsg && <p className={styles.note}>{newMsg}</p>}
      {keyMsg && <p className={styles.ok}>{keyMsg}</p>}
      <p className={styles.hint}>
        Providers are stored in config.toml; keys in auth.json. Pick the default in the Agent card
        below.
      </p>
    </Card>
  );
}

/** One provider: what it is, where it points, and the key for it. */
function ProviderRow({
  provider,
  busy,
  onChange,
  onPersist,
  onSaveKey,
  onRemove,
}: {
  provider: ProviderEntryDto;
  busy: boolean;
  onChange: (provider: ProviderEntryDto, patch: Partial<ProviderEntryDto>) => void;
  onPersist: (provider: ProviderEntryDto) => void;
  onSaveKey: (provider: ProviderEntryDto) => void;
  onRemove: (provider: ProviderEntryDto) => void;
}) {
  const [asking, setAsking] = useState(false);
  // The confirmation hang off a span because the button it belongs to is a layer
  // component, and the layer does not hand out refs from inside.
  const anchorRef = useRef<HTMLSpanElement | null>(null);

  return (
    <div className={styles.row}>
      <div className={styles.head}>
        <span className={styles.name}>{provider.name}</span>
        {provider.is_default && (
          <Chip size="sm" status="attention">
            DEFAULT
          </Chip>
        )}
        <span className={styles.spacer} />
        <span ref={anchorRef}>
          <IconButton
            tier="ghost"
            size="sm"
            icon={Trash2}
            label={`Delete ${provider.name}`}
            disabled={busy}
            onClick={() => setAsking(true)}
          />
        </span>
        <PopConfirm
          open={asking}
          anchorRef={anchorRef}
          title={`Delete "${provider.name}"?`}
          message="The default (if this one) moves to another provider."
          confirmLabel="delete"
          tone="danger"
          side="bottom"
          align="end"
          onClose={() => setAsking(false)}
          onConfirm={() => {
            setAsking(false);
            onRemove(provider);
          }}
        />
      </div>
      <div className={styles.fields}>
        <Select
          ariaLabel={`${provider.name} type`}
          value={provider.type}
          onChange={(v) => onPersist({ ...provider, type: v })}
          options={TYPES}
        />
        <TextInput
          mono
          placeholder="base url"
          value={provider.base_url ?? ''}
          onChange={(e) => onChange(provider, { base_url: e.target.value || null })}
          onBlur={() => onPersist(provider)}
        />
        <TextInput
          placeholder="model"
          value={provider.model}
          onChange={(e) => onChange(provider, { model: e.target.value })}
          onBlur={() => onPersist(provider)}
        />
      </div>
      <div className={styles.key}>
        <TextInput
          mono
          type="password"
          placeholder={`api key for ${provider.name} (empty = use env)`}
          value={provider.api_key ?? ''}
          onChange={(e) => onChange(provider, { api_key: e.target.value })}
        />
        <Button size="sm" disabled={busy} onClick={() => onSaveKey(provider)}>
          Save key
        </Button>
      </div>
    </div>
  );
}
