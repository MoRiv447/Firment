import { useEffect, useState } from 'react';

import { Callout } from '../ui';
import styles from './SettingsView.module.css';
import { api } from '../lib/api';
import type { ProviderEntryDto, SettingsDto } from '../types';
import { setThemeSetting } from '../lib/theme';
import { AgentCard } from './settings/AgentCard';
import { ProvidersCard } from './settings/ProvidersCard';

export function SettingsView() {
  const [settings, setSettings] = useState<SettingsDto | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [keyMsg, setKeyMsg] = useState('');
  const [saveMsg, setSaveMsg] = useState('');
  const [saveErr, setSaveErr] = useState('');

  const [newMsg, setNewMsg] = useState('');

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
      setNewMsg(`saved provider "${fields.name}"`);
      load();
      return true;
    } catch (err) {
      setNewMsg(`failed: ${err}`);
      console.error(err);
      return false;
    }
  };

  const editProvider = async (p: ProviderEntryDto) => {
    try {
      await api.setProvider(p.name, p.type, p.base_url, p.model);
      load();
    } catch (err) {
      console.error(err);
    }
  };

  const removeProvider = async (p: ProviderEntryDto) => {
    try {
      await api.removeProvider(p.name);
      load();
    } catch (err) {
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


  const saveProviderKey = async (p: ProviderEntryDto) => {
    const key = p.api_key?.trim() ?? '';
    if (!key) {
      setKeyMsg(`empty key for ${p.name} — nothing saved (env fallback still applies)`);
      return;
    }
    try {
      await api.setApiKey(p.name, key);
      setKeyMsg(`key saved for ${p.name}`);
      load();
    } catch (err) {
      setKeyMsg(`failed: ${err}`);
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
