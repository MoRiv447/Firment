import { useState } from 'react';
import { Alert, Button, Input, Modal, Space, Tag, Typography } from 'antd';
import { api } from '../lib/api';
import { ToolCard } from './ToolCard';
import type { AskRequest, PermissionRequest, ToolCardState } from '../types';

const { Text } = Typography;

/** Which chat's agent is asking — matters once several chats run in parallel. */
function SessionChip({ sid }: { sid?: string }) {
  if (!sid) return null;
  return (
    <Tag
      color="#2f6bff"
      style={{ borderRadius: 0, border: '2px solid #000', fontWeight: 700 }}
    >
      chat {sid.slice(0, 8)}
    </Tag>
  );
}

export function PermissionDialog({
  req,
  onClose,
}: {
  req: PermissionRequest;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const toolCard: ToolCardState = {
    seq: 0,
    name: req.tool,
    args: req.args,
    status: 'running',
  };
  const respond = async (allowed: boolean) => {
    setBusy(true);
    try {
      await api.respondPermission(req.id, allowed);
      onClose();
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal
      open
      title="Permission requested"
      closable={false}
      maskClosable={false}
      footer={
        <Space>
          <Button type="primary" danger loading={busy} onClick={() => respond(false)}>
            Deny
          </Button>
          <Button type="primary" loading={busy} onClick={() => respond(true)}>
            Allow
          </Button>
        </Space>
      }
    >
      <Space size={6} style={{ marginBottom: 8 }}>
        <SessionChip sid={req.session_id} />
        <Text type="secondary" style={{ fontSize: 12 }}>
          This chat wants to run a tool before continuing.
        </Text>
      </Space>
      <ToolCard tool={toolCard} standalone />
      {req.reason && (
        <Alert style={{ marginTop: 8 }} type="info" showIcon message={<Text>{req.reason}</Text>} />
      )}
    </Modal>
  );
}

export function AskDialog({
  req,
  onClose,
}: {
  req: AskRequest;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [custom, setCustom] = useState('');
  const respond = async (answer: string | null) => {
    if (answer !== null && !answer.trim()) return;
    setBusy(true);
    try {
      await api.respondAsk(req.id, answer);
      setCustom('');
      onClose();
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal
      open
      title="Question from agent"
      closable={false}
      maskClosable={false}
      width={480}
      onCancel={() => respond(null)}
      footer={
        req.options.length > 0 ? (
          <Space wrap>
            {req.options.map((o) => (
              <Button
                key={o}
                loading={busy}
                onClick={() => respond(o)}
              >
                {o}
              </Button>
            ))}
            <Button loading={busy} onClick={() => respond(null)}>
              Dismiss
            </Button>
          </Space>
        ) : (
          <Space>
            <Button
              type="primary"
              loading={busy}
              disabled={!custom.trim()}
              onClick={() => respond(custom)}
            >
              Reply
            </Button>
            <Button loading={busy} onClick={() => respond(null)}>
              Dismiss
            </Button>
          </Space>
        )
      }
    >
      <Space size={6} style={{ marginBottom: 8 }}>
        <SessionChip sid={req.session_id} />
      </Space>
      <Text>{req.question}</Text>
      {req.options.length === 0 && (
        <Input.TextArea
          style={{ marginTop: 10 }}
          rows={2}
          placeholder="Type your answer and press Send (or press Enter)"
          value={custom}
          onChange={(e) => setCustom(e.target.value)}
          onPressEnter={(e) => {
            if (!e.shiftKey) respond(custom);
          }}
          autoFocus
        />
      )}
    </Modal>
  );
}