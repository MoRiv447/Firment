import { useEffect, useState } from 'react';

import { Callout } from '../ui';
import styles from './SettingsView.module.css';
import { api } from '../lib/api';
import type { CardMessage, ProviderEntryDto, SettingsDto } from '../types';
import { setThemeSetting } from '../lib/theme';
import { AgentCard } from './settings/AgentCard';
import { ProvidersCard } from './settings/ProvidersCard';

export function SettingsView() {
  const [settings, setSettings] = useState<SettingsDto | null>(null);
  const [models, setModels] = useState<string[]>([]);
  // Toned, not bare strings: these two lines report writes, and a card cannot tell a refusal
  // from a receipt by reading the sentence. `save` below already keeps its success and failure
  // in two separate states; these two had one state each and the colour was guessed.
  const [keyMsg, setKeyMsg] = useState<CardMessage | null>(null);
  const [saveMsg, setSaveMsg] = useState('');
  const [saveErr, setSaveErr] = useState('');

  const [newMsg, setNewMsg] = useState<CardMessage | null>(null);

  const load = () => {
    void api
      .getSettings()
      .then((s) => {
        setSettings(s);
        // Publish the stored scheme so the shell repaints without needing its own
        // fetch of the same settings.
        setThemeSetting(s.theme ?? 'auto');
      })
      .catch((err) => {
        // Without this the drawer sat on "Loading settings…" forever and the
        // user had no way to tell a slow read from a failed one.
        console.error(err);
        setSaveErr(`could not read settings: ${err}`);
      });
  };

  useEffect(() => {
    load();
  }, []);

  const refreshModels = async (provider: string) => {
    if (!provider) return;
    try {
      const list = await api.fetchModels(provider);
      setModels(list);
    } catch (err) {
      setModels([]);
      console.error(err);
    }
  };

  const save = async (draft: SettingsDto) => {
    if (!settings) {
      setSaveErr('settings are still loading — nothing has been saved');
      return false;
    }
    setSaveMsg('');
    setSaveErr('');
    try {
      await api.saveSettings(draft);
      // Repaint immediately instead of waiting for the reload below: the user
      // just chose a scheme and should see it on the spot.
      setThemeSetting(draft.theme ?? 'auto');
      setSaveMsg('saved ✓');
      load();
      return true;
    } catch (err) {
      setSaveErr(`save failed: ${err}`);
      console.error(err);
      return false;
    }
  };

  const upsertProvider = async (fields: {
    name: string;
    type: string;
    baseUrl: string | null;
    model: string;
  }) => {
    try {
      await api.setProvider(fields.name, fields.type, fields.baseUrl, fields.model);
      setNewMsg({ text: `saved provider "${fields.name}"`, tone: 'ok' });
      load();
      return true;
    } catch (err) {
      setNewMsg({ text: `failed: ${err}`, tone: 'error' });
      console.error(err);
      return false;
    }
  };

  const editProvider = async (p: ProviderEntryDto) => {
    try {
      await api.setProvider(p.name, p.type, p.base_url, p.model);
      load();
    } catch (err) {
      // Said on the card, not only in the console. The row keeps its draft on a failed write --
      // that is this card's own rule -- so without a line here the user sees the value they typed
      // still in the box and no indication that the config on disk says otherwise. The reload
      // that would have corrected the row is the `load()` on the success path, and a failure
      // skips it.
      setNewMsg({ text: `could not save "${p.name}": ${err}`, tone: 'error' });
      console.error(err);
    }
  };

  const removeProvider = async (p: ProviderEntryDto) => {
    try {
      await api.removeProvider(p.name);
      load();
    } catch (err) {
      // A delete that failed leaves the row on screen looking exactly as it did before the
      // confirmation was accepted, which reads as "it is gone from the config, just not from my
      // list" unless something says otherwise.
      setNewMsg({ text: `could not delete "${p.name}": ${err}`, tone: 'error' });
      console.error(err);
    }
  };

  // per-provider editing: update local state on change so typing never
  // triggers an RPC + reload per keystroke; persist on blur / save button.
  const setProviderLocal = (p: ProviderEntryDto, patch: Partial<ProviderEntryDto>) => {
    setSettings((s) =>
      s
        ? {
            ...s,
            providers: s.providers.map((x) => (x.name === p.name ? { ...x, ...patch } : x)),
          }
        : s,
    );
  };


  // The value is the row's draft, not a field of the settings: a stored key is never read back
  // to this side, so there is nothing here to clear and nothing here to leak. The row's Save
  // button is off until there is a draft, so what reaches here is always a key.
  const saveProviderKey = async (p: ProviderEntryDto, key: string) => {
    try {
      await api.setApiKey(p.name, key);
      setKeyMsg({ text: `key saved for ${p.name}`, tone: 'ok' });
      load();
    } catch (err) {
      // The tone is the point of this line: the string arrived in `failed: …` form and was
      // drawn in the success colour, next to a row whose key had not been stored.
      setKeyMsg({ text: `failed: ${err}`, tone: 'error' });
      console.error(err);
    }
  };

  const providerOptions = (settings?.providers ?? []).map((p) => ({ label: p.name, value: p.name }));

  return (
    <div className={styles.page}>
      <div className={styles.stack}>
        {!settings && <Callout tone="info">Loading settings…</Callout>}

        <ProvidersCard
          providers={settings?.providers ?? []}
          newMsg={newMsg}
          keyMsg={keyMsg}
          onChange={setProviderLocal}
          onPersist={editProvider}
          onSaveKey={saveProviderKey}
          onRemove={removeProvider}
          onAdd={upsertProvider}
        />

        {settings && (
          <AgentCard
            settings={settings}
            models={models}
            providers={providerOptions}
            saveMsg={saveMsg}
            saveErr={saveErr}
            onFetchModels={(p) => void refreshModels(p)}
            onSave={save}
          />
        )}
      </div>
    </div>
  );
}
