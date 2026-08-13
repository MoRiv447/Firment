import { useState } from 'react';
import { Alert, Button, Modal, Space, Typography } from 'antd';
import { api } from '../lib/api';
import { ToolCard } from './ToolCard';
import type { PermissionRequest, ToolCardState } from '../types';

const { Text } = Typography;

export function PermissionDialog({ req }: { req: PermissionRequest }) {
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
      <ToolCard tool={toolCard} standalone />
      {req.reason && (
        <Alert style={{ marginTop: 8 }} type="info" showIcon message={<Text>{req.reason}</Text>} />
      )}
    </Modal>
  );
}

export function AskDialog({
  req,
}: {
  req: { id: number; question: string; options: string[] };
}) {
  const [busy, setBusy] = useState(false);
  const respond = async (answer: string | null) => {
    setBusy(true);
    try {
      await api.respondAsk(req.id, answer);
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
          <Button loading={busy} onClick={() => respond(null)}>
            Dismiss
          </Button>
        )
      }
    >
      <Text>{req.question}</Text>
    </Modal>
  );
}