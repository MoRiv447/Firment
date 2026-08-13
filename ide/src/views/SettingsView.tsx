import { Alert, Button, Card, Form, Input, InputNumber, Select, Space, Tag, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { api } from '../lib/api';
import type { SettingsDto } from '../types';

const { Text } = Typography;

export function SettingsView() {
  const [settings, setSettings] = useState<SettingsDto | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);
  const [keyProvider, setKeyProvider] = useState('openai');
  const [keyValue, setKeyValue] = useState('');
  const [keyMsg, setKeyMsg] = useState('');
  const [form] = Form.useForm<SettingsDto>();

  useEffect(() => {
    void api.getSettings().then((s) => {
      setSettings(s);
      form.setFieldsValue(s);
    });
  }, [form]);

  const refreshModels = async (provider: string) => {
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
    try {
      const values = form.getFieldsValue() as SettingsDto;
      await api.saveSettings(values);
      setSaving(false);
    } catch (err) {
      setSaving(false);
      console.error(err);
    }
  };

  const saveKey = async () => {
    if (!keyValue.trim()) return;
    try {
      await api.setApiKey(keyProvider, keyValue.trim());
      setKeyValue('');
      setKeyMsg(`saved for ${keyProvider}`);
    } catch (err) {
      console.error(err);
    }
  };

  return (
    <div style={{ padding: 20, maxWidth: 760 }}>
      <Space direction="vertical" size={16} style={{ width: '100%' }}>
        {!settings && <Alert type="info" showIcon message="Loading settings…" />}

        <Card title="API key" size="small">
          <Space direction="vertical" style={{ width: '100%' }}>
            <Space>
              <Select
                style={{ width: 180 }}
                placeholder="provider"
                value={keyProvider}
                onChange={(p) => setKeyProvider(p ?? 'openai')}
                options={[{ label: 'openai', value: 'openai' }, { label: 'anthropic', value: 'anthropic' }]}
              />
              <Input.Password
                style={{ width: 320 }}
                placeholder="sk-…"
                value={keyValue}
                onChange={(e) => setKeyValue(e.target.value)}
              />
              <Button onClick={saveKey}>Save</Button>
            </Space>
            {keyMsg && <Text type="success">{keyMsg}</Text>}
            <Text type="secondary" style={{ fontSize: 12 }}>
              Keys are stored in auth.json next to config.toml. Use the /models flow below to
              verify connectivity.
            </Text>
          </Space>
        </Card>

        <Card title="Agent" size="small">
          <Form form={form} layout="vertical">
            <Space size={16} wrap>
              <Form.Item name="default_provider" label="Default provider" style={{ minWidth: 180 }}>
                <Select
                  options={[{ label: 'openai', value: 'openai' }, { label: 'anthropic', value: 'anthropic' }]}
                  onChange={(v) => refreshModels(v)}
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
          </Form>
        </Card>

        <Card title="Workspace / tools" size="small">
          <Form form={form} layout="vertical">
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
          </Form>
        </Card>

        <Button type="primary" onClick={save} loading={saving}>
          Save settings
        </Button>
      </Space>
    </div>
  );
}