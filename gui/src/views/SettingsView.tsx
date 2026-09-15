import {
  Alert,
  Button,
  Card,
  Form,
  Input,
  InputNumber,
  Select,
  Space,
  Tag,
  Typography,
  Divider,
} from 'antd';
import { useEffect, useState } from 'react';
import { api } from '../lib/api';
import type { ProviderEntryDto, SettingsDto } from '../types';
import { setThemeSetting } from '../lib/theme';
import { ActionButton } from '../components/ActionButton';
import { ProvidersCard } from './settings/ProvidersCard';

const { Text } = Typography;

export function SettingsView() {
  const [settings, setSettings] = useState<SettingsDto | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);
  const [keyMsg, setKeyMsg] = useState('');
  const [saveMsg, setSaveMsg] = useState('');
  const [saveErr, setSaveErr] = useState('');
  const [form] = Form.useForm<SettingsDto>();

  const [newMsg, setNewMsg] = useState('');

  const load = () => {
    void api
      .getSettings()
      .then((s) => {
        setSettings(s);
        form.setFieldsValue(s);
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
  }, [form]);

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

  const save = async () => {
    // `providers` is not an editable field in this form -- it is carried over
    // from the loaded settings. While that load is pending (or has failed), the
    // carry-over value is `[]`, and saving would write it over the real list.
    if (!settings) {
      setSaveErr('settings are still loading — nothing has been saved');
      return;
    }
    setSaving(true);
    setSaveMsg('');
    setSaveErr('');
    try {
      const values = form.getFieldsValue() as SettingsDto;
      // providers is not part of the antd form; carry it over from state
      values.providers = settings?.providers ?? [];
      await api.saveSettings(values);
      // Repaint immediately instead of waiting for the reload below: the user
      // just chose a scheme and should see it on the spot.
      setThemeSetting(values.theme ?? 'auto');
      setSaving(false);
      setSaveMsg('saved ✓');
      load();
    } catch (err) {
      setSaving(false);
      setSaveErr(`save failed: ${err}`);
      console.error(err);
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
    <div style={{ padding: 20, height: '100%', overflowY: 'auto' }}>
      <Space direction="vertical" size={16} style={{ width: '100%' }}>
        {!settings && <Alert type="info" showIcon message="Loading settings…" />}

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

        <Card title="Agent" size="small">
          <Form form={form} layout="vertical">
            <Space size={16} wrap>
              <Form.Item name="default_provider" label="Default provider" style={{ minWidth: 180 }}>
                <Select
                  options={providerOptions}
                  onChange={(v) => refreshModels(v)}
                  showSearch
                />
              </Form.Item>
              <Form.Item name="default_model" label="Model">
                <Input style={{ width: 240 }} />
              </Form.Item>
              <Form.Item name="thinking" label="Thinking">
                <Select
                  style={{ width: 120 }}
                  options={['off', 'low', 'medium', 'high', 'xhigh', 'max'].map((t) => ({ label: t, value: t }))}
                />
              </Form.Item>
              <Form.Item name="max_iterations" label="Max iterations">
                <InputNumber min={1} max={100} />
              </Form.Item>
              <Form.Item
                name="theme"
                label="Theme"
                tooltip="auto follows the OS. Pinning light or dark overrides it, so on a dark machine pick light to see the light scheme."
              >
                <Select
                  style={{ width: 120 }}
                  options={[
                    { label: 'auto', value: 'auto' },
                    { label: 'light', value: 'light' },
                    { label: 'dark', value: 'dark' },
                  ]}
                />
              </Form.Item>
            </Space>
            <Button size="small" style={{ marginBottom: 12 }} onClick={() => refreshModels(form.getFieldValue('default_provider'))}>
              Fetch models
            </Button>
            {models.length > 0 && (
              <Space wrap style={{ marginBottom: 12 }}>
                {models.map((m) => (
                  <Tag key={m} style={{ cursor: 'pointer' }} onClick={() => form.setFieldValue('default_model', m)}>
                    {m}
                  </Tag>
                ))}
              </Space>
            )}
            <Divider style={{ margin: '8px 0' }} />
            <Space size={16} wrap>
              <Form.Item name="build_command" label="Build command">
                <Input style={{ width: 280 }} placeholder="e.g. cargo build" />
              </Form.Item>
              <Form.Item name="default_chip" label="Default chip">
                <Input style={{ width: 220 }} placeholder="nrf52840, stm32f103, …" />
              </Form.Item>
              <Form.Item name="monitor_port" label="Monitor port">
                <Input style={{ width: 160 }} placeholder="COM3" />
              </Form.Item>
              <Form.Item name="monitor_baud" label="Baud">
                <InputNumber min={1200} max={3000000} step={9600} />
              </Form.Item>
            </Space>
            <Form.Item name="auto_approve" label="Auto-approve tools" style={{ maxWidth: 480 }}>
              <Select
                mode="tags"
                placeholder="tool names, e.g. read_file"
                tokenSeparators={[',']}
              />
            </Form.Item>
            <Form.Item name="context_budget_chars" label="Context budget (chars)">
              <InputNumber min={10000} step={10000} style={{ width: 200 }} />
            </Form.Item>
            <Form.Item
              name="web_search"
              label="Web search provider"
              extra="Default: bing (no key, CN-reachable). Set to duckduckgo for international."
            >
              <Select
                allowClear
                placeholder="bing (default)"
                options={[
                  { label: 'bing (default — no key, CN-reachable)', value: 'bing' },
                  { label: 'duckduckgo (no key)', value: 'duckduckgo' },
                  { label: 'tavily (API key)', value: 'tavily' },
                  { label: 'brave (API key)', value: 'brave' },
                ]}
              />
            </Form.Item>
            <ActionButton tier="primary" onClick={save} loading={saving}>
              Save settings
            </ActionButton>
            {saveMsg && <Text type="success" style={{ fontSize: 12 }}>{saveMsg}</Text>}
            {saveErr && <Text type="danger" style={{ fontSize: 12 }}>{saveErr}</Text>}
          </Form>
        </Card>
      </Space>
    </div>
  );
}
