import {
  FolderOpen,
  Moon,
  Play,
  Settings,
  Sun,
  Terminal,
  Trash2,
} from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';

import {
  currentThemeSetting,
  publishScheme,
  resolveTheme,
  setThemeSetting,
  systemPrefersDark,
} from '../lib/theme';
import {
  Button,
  Callout,
  Card,
  Checkbox,
  Chip,
  Confirm,
  confirm,
  CopyButton,
  Drawer,
  EmptyState,
  Eyebrow,
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
  StatusMark,
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
import { TurnTimeline } from '../components/TurnTimeline';
import { LiveRun } from '../components/LiveRun';
import { ChatView } from '../views/ChatView';
import { ToolCard } from '../components/ToolCard';
import { TurnVerdict } from '../components/TurnVerdict';
import { ToolRun } from '../components/MessageList';
import type { ChatMessage, SessionDto, ToolCardState } from '../types';
import styles from './Showcase.module.css';

/**
 * Three turns that never happened, on a clock that never moves.
 *
 * A proportion cannot be judged against data that re-randomises on every render,
 * and the only thing worth reviewing here is the ratio between the three parts.
 */
const SHOWCASE_NOW = 1_700_000_000_000;
const at = (ms: number) => SHOWCASE_NOW + ms;
function shown(
  seq: number,
  name: string,
  from: number,
  to: number,
  waitedMs?: number,
): ToolCardState {
  return { seq, name, args: {}, status: 'ok', startedAt: at(from), endedAt: at(to), waitedMs };
}

/** A build, then a flash: the bar is nearly all one colour, and that is the boring case. */
const FLASH_TURN = [shown(1, 'build', 2_000, 42_000), shown(2, 'flash', 42_000, 50_000)];
/** A flash approved two minutes after it was offered: the wait dominates, the tool did not. */
const APPROVAL_TURN = [shown(1, 'flash', 2_000, 130_000, 120_000)];
/** A turn that spent almost all of its time before any call ran — the model's turn. */
const READING_TURN = [shown(1, 'read_file', 90_000, 95_000)];

/**
 * A live run, in all four row states at once.
 *
 * The states are the reason this fixture exists: a column of ticks tells you nothing
 * about whether the layout works, and the four marks have to be told apart down a
 * single column at 14px. `unknown` is here on purpose -- it is the reopened-session
 * case and it must read as "not run", never as a failure.
 */
const STACK_RUN: ToolCardState[] = [
  { seq: 1, name: 'read_file', args: { path: 'src/main.c' }, status: 'ok', startedAt: at(0), endedAt: at(400) },
  {
    seq: 2,
    name: 'edit_file',
    args: { path: 'src/main.c' },
    status: 'ok',
    startedAt: at(400),
    endedAt: at(1_600),
    detail: '@@ -12,3 +12,4 @@\n-HAL_Delay(500);\n+__HAL_TIM_SET_COMPARE(&htim2, TIM_CHANNEL_1, duty);',
  },
  { seq: 3, name: 'build', args: { cmd: 'cmake --build build' }, status: 'running', startedAt: at(1_600), progress: 'linking' },
  { seq: 4, name: 'flash', args: { elf: 'firmware.elf' }, status: 'unknown' },
];

/**
 * The three card states the design shows side by side.
 *
 * These are here because the card is the one object in the transcript that has a
 * body, a footer and an edge that all move together, and until now the gallery drew
 * the row but never the card -- which is how "the failed body looks like an
 * attachment" survived a whole restyle unseen by anyone.
 */
const CARD_RUN: ToolCardState[] = [
  {
    seq: 5,
    name: 'build',
    args: { cmd: 'cmake --build build --target firmware' },
    status: 'running',
    startedAt: at(0),
    progress: 'compiling',
    detail:
      '[ 62%] Building C object CMakeFiles/firmware.dir/src/main.c.obj\n[ 78%] Linking C executable firmware.elf',
  },
  {
    seq: 6,
    name: 'monitor',
    args: { port: '/dev/ttyUSB0', baud: 115200 },
    status: 'failed',
    startedAt: at(0),
    endedAt: at(2_000),
    summary: 'no data in 2.0s',
    detail:
      'expected "LED ON" x2, saw 0 lines in 2.0s\nport opened, but no data -- check TX/RX and the baud rate',
  },
  {
    seq: 7,
    name: 'edit_file',
    args: { path: 'src/main.c' },
    status: 'ok',
    startedAt: at(0),
    endedAt: at(200),
    detail:
      '@@ -30,1 +30,3 @@\n-HAL_Delay(500);\n+__HAL_TIM_SET_COMPARE(&htim2, TIM_CHANNEL_1, duty);\n+HAL_Delay(10);',
  },
];

/** A call in flight, so the live turn can be seen with something in it. */
const LIVE_TURN: ToolCardState = {
  seq: 8,
  name: 'build',
  args: { cmd: 'cmake --build build' },
  status: 'running',
  startedAt: at(0),
  progress: 'compiling',
};

/** A finished run, for the band that ends one. */
const VERDICT_TURN = [
  shown(1, 'build', 0, 1_400),
  shown(2, 'flash', 1_400, 9_400),
  shown(3, 'monitor', 9_400, 12_400),
];

/** A chat with one exchange in it, so the composer is seen in place, not alone. */
const COMPOSER_SESSION: SessionDto = {
  id: 'showcase',
  cwd: 'D:/OldStudy66/alt_testface',
  provider: 'glm',
  model: 'glm-5.3-flash',
  mode: 'agent',
  thinking: 'max',
  created_at: 0,
  updated_at: 0,
  messages: [
    { role: 'user', content: 'PA0 上有颗 LED。给它写一个 1 kHz 的 PWM 呼吸灯。' },
    {
      role: 'assistant',
      content: '先确认工具链，再初始化 TIM2 通道 1，最后把固定延时换成占空比渐变。',
      tool_calls: [{ id: 'a', name: 'edit_file', arguments: { path: 'src/main.c' } }],
    },
    { role: 'tool', content: 'Edited src/main.c (42 lines -> 76 lines)', tool_call_id: 'a' },
  ],
};

/** A finished run as the store keeps it: the calls, their text, and no outcome. */
const HISTORY_RUN: ChatMessage[] = [
  {
    role: 'assistant',
    content: '',
    tool_calls: [
      { id: 'a', name: 'read_file', arguments: { path: 'src/main.c' } },
      { id: 'b', name: 'read_file', arguments: { path: 'src/tim.c' } },
    ],
  },
  { role: 'tool', content: 'the file', tool_call_id: 'a' },
  { role: 'tool', content: 'the other file', tool_call_id: 'b' },
];

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

/** Card-sized things, stacked at the gap the cards themselves use. */
function Stack({ children }: { children: ReactNode }) {
  return <div className={styles.stack}>{children}</div>;
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

  /*
   * The gallery paints its own scheme, and it did not.
   *
   * `setThemeSetting` only tells subscribers; the write to `data-scheme` lives in
   * `App`, and this page renders INSTEAD of `App`. So the segmented control set the
   * store, announced it to nobody, and left the document in whatever scheme
   * `index.html` had guessed -- three buttons that did nothing, in the one place
   * both schemes are supposed to be reviewed side by side.
   */
  useEffect(() => {
    publishScheme(theme, resolveTheme(theme, systemPrefersDark()));
  }, [theme]);

  const applyTheme = (next: string) => {
    setTheme(next as typeof theme);
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

        <Section
          title="Button"
          note="Five tiers. A fill is a call to action, an edge is a control, bare text is an offer."
        >
          <Row>
            <Button tier="primary">Save</Button>
            <Button tier="secondary">Cancel</Button>
            <Button tier="ghost">Rename</Button>
            <Button tier="quiet">View the run</Button>
            <Button tier="danger">Delete</Button>
          </Row>
          <Row>
            {/* Two slanted primaries sat here. They were the same button
                twice once the cut went, so the gallery shows one. */}
            <Button tier="primary">Run agent</Button>
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

        <Section title="Callout" note="Three tones; each fill and ink pair is taken from one [data-tone] rule.">
          <div className={styles.types}>
            <Callout>Provider key verified against the endpoint.</Callout>
            <Callout tone="warn" title="The agent is waiting">
              No event for 132s — the stream budget is 120s, so this turn may be wedged.
            </Callout>
            <Callout tone="failed" mono title="error: failed to open .cargo-build-lock">
              {'拒绝访问。 (os error 5)\nretry with CARGO_BUILD_JOBS=1'}
            </Callout>
          </div>
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

        <Section title="Turn timeline" note="One wall clock, attributed once: tools, a person at a dialog, and the model.">
          <span className={styles.half}>
            <TurnTimeline tools={FLASH_TURN} now={SHOWCASE_NOW} turnStartedAt={SHOWCASE_NOW} />
            <TurnTimeline tools={APPROVAL_TURN} now={SHOWCASE_NOW} turnStartedAt={SHOWCASE_NOW} />
            <TurnTimeline tools={READING_TURN} now={SHOWCASE_NOW} turnStartedAt={SHOWCASE_NOW} />
          </span>
        </Section>

        <Section title="Tool work" note="One row per call while it runs; one band once it is over.">
          <span className={styles.half}>
            {/* The clock is placed just after the build started, so the running row
                shows a real fraction of a second rather than a negative one clamped
                to zero. `LiveRun` takes `now` instead of reading it, which is what
                makes an instant like this expressible at all. */}
            <LiveRun tools={STACK_RUN} now={at(2_100)} turnStartedAt={at(0)} />
          </span>
          <span className={styles.half}>
            <ToolRun messages={HISTORY_RUN} />
          </span>
          <span className={styles.line}>
            {(['done', 'current', 'pending', 'failed'] as const).map((state) => (
              <StatusMark key={state} state={state} label={state} />
            ))}
            <Eyebrow latin>Sessions</Eyebrow>
            {/* The same label in the other script: no case to change, and the CJK
                tracking that the default carries. */}
            <Eyebrow upper={false} latin={false}>
              本轮进度
            </Eyebrow>
          </span>
        </Section>

        <Section
          title="Tool card"
          note="The same object in its three states: a log while it runs, a failure that reads as one, and an edit with a way to the pane that holds it."
        >
          <Stack>
            {CARD_RUN.map((t) => (
              <ToolCard key={t.seq} tool={t} onOpenChanges={() => {}} />
            ))}
            <TurnVerdict tools={VERDICT_TURN} now={at(12_400)} turnStartedAt={at(0)} />
          </Stack>
        </Section>

        <Section
          title="Composer"
          note="The pane's own bottom edge: the field, the settings inside it, and the square action that swaps between send and stop."
        >
          <Stack>
            <div className={styles.pane}>
              <ChatView
                now={at(2_100)}
                session={COMPOSER_SESSION}
                running={false}
                turn={null}
                infos={[]}
                onSend={() => {}}
                onCancel={() => {}}
              />
            </div>
            <div className={styles.pane}>
              <ChatView
                now={at(2_100)}
                session={COMPOSER_SESSION}
                running
                turn={{
                  text: '',
                  tools: { 8: LIVE_TURN },
                  thinking: '',
                  startedAt: at(0),
                  finished: false,
                }}
                infos={[]}
                onSend={() => {}}
                onCancel={() => {}}
              />
            </div>
          </Stack>
        </Section>

        <Section title="Card" note="Head, body, one hairline. The second has no extra, so the head cannot lean on it.">
          <div className={styles.cards}>
            <Card title="Flash history" extra=".firment/work/flash-history.jsonl">
              <span className={styles.line}>
                <Chip size="sm" status="ok">
                  OK
                </Chip>
                <code>stm32f407vet6</code>
              </span>
              <span className={styles.line}>
                <Chip size="sm" status="failed">
                  FAIL
                </Chip>
                <code>no probe found</code>
              </span>
            </Card>
            <Card title="Decisions (ADR-lite)">
              <KeyValue label="HSE 8 MHz" value="the crystal holds -20°C" />
            </Card>
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
