import {
  FolderOpen,
  Moon,
  Play,
  Settings,
  Sun,
  Terminal,
  Trash2,
} from 'lucide-react';
import { useRef, useState } from 'react';
import type { ReactNode } from 'react';

import { currentThemeSetting, setThemeSetting } from '../lib/theme';
import {
  Button,
  Checkbox,
  Chip,
  Confirm,
  confirm,
  CopyButton,
  Drawer,
  EmptyState,
  Field,
  Icon,
  IconButton,
  KeyValue,
  Menu,
  Modal,
  PopConfirm,
  Popover,
  Radio,
  RadioGroup,
  SearchInput,
  Segmented,
  Select,
  Skeleton,
  Slider,
  Spinner,
  Stat,
  Switch,
  Tabs,
  TextArea,
  TextInput,
  Tooltip,
  ToastViewport,
  pushToast,
  useTooltip,
} from '../ui';
import type { MenuEntry } from '../ui';
import styles from './Showcase.module.css';

/** The four chip states, so a row of them says "this is the vocabulary". */
const CHIP_STATES = ['ok', 'failed', 'running', 'attention', 'neutral'] as const;

const MENU_ITEMS: MenuEntry[] = [
  { key: 'open', label: 'Open folder', icon: FolderOpen, hint: 'Ctrl+O' },
  { key: 'run', label: 'Run target', icon: Play },
  { separator: true, key: 'sep' },
  { key: 'settings', label: 'Settings', icon: Settings },
  { key: 'delete', label: 'Delete session', icon: Trash2, danger: true },
];

function Section({ title, note, children }: { title: string; note?: string; children: ReactNode }) {
  return (
    <section className={styles.section}>
      <header className={styles.sectionHead}>
        <h2 className={styles.sectionTitle}>{title}</h2>
        {note ? <p className={styles.sectionHint}>{note}</p> : null}
      </header>
      <div className={styles.sectionBody}>{children}</div>
    </section>
  );
}

function Row({ children }: { children: ReactNode }) {
  return <div className={styles.row}>{children}</div>;
}

/**
 * The primitive gallery, at `?showcase=1`.
 *
 * It exists because a component that has never been drawn is not finished. Stage 1
 * builds twenty-four primitives with no consumer yet, and "it type-checks" is weak
 * evidence for a thing whose whole job is to be looked at -- the chip that pairs two
 * tokens measured against different grounds, the menu that lands off-screen, the
 * focus ring that only appears to a keyboard user, all pass a compiler.
 *
 * It also exists for the light scheme. Every judgement in this rebuild so far was
 * made against the dark ground, and half the bugs in the old tree were
 * light-scheme bugs (`successInk` as a fill, acid text at 1.19:1). Both schemes
 * are one click here, which is the only way that gets checked instead of promised.
 *
 * Dev-only: `main.tsx` reaches it through a `lazy()` behind `import.meta.env.DEV`,
 * so the branch is dead in a production build and the chunk is never fetched.
 */
export function Showcase() {
  const [theme, setTheme] = useState(currentThemeSetting());
  const [text, setText] = useState('');
  const [provider, setProvider] = useState<string | undefined>('openai-main');
  const [switchOn, setSwitchOn] = useState(true);
  const [checked, setChecked] = useState(true);
  const [radio, setRadio] = useState('auto');
  const [slider, setSlider] = useState(35);
  const [segment, setSegment] = useState('chat');
  const [tab, setTab] = useState('changes');

  const [menuOpen, setMenuOpen] = useState(false);
  const [popOpen, setPopOpen] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [popConfirmOpen, setPopConfirmOpen] = useState(false);

  const menuRef = useRef<HTMLButtonElement | null>(null);
  const popRef = useRef<HTMLButtonElement | null>(null);
  const popConfirmRef = useRef<HTMLButtonElement | null>(null);
  const tip = useTooltip<HTMLButtonElement>();

  const applyTheme = (next: string) => {
    setTheme(next as typeof theme);
    // The same setter the app uses: it writes the cache and republishes
    // `data-scheme`, so the gallery switches with the rest of the document.
    setThemeSetting(next);
  };

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className={styles.headWord}>
          <img src="/icons/logo-64.png" alt="" width={28} height={28} />
          <div>
            <strong className={styles.headName}>Firment</strong>
            <span className={styles.kicker}>primitive gallery · dev only</span>
          </div>
        </div>
        <Segmented
          ariaLabel="Colour scheme"
          value={theme}
          onChange={applyTheme}
          options={[
            { value: 'auto', label: 'Auto', icon: Terminal },
            { value: 'light', label: 'Light', icon: Sun },
            { value: 'dark', label: 'Dark', icon: Moon },
          ]}
        />
      </header>

      <main className={styles.grid}>
        <Section title="Type" note="Six steps, named by role. Nothing in the app sets a font-size of its own.">
          <div className={styles.types}>
            <span className={styles.t28}>Firment — display, the wordmark only</span>
            <span className={styles.t14}>ui — control labels, the active tab</span>
            <span className={styles.t13}>body — the default: prose, inputs, card titles</span>
            <span className={styles.t12}>minor — tool args, session meta, status bar</span>
            <span className={styles.t11}>meta — chips, tab labels, mono paths</span>
            <span className={styles.t10}>micro — counters and state words</span>
          </div>
        </Section>

        <Section title="Button" note="Four tiers; the slant belongs to the primary CTA and nowhere else.">
          <Row>
            <Button tier="primary">Save</Button>
            <Button tier="secondary">Cancel</Button>
            <Button tier="ghost">Rename</Button>
            <Button tier="danger">Delete</Button>
          </Row>
          <Row>
            <Button tier="primary" edge="left">
              Run agent
            </Button>
            <Button tier="primary" edge="both">
              Flash
            </Button>
            <Button tier="primary" loading>
              Building
            </Button>
            <Button tier="secondary" disabled>
              Save
            </Button>
          </Row>
          <Row>
            <Button size="sm">sm</Button>
            <Button size="md">md</Button>
            <Button size="lg">lg</Button>
            <IconButton label="Settings" icon={Settings} />
            <IconButton label="Delete" icon={Trash2} tier="danger" size="sm" />
            <span className={styles.inline}>
              <Spinner />
              <Spinner size="lg" label="Flashing" />
            </span>
          </Row>
        </Section>

        <Section title="Chip" note="Fill and ink come from one documented pair, chosen by [data-status].">
          <Row>
            {CHIP_STATES.map((status) => (
              <Chip key={status} status={status}>
                {status}
              </Chip>
            ))}
          </Row>
          <Row>
            <Chip status="ok" size="sm">
              passed
            </Chip>
            <Chip status="neutral" mono>
              feature/gui-rebuild
            </Chip>
            <Chip status="running" icon={Play}>
              running
            </Chip>
            <Chip status="attention">3 warnings</Chip>
          </Row>
        </Section>

        <Section title="Input" note="One frame for text, search and the listbox trigger, so a mixed panel lines up.">
          <Row>
            <TextInput placeholder="Session name" value={text} onChange={(e) => setText(e.target.value)} />
            <TextInput icon={FolderOpen} mono placeholder="build/firment.elf" readOnly value="target/thumbv7em-none-eabihf/debug/firment" />
            <TextInput suffix="baud" value="115200" readOnly />
            <TextInput invalid defaultValue="already taken" />
          </Row>
          <Row>
            <SearchInput placeholder="Filter sessions" />
            <TextArea placeholder="Describe the change…" rows={2} />
          </Row>
          <Row>
            <span className={styles.half}>
              <Field label="Model" hint="Used for reasoning steps." required>
                <TextInput defaultValue="claude-sonnet" mono />
              </Field>
            </span>
            <span className={styles.half}>
              <Field label="API key" error="Not set — the run will fail at the first call.">
                <TextInput type="password" defaultValue="" />
              </Field>
            </span>
          </Row>
        </Section>

        <Section title="Choice" note="Switch, checkbox, radio, segmented, slider — all controlled, none of them re-render a list.">
          <Row>
            <Switch checked={switchOn} onChange={setSwitchOn} label="Verbose logging" />
            <Switch checked={!switchOn} onChange={setSwitchOn} name="Muted switch" />
            <Checkbox checked={checked} onChange={setChecked} label="Include untracked" />
            <Checkbox checked="indeterminate" onChange={() => {}} label="Some files" />
          </Row>
          <Row>
            <RadioGroup value={radio} onChange={setRadio} layout="row" name="Theme">
              <Radio value="auto" label="Auto" />
              <Radio value="light" label="Light" />
              <Radio value="dark" label="Dark" disabled />
            </RadioGroup>
          </Row>
          <Row>
            <Segmented
              ariaLabel="View"
              value={segment}
              onChange={setSegment}
              options={[
                { value: 'chat', label: 'Chat' },
                { value: 'changes', label: 'Changes' },
                { value: 'agents', label: 'Agents' },
              ]}
            />
            <span className={styles.slider}>
              <Slider
                value={slider}
                onChange={setSlider}
                name="Baud"
                format={(v) => `${v}%`}
              />
            </span>
          </Row>
        </Section>

        <Section title="Floating" note="One placement function, one z-ladder, one dismissal rule.">
          <Row>
            <Select
              ariaLabel="Provider"
              value={provider}
              onChange={setProvider}
              mono
              options={[
                { value: 'openai-main', label: 'openai-main', hint: 'https://api.example' },
                { value: 'local', label: 'ollama', hint: '127.0.0.1:11434' },
                { value: 'none', label: 'No provider', disabled: true },
              ]}
            />
            <Button ref={menuRef} tier="secondary" onClick={() => setMenuOpen((v) => !v)} aria-haspopup="menu" aria-expanded={menuOpen}>
              Commands
            </Button>
            <Menu open={menuOpen} anchorRef={menuRef} onClose={() => setMenuOpen(false)} items={MENU_ITEMS} />
            <Button ref={popRef} tier="secondary" onClick={() => setPopOpen((v) => !v)}>
              Popover
            </Button>
            <Popover open={popOpen} anchorRef={popRef} onClose={() => setPopOpen(false)} padded role="dialog">
              <p className={styles.popText}>A panel holding prose rather than a flush list of rows.</p>
            </Popover>
            <Button
              ref={popConfirmRef}
              tier="danger"
              icon={Trash2}
              onClick={() => setPopConfirmOpen((v) => !v)}
            >
              PopConfirm
            </Button>
            <PopConfirm
              open={popConfirmOpen}
              anchorRef={popConfirmRef}
              onClose={() => setPopConfirmOpen(false)}
              onConfirm={() => {
                setPopConfirmOpen(false);
                pushToast('Session deleted', 'failed');
              }}
              tone="danger"
              title="Delete this session?"
              message="The transcript and its tool output go with it. This cannot be undone."
              confirmLabel="Delete"
            />
            <button
              ref={tip.anchorRef}
              type="button"
              className={styles.tipHost}
              {...tip.triggerProps}
            >
              Hover for a tooltip
            </button>
            <Tooltip tip={tip} text="400ms in, nothing mounted until then." />
          </Row>
        </Section>

        <Section title="Tabs" note="The strip only: the panes belong to the shell that owns the grid.">
          <Tabs
            ariaLabel="Inspector"
            active={tab}
            onChange={setTab}
            items={[
              { key: 'changes', label: 'Changes', icon: FolderOpen, meta: '4' },
              { key: 'agents', label: 'Agents', icon: Play, meta: '2' },
              { key: 'todos', label: 'Todos', meta: '7' },
              { key: 'memory', label: 'Memory', disabled: true },
            ]}
          />
        </Section>

        <Section title="Windows" note="Modal, Drawer, Confirm and the promise form — one sheet, two positions.">
          <Row>
            <Button onClick={() => setModalOpen(true)}>Modal</Button>
            <Button onClick={() => setDrawerOpen(true)}>Drawer</Button>
            <Button onClick={() => setConfirmOpen(true)}>Confirm</Button>
            <Button
              tier="danger"
              onClick={() => {
                void confirm({
                  title: 'Discard the draft?',
                  message: 'It was modified by the agent since you opened it.',
                  confirmLabel: 'Discard',
                  tone: 'danger',
                }).then((ok) => pushToast(ok ? 'Draft discarded' : 'Draft kept', ok ? 'failed' : 'info'));
              }}
            >
              confirm()
            </Button>
            <Button
              tier="ghost"
              onClick={() => {
                pushToast('Saved to .env', 'ok');
                pushToast('The port is busy — serial is not responding', 'attention');
                pushToast('Build failed: 2 errors', 'failed');
              }}
            >
              Three toasts
            </Button>
          </Row>
          <Modal
            open={modalOpen}
            title="New provider"
            onClose={() => setModalOpen(false)}
            footer={
              <>
                <Button tier="ghost" onClick={() => setModalOpen(false)}>
                  Cancel
                </Button>
                <Button tier="primary" onClick={() => setModalOpen(false)}>
                  Add
                </Button>
              </>
            }
          >
            <Field label="Base URL" hint="The vendor's OpenAI-compatible endpoint.">
              <TextInput mono placeholder="https://api.example.com/v1" />
            </Field>
          </Modal>
          <Drawer open={drawerOpen} title="Settings" onClose={() => setDrawerOpen(false)}>
            <Field label="Model" inline hint="Leave blank to use the provider default.">
              <TextInput defaultValue="claude-sonnet" mono />
            </Field>
            <Skeleton lines={4} />
          </Drawer>
          <Confirm
            open={confirmOpen}
            title="Reload from disk?"
            message="The knowledge file changed while the draft was open. Your edits will be replaced."
            confirmLabel="Reload"
            cancelLabel="Keep draft"
            onConfirm={() => {
              setConfirmOpen(false);
              pushToast('Reloaded from disk', 'ok');
            }}
            onCancel={() => setConfirmOpen(false)}
          />
        </Section>

        <Section title="Content" note="What a pane shows when it has nothing, something, or a number.">
          <Row>
            <span className={styles.half}>
              <KeyValue label="Provider" value="openai-main" mono />
              <KeyValue label="Flash size" value="131072 bytes" />
              <KeyValue label="Path" value="/d/OldStudy66/Firment/target/debug/firment.exe" mono />
            </span>
            <span className={styles.stats}>
              <Stat value="47" label="steps" hint="2 failed" />
              <Stat value="128.0 KiB" label="flash" hint="of 256 KiB" />
            </span>
            <CopyButton value="B4F779B4F779" />
          </Row>
          <Row>
            <span className={styles.skeletonBox}>
              <Skeleton lines={3} />
            </span>
            <span className={styles.skeletonBox}>
              <Skeleton lines={1} last="full" />
            </span>
          </Row>
          <div className={styles.emptyBox}>
            <EmptyState
              icon={FolderOpen}
              title="No sessions in this folder"
              hint="Open a project to start a run — the transcript is kept per folder, not per session."
              action={
                <Button tier="primary" icon={FolderOpen}>
                  Open folder
                </Button>
              }
            />
          </div>
        </Section>
      </main>

      <footer className={styles.foot}>
        <Icon src={Terminal} tone="muted" /> every value on this page comes from styles/tokens.css — a literal here is
        a bug, not a shortcut.
      </footer>
      <ToastViewport />
    </div>
  );
}
