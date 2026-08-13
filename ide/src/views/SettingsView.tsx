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
  Popconfirm,
  Divider,
} from 'antd';
import { DeleteOutlined, PlusOutlined } from '@ant-design/icons';
import { useEffect, useState } from 'react';
import { api } from '../lib/api';
import type { ProviderEntryDto, SettingsDto } from '../types';

const { Text } = Typography;

export function SettingsView() {
  const [settings, setSettings] = useState<SettingsDto | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);
  const [keyMsg, setKeyMsg] = useState('');
  const [saveMsg, setSaveMsg] = useState('');
  const [saveErr, setSaveErr] = useState('');
  const [form] = Form.useForm<SettingsDto>();

  // new-provider form
  const [newName, setNewName] = useState('');
  const [newType, setNewType] = useState('openai');
  const [newBaseUrl, setNewBaseUrl] = useState('');
  const [newModel, setNewModel] = useState('');
  const [newMsg, setNewMsg] = useState('');

  const load = () => {
    void api.getSettings().then((s) => {
      setSettings(s);
      form.setFieldsValue(s);
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
    setSaving(true);
    setSaveMsg('');
    setSaveErr('');
    try {
      const values = form.getFieldsValue() as SettingsDto;
      // providers is not part of the antd form; carry it over from state
      values.providers = settings?.providers ?? [];
      await api.saveSettings(values);
      setSaving(false);
      setSaveMsg('saved ✓');
      load();
    } catch (err) {
      setSaving(false);
      setSaveErr(`save failed: ${err}`);
      console.error(err);
    }
  };

  const upsertProvider = async () => {
    const name = newName.trim();
    if (!name || !newModel.trim()) {
      setNewMsg('name and model are required');
      return;
    }
    try {
      await api.setProvider(name, newType, newBaseUrl.trim() || null, newModel.trim());
      setNewMsg(`saved provider "${name}"`);
      setNewName('');
      setNewModel('');
      setNewBaseUrl('');
      load();
    } catch (err) {
      setNewMsg(`failed: ${err}`);
      console.error(err);
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

  // per-provider API key editing: update local state on change, persist on save
  const setProviderKeyLocal = (p: ProviderEntryDto, key: string) => {
    setSettings((s) =>
      s
        ? {
            ...s,
            providers: s.providers.map((x) => (x.name === p.name ? { ...x, api_key: key } : x)),
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

        <Card title="Providers" size="small">
          <Space direction="vertical" size={12} style={{ width: '100%' }}>
            <Text type="secondary" style={{ fontSize: 12 }}>
              Each provider has its own API key, base URL and model. The default provider is
              used for new sessions; delete any provider — the default automatically moves to
              the next one.
            </Text>
            {(settings?.providers ?? []).map((p) => (
              <div
                key={p.name}
                style={{
                  border: '2px solid #000',
                  padding: 10,
                  background: '#12151b',
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 8,
                }}
              >
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                  <Text strong style={{ color: '#f2f4f8' }}>{p.name}</Text>
                  {p.is_default && (
                    <Tag color="#facc15" style={{ borderRadius: 0, border: '2px solid #000', color: '#000', fontWeight: 700 }}>
                      DEFAULT
                    </Tag>
                  )}
                  <div style={{ flex: 1 }} />
                  <Popconfirm
                    title={`Delete "${p.name}"?`}
                    description="The default (if this one) moves to another provider."
                    onConfirm={() => removeProvider(p)}
                  >
                    <Button size="small" danger icon={<DeleteOutlined />} />
                  </Popconfirm>
                </div>
                <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                  <Select
                    style={{ width: 120 }}
                    value={p.type}
                    onChange={(v) => editProvider({ ...p, type: v })}
                    options={[{ label: 'openai', value: 'openai' }, { label: 'anthropic', value: 'anthropic' }]}
                  />
                  <Input
                    style={{ flex: 1, minWidth: 200, fontFamily: 'Consolas, monospace' }}
                    placeholder="base url"
                    value={p.base_url ?? ''}
                    onChange={(e) => setProviderLocal(p, { base_url: e.target.value || null })}
                    onBlur={() => editProvider(p)}
                  />
                  <Input
                    style={{ flex: 1, minWidth: 140 }}
                    placeholder="model"
                    value={p.model}
                    onChange={(e) => setProviderLocal(p, { model: e.target.value })}
                    onBlur={() => editProvider(p)}
                  />
                </div>
                <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'center' }}>
                  <Input.Password
                    style={{ flex: 1, minWidth: 260, fontFamily: 'Consolas, monospace' }}
                    placeholder={`api key for ${p.name} (empty = use env)`}
                    value={p.api_key ?? ''}
                    onChange={(e) => setProviderKeyLocal(p, e.target.value)}
                  />
                  <Button size="small" onClick={() => saveProviderKey(p)}>
                    Save key
                  </Button>
                </div>
              </div>
            ))}
            <Divider style={{ margin: '4px 0' }} />
            <Text strong style={{ color: '#e6e9ef' }}>Add provider</Text>
            <Space wrap>
              <Input
                style={{ width: 130 }}
                placeholder="name (e.g. deepseek)"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
              />
              <Select
                style={{ width: 120 }}
                value={newType}
                onChange={(v) => setNewType(v ?? 'openai')}
                options={[{ label: 'openai', value: 'openai' }, { label: 'anthropic', value: 'anthropic' }]}
              />
              <Input
                style={{ width: 260, fontFamily: 'Consolas, monospace' }}
                placeholder="base url (e.g. https://api.deepseek.com/v1)"
                value={newBaseUrl}
                onChange={(e) => setNewBaseUrl(e.target.value)}
              />
              <Input
                style={{ width: 180 }}
                placeholder="model (e.g. deepseek-v4-flash)"
                value={newModel}
                onChange={(e) => setNewModel(e.target.value)}
              />
              <Button type="primary" icon={<PlusOutlined />} onClick={upsertProvider}>
                Save provider
              </Button>
            </Space>
            {newMsg && <Text type="secondary" style={{ fontSize: 12 }}>{newMsg}</Text>}
            {keyMsg && <Text type="success" style={{ fontSize: 12 }}>{keyMsg}</Text>}
            <Text type="secondary" style={{ fontSize: 12 }}>
              Providers are stored in config.toml; keys in auth.json. Pick the default in the
              Agent card below.
            </Text>
          </Space>
        </Card>

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
            <Form.Item name="web_search" label="Web search provider">
              <Select
                allowClear
                placeholder="none"
                options={['duckduckgo', 'tavily', 'brave'].map((t) => ({ label: t, value: t }))}
              />
            </Form.Item>
            <Button type="primary" onClick={save} loading={saving}>
              Save settings
            </Button>
            {saveMsg && <Text type="success" style={{ fontSize: 12 }}>{saveMsg}</Text>}
            {saveErr && <Text type="danger" style={{ fontSize: 12 }}>{saveErr}</Text>}
          </Form>
        </Card>
      </Space>
    </div>
  );
}
