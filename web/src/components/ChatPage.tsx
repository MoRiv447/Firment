'use client';

import { useState, useRef, useEffect, useCallback } from 'react';
import {
  Send,
  Plus,
  MessageSquare,
  Trash2,
  Settings,
  User,
  Sparkles,
  Zap,
  Shield,
  FileText,
  Globe,
  Search,
  Code,
  Loader2,
  PlusCircle,
  Trash,
  Key,
  Link as LinkIcon,
  Wrench,
} from 'lucide-react';
import { Config, ProviderConfig, getStoredConfig, saveConfig } from '@/lib/config';
import { ChatMessage } from '@/lib/types';
import {
  LocalSession,
  loadSessions,
  saveSessions,
  loadCurrentId,
  saveCurrentId,
  createSession,
} from '@/lib/localSessions';

export default function ChatPage() {
  const [sessions, setSessions] = useState<LocalSession[]>([]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [input, setInput] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [config, setConfig] = useState<Config>(getStoredConfig());
  const [liveAssistant, setLiveAssistant] = useState<{ content: string; tool_calls: any[] } | null>(null);
  const [toolStatus, setToolStatus] = useState<string>('');

  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    const loaded = loadSessions();
    if (loaded.length === 0) {
      const s = createSession();
      const list = [s];
      setSessions(list);
      saveSessions(list);
      saveCurrentId(s.id);
      setCurrentId(s.id);
    } else {
      setSessions(loaded);
      const cur = loadCurrentId();
      setCurrentId(cur && loaded.some((s) => s.id === cur) ? cur : loaded[0].id);
    }
    setConfig(getStoredConfig());
  }, []);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [sessions, liveAssistant]);

  const current = sessions.find((s) => s.id === currentId) || null;
  const displayMessages: ChatMessage[] = current
    ? current.messages.filter((m) => m.role !== 'tool')
    : [];
  const renderMessages: ChatMessage[] = liveAssistant
    ? [...displayMessages, { role: 'assistant', content: liveAssistant.content, tool_calls: liveAssistant.tool_calls }]
    : displayMessages;

  const upsertSession = useCallback((updated: LocalSession) => {
    setSessions((prev) => {
      const exists = prev.some((s) => s.id === updated.id);
      const next = exists ? prev.map((s) => (s.id === updated.id ? updated : s)) : [updated, ...prev];
      next.sort((a, b) => b.updatedAt - a.updatedAt);
      saveSessions(next);
      return next;
    });
  }, []);

  function createNewSession() {
    const s = createSession();
    setSessions((prev) => {
      const next = [s, ...prev];
      saveSessions(next);
      return next;
    });
    saveCurrentId(s.id);
    setCurrentId(s.id);
    setLiveAssistant(null);
    setToolStatus('');
    setSidebarOpen(false);
  }

  function selectSession(id: string) {
    saveCurrentId(id);
    setCurrentId(id);
    setLiveAssistant(null);
    setToolStatus('');
    setSidebarOpen(false);
  }

  function deleteSession(id: string) {
    const next = sessions.filter((s) => s.id !== id);
    saveSessions(next);
    setSessions(next);
    if (currentId === id) {
      const fallback = next[0]?.id || null;
      saveCurrentId(fallback);
      setCurrentId(fallback);
    }
  }

  function updateConfig(newConfig: Config) {
    setConfig(newConfig);
    saveConfig(newConfig);
  }

  function addProvider() {
    const name = `provider-${Object.keys(config.providers).length + 1}`;
    const newProviders = {
      ...config.providers,
      [name]: {
        type: 'openai' as const,
        baseUrl: '',
        apiKey: '',
        model: '',
      },
    };
    updateConfig({ ...config, providers: newProviders, defaultProvider: name });
  }

  function removeProvider(name: string) {
    const newProviders = { ...config.providers };
    delete newProviders[name];
    const newDefault = config.defaultProvider === name ? Object.keys(newProviders)[0] || '' : config.defaultProvider;
    updateConfig({ ...config, providers: newProviders, defaultProvider: newDefault });
  }

  function updateProvider(providerName: string, updates: Partial<ProviderConfig>) {
    const current = config.providers[providerName];
    if (!current) return;
    const newProviders = {
      ...config.providers,
      [providerName]: { ...current, ...updates },
    };
    updateConfig({ ...config, providers: newProviders });
  }

  async function sendMessage(text: string) {
    if (!text.trim() || isLoading) return;
    const session = sessions.find((s) => s.id === currentId);
    if (!session) return;

    const userMsg: ChatMessage = { role: 'user', content: text };
    const title = session.title === 'New chat' ? text.slice(0, 30) : session.title;
    const base: LocalSession = {
      ...session,
      title,
      updatedAt: Date.now(),
      messages: [...session.messages, userMsg],
    };
    upsertSession(base);

    setInput('');
    setIsLoading(true);
    setLiveAssistant({ content: '', tool_calls: [] });
    setToolStatus('');

    try {
      const res = await fetch('/api/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          messages: base.messages,
          userInput: text,
          // 只发送当前默认 provider 的配置，避免把所有 provider 的 API key
          // 一次性全部暴露给服务端。
          config: {
            providers: {
              [config.defaultProvider]: config.providers[config.defaultProvider],
            },
            defaultProvider: config.defaultProvider,
            tools: config.tools,
            maxIterations: config.maxIterations,
            contextBudgetChars: config.contextBudgetChars,
            thinking: config.thinking,
          },
        }),
      });

      if (!res.ok || !res.body) {
        const data = await res.json().catch(() => ({}));
        throw new Error(data.error || `HTTP ${res.status}`);
      }

      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';
      let streamed = '';

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });

        const frames = buffer.split('\n\n');
        buffer = frames.pop() || '';

        for (const frame of frames) {
          const line = frame.startsWith('data: ') ? frame.slice(6) : frame;
          if (!line.trim()) continue;
          let evt: any;
          try {
            evt = JSON.parse(line);
          } catch {
            continue;
          }

          if (evt.type === 'text_delta' && evt.text) {
            streamed += evt.text;
            setLiveAssistant({ content: streamed, tool_calls: [] });
          } else if (evt.type === 'tool_start') {
            setToolStatus(`🔧 ${evt.toolName} …`);
          } else if (evt.type === 'tool_end') {
            setToolStatus(evt.toolOk === false ? `⚠️ ${evt.toolName} failed` : `✅ ${evt.toolName}`);
          } else if (evt.type === 'done') {
            const newMessages: ChatMessage[] = evt.newMessages || [];
            const finalText: string = evt.finalText || streamed;
            const committed: ChatMessage[] = newMessages.length
              ? newMessages
              : [{ role: 'assistant', content: finalText || 'No response from model' }];
            const upd: LocalSession = {
              ...base,
              updatedAt: Date.now(),
              messages: [...base.messages, ...committed],
            };
            upsertSession(upd);
            setLiveAssistant(null);
            setToolStatus('');
          } else if (evt.type === 'error') {
            throw new Error(evt.error || 'Unknown error');
          }
        }
      }
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : 'Unknown error';
      const upd: LocalSession = {
        ...base,
        updatedAt: Date.now(),
        messages: [...base.messages, { role: 'assistant', content: `Error: ${errorMsg}` }],
      };
      upsertSession(upd);
      setLiveAssistant(null);
      setToolStatus('');
    } finally {
      setIsLoading(false);
    }
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendMessage(input);
    }
  }

  function autoResizeTextarea() {
    const textarea = textareaRef.current;
    if (textarea) {
      textarea.style.height = 'auto';
      // Respect the explicit minHeight (44px) so single-line text sits
      // vertically centered; cap the upper bound at 200px.
      const next = Math.min(Math.max(textarea.scrollHeight, 44), 200);
      textarea.style.height = `${next}px`;
    }
  }

  const currentProvider = config.providers[config.defaultProvider];

  return (
    <div className="flex h-dvh bg-gray-950 text-gray-100 overflow-hidden">
      {/* 移动端遮罩 */}
      {sidebarOpen && (
        <div
          className="fixed inset-0 bg-black/60 z-30 md:hidden"
          onClick={() => setSidebarOpen(false)}
        />
      )}

      {/* Sidebar：桌面常驻；移动端抽屉（汉堡展开） */}
      <aside
        className={`w-64 md:w-72 bg-gray-900 border-r-[3px] border-black flex flex-col shrink-0
          fixed inset-y-0 left-0 z-40 transform transition-transform duration-200 md:static md:translate-x-0
          ${sidebarOpen ? 'translate-x-0' : '-translate-x-full'}`}
      >
        <div className="p-4 border-b-[3px] border-black flex items-center justify-between">
          <div className="flex items-center gap-3">
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img
              src="/logo-w-64.png"
              alt="Firment"
              className="w-10 h-10 rounded-md object-contain bg-[#2f6bff] p-1 shadow-[3px_3px_0_#000]"
            />
            <div>
              <h1 className="font-extrabold text-white text-lg leading-tight tracking-wide">FIRMENT</h1>
              <p className="text-[10px] text-gray-500 tracking-[1.5px]">FIRMWARE + AGENT</p>
            </div>
          </div>
          {/* 移动端关闭按钮 */}
          <button
            onClick={() => setSidebarOpen(false)}
            className="p-1 text-gray-400 hover:text-white md:hidden"
            aria-label="Close menu"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <div className="p-4">
          <button
            onClick={createNewSession}
            className="w-full flex items-center justify-center gap-2 px-4 py-3 bg-[#2f6bff] hover:bg-[#2456d6] text-white font-bold border-[3px] border-black shadow-[4px_4px_0_#000] transition-all duration-100 active:translate-x-[2px] active:translate-y-[2px] active:shadow-none"
          >
            <Plus className="w-5 h-5" />
            New Session
          </button>
        </div>

        <div className="flex-1 overflow-y-auto px-3 pb-3">
          {sessions.length === 0 ? (
            <div className="text-center py-8 text-gray-500 text-sm">
              <MessageSquare className="w-8 h-8 mx-auto mb-2 opacity-50" />
              No sessions yet
            </div>
          ) : (
            <div className="space-y-1">
              {sessions.map((session) => (
                <div
                  key={session.id}
                  className={`group flex items-center gap-2 px-3 py-2.5 cursor-pointer transition-all duration-100 ${
                    currentId === session.id
                      ? 'bg-[#2f6bff] border-[3px] border-black shadow-[3px_3px_0_#000]'
                      : 'hover:bg-gray-800 border-[3px] border-transparent'
                  }`}
                  onClick={() => selectSession(session.id)}
                >
                  <MessageSquare
                    className={`w-4 h-4 shrink-0 ${
                      currentId === session.id ? 'text-white' : 'text-gray-400'
                    }`}
                  />
                  <div className="flex-1 min-w-0">
                    <p
                      className={`text-sm truncate ${
                        currentId === session.id ? 'text-white font-bold' : 'text-gray-200'
                      }`}
                    >
                      {session.title || 'Empty session'}
                    </p>
                    <p className={`text-xs ${currentId === session.id ? 'text-blue-100' : 'text-gray-500'}`}>
                      {session.messages.length} messages
                    </p>
                  </div>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      deleteSession(session.id);
                    }}
                    className="opacity-0 group-hover:opacity-100 p-1 hover:bg-red-500/30 transition-all"
                  >
                    <Trash2 className="w-3 h-3 text-red-400" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="p-4 border-t-[3px] border-black">
          <button
            onClick={() => setShowSettings(true)}
            className="w-full flex items-center gap-2 px-3 py-2 text-sm text-gray-300 hover:text-white hover:bg-gray-800 border-[2px] border-black transition-colors"
          >
            <Settings className="w-4 h-4" />
            Settings
          </button>
        </div>
      </aside>

      {/* Main Content */}
      <main className="flex-1 flex flex-col min-w-0">
        <header className="flex items-center justify-between gap-2 px-3 md:px-6 py-3 bg-gray-900/60 border-b-[3px] border-black">
          <div className="flex items-center gap-2 md:gap-3 min-w-0">
            {/* 移动端汉堡按钮 */}
            <button
              onClick={() => setSidebarOpen(true)}
              className="p-2 text-gray-300 hover:text-white border-[2px] border-black bg-gray-800 md:hidden"
              aria-label="Open menu"
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
              </svg>
            </button>
            {currentProvider && (
              <div className="flex items-center gap-2 px-3 py-1.5 bg-gray-800 border-[2px] border-black min-w-0">
                <div className="w-2 h-2 bg-green-500 animate-pulse shrink-0" />
                <span className="text-xs text-gray-300 font-mono font-bold truncate">
                  {currentProvider.model || 'no model'}
                </span>
              </div>
            )}
            {current && (
              <span className="text-xs text-gray-500 font-mono hidden sm:inline">
                {current.id.slice(0, 8)}
              </span>
            )}
          </div>
          <div className="flex items-center gap-2 shrink-0">
            <span className="text-xs text-gray-500 font-bold">
              {displayMessages.length} messages
            </span>
          </div>
        </header>

        <div className="flex-1 overflow-y-auto px-3 md:px-6 py-4 md:py-6">
          {renderMessages.length === 0 ? (
            <EmptyState onSend={sendMessage} workspace={config.tools?.workspace || '.'} />
          ) : (
            <div className="max-w-4xl mx-auto space-y-4">
              {renderMessages.map((msg, i) => (
                <MessageBubble key={i} message={msg} streaming={!!(liveAssistant && i === renderMessages.length - 1)} />
              ))}
              {toolStatus && (
                <div className="text-xs text-gray-500 font-mono px-2">{toolStatus}</div>
              )}
              {isLoading && !liveAssistant && <LoadingBubble />}
              <div ref={messagesEndRef} />
            </div>
          )}
        </div>

        <div className="border-t-[3px] border-black bg-gray-900/60 px-2 md:px-6 py-3 md:py-4">
          <div className="max-w-3xl mx-auto">
            <div className="relative flex items-end gap-2 md:gap-3 bg-gray-800 border-[3px] border-black shadow-[3px_3px_0_#000] md:shadow-[5px_5px_0_#000] p-2 md:p-3">
              <textarea
                ref={textareaRef}
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={handleKeyDown}
                onInput={autoResizeTextarea}
                placeholder="Describe your task or question..."
                rows={1}
                disabled={isLoading}
                style={{ minHeight: '44px', lineHeight: '20px', padding: '12px 4px' }}
                className="flex-1 bg-transparent resize-none outline-none text-gray-100 placeholder-gray-500 text-sm font-mono max-h-[200px]"
              />
              <button
                onClick={() => sendMessage(input)}
                disabled={isLoading || !input.trim()}
                className={`p-2.5 border-[3px] border-black ${
                  isLoading || !input.trim()
                    ? 'bg-gray-700 text-gray-500 cursor-not-allowed'
                    : 'bg-[#2f6bff] hover:bg-[#2456d6] text-white shadow-[3px_3px_0_#000] active:translate-x-[2px] active:translate-y-[2px] active:shadow-none transition-all duration-100'
                }`}
              >
                {isLoading ? (
                  <Loader2 className="w-5 h-5 animate-spin" />
                ) : (
                  <Send className="w-5 h-5" />
                )}
              </button>
            </div>
            <p className="text-xs text-gray-500 mt-2 text-center">
              Firment Web · Read-only mode · Cannot write files or execute commands
            </p>
          </div>
        </div>
      </main>

      {/* Settings Modal */}
      {showSettings && (
        <div className="fixed inset-0 bg-black/70 flex items-end md:items-center justify-center z-50 p-0 md:p-4">
          <div
            className="bg-gray-900 border-[3px] border-black shadow-[8px_8px_0_#000] w-full max-w-2xl max-h-[92dvh] md:max-h-[90vh] flex flex-col"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between px-6 py-4 border-b-[3px] border-black">
              <h2 className="font-bold text-white flex items-center gap-2">
                <Settings className="w-5 h-5 text-[#2f6bff]" />
                Settings
              </h2>
              <button
                onClick={() => setShowSettings(false)}
                className="text-gray-400 hover:text-white transition-colors"
              >
                <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>

            <div className="flex-1 overflow-y-auto p-6 space-y-6">
              <div>
                <div className="flex items-center justify-between mb-4">
                  <h3 className="text-sm font-medium text-gray-300 flex items-center gap-2">
                    <Zap className="w-4 h-4 text-yellow-500" />
                    Providers
                  </h3>
                  <button
                    onClick={addProvider}
                    className="flex items-center gap-1 px-3 py-1.5 text-xs text-[#5b8cff] hover:text-[#7aa5ff] hover:bg-blue-500/10 font-bold transition-colors"
                  >
                    <PlusCircle className="w-3.5 h-3.5" />
                    Add Provider
                  </button>
                </div>

                <div className="space-y-4">
                  {Object.entries(config.providers).map(([name, provider]) => (
                    <div key={`provider-${name}`} className="p-4 bg-gray-800/50 border border-gray-700 ">
                      <div className="flex items-center justify-between gap-2 mb-3 flex-wrap">
                        <div className="flex items-center gap-2 min-w-0 flex-1 flex-wrap">
                          <input
                            type="text"
                            value={name}
                            onChange={(e) => {
                              const newName = e.target.value;
                              const newProviders = { ...config.providers };
                              delete newProviders[name];
                              newProviders[newName] = provider;
                              updateConfig({
                                ...config,
                                providers: newProviders,
                                defaultProvider: config.defaultProvider === name ? newName : config.defaultProvider,
                              });
                            }}
                            className="px-2 py-1 bg-gray-700 border-[2px] border-black text-sm text-gray-200 font-mono focus:border-blue-500 outline-none min-w-0 max-w-[140px] flex-1"
                          />
                          <select
                            value={config.defaultProvider === name ? 'selected' : ''}
                            onChange={(e) => {
                              if (e.target.value) {
                                updateConfig({ ...config, defaultProvider: name });
                              }
                            }}
                            className="px-2 py-1 bg-gray-700 border-[2px] border-black text-sm text-gray-200 focus:border-blue-500 outline-none shrink-0"
                          >
                            <option value="selected">Default</option>
                          </select>
                        </div>
                        <button
                          onClick={() => removeProvider(name)}
                          className="p-1.5 text-red-400 hover:text-red-300 hover:bg-red-500/10 transition-colors"
                        >
                          <Trash className="w-4 h-4" />
                        </button>
                      </div>

                      <div className="flex flex-col gap-3">
                        <div>
                          <label className="block text-xs text-gray-400 mb-1">Type</label>
                          <select
                            value={provider.type}
                            onChange={(e) => updateProvider(name, { type: e.target.value as 'openai' | 'anthropic' })}
                            className="w-full px-3 py-2 bg-gray-700 border-[2px] border-black-lg text-sm text-gray-200 focus:border-blue-500 outline-none"
                          >
                            <option value="openai">OpenAI Compatible</option>
                            <option value="anthropic">Anthropic</option>
                          </select>
                        </div>
                        <div>
                          <label className="block text-xs text-gray-400 mb-1">Base URL</label>
                          <div className="relative">
                            <LinkIcon className="absolute left-3 top-2.5 w-4 h-4 text-gray-500" />
                            <input
                              type="text"
                              value={provider.baseUrl}
                              onChange={(e) => updateProvider(name, { baseUrl: e.target.value })}
                              placeholder="https://api.deepseek.com/v1"
                              className="w-full pl-9 pr-3 py-2 bg-gray-700 border-[2px] border-black-lg text-sm text-gray-200 font-mono focus:border-blue-500 outline-none"
                            />
                          </div>
                        </div>
                        <div className="col-span-2">
                          <label className="block text-xs text-gray-400 mb-1">API Key</label>
                          <div className="relative">
                            <Key className="absolute left-3 top-2.5 w-4 h-4 text-gray-500" />
                            <input
                              type="password"
                              value={provider.apiKey}
                              onChange={(e) => updateProvider(name, { apiKey: e.target.value })}
                              placeholder="sk-..."
                              className="w-full pl-9 pr-3 py-2 bg-gray-700 border-[2px] border-black-lg text-sm text-gray-200 font-mono focus:border-blue-500 outline-none"
                            />
                          </div>
                        </div>
                        <div className="col-span-2">
                          <label className="block text-xs text-gray-400 mb-1">Model</label>
                          <input
                            type="text"
                            value={provider.model}
                            onChange={(e) => updateProvider(name, { model: e.target.value })}
                            placeholder="deepseek-v4-flash"
                            className="w-full px-3 py-2 bg-gray-700 border-[2px] border-black-lg text-sm text-gray-200 font-mono focus:border-blue-500 outline-none"
                          />
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              </div>

              <div className="pt-4 border-t border-gray-800">
                <h3 className="text-sm font-medium text-gray-300 mb-3 flex items-center gap-2">
                  <Code className="w-4 h-4 text-green-500" />
                  Tools
                </h3>
                <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                  <div>
                    <label className="block text-xs text-gray-400 mb-1">Workspace</label>
                    <input
                      type="text"
                      value={config.tools?.workspace || '.'}
                      onChange={(e) =>
                        updateConfig({
                          ...config,
                          tools: { ...config.tools, workspace: e.target.value },
                        })
                      }
                      className="w-full px-3 py-2 bg-gray-800 border-[2px] border-black text-sm text-gray-200 font-mono focus:border-blue-500 outline-none"
                    />
                  </div>
                  <div>
                    <label className="block text-xs text-gray-400 mb-1">Web Search</label>
                    <select
                      value={config.tools?.webSearch || 'duckduckgo'}
                      onChange={(e) =>
                        updateConfig({
                          ...config,
                          tools: { ...config.tools, webSearch: e.target.value },
                        })
                      }
                      className="w-full px-3 py-2 bg-gray-800 border-[2px] border-black text-sm text-gray-200 focus:border-blue-500 outline-none"
                    >
                      <option value="duckduckgo">DuckDuckGo (no key)</option>
                    </select>
                    <p className="text-xs text-gray-500 mt-1">Tavily/Brave coming soon</p>
                  </div>
                </div>
              </div>

              <div className="p-4 bg-gray-800/50 border-[2px] border-black">
                <p className="text-xs text-gray-400 flex items-start gap-2">
                  <Shield className="w-4 h-4 text-blue-500 shrink-0 mt-0.5" />
                  API keys are stored locally in your browser. They are only sent to the configured model provider when making requests.
                </p>
              </div>
            </div>

            <div className="flex items-center justify-end gap-3 px-6 py-4 border-t border-gray-800">
              <button
                onClick={() => setShowSettings(false)}
                className="px-4 py-2 text-sm text-gray-400 hover:text-white transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={() => setShowSettings(false)}
                className="px-4 py-2 bg-[#2f6bff] hover:bg-[#2456d6] text-white text-sm font-bold border-[2px] border-black shadow-[3px_3px_0_#000] transition-colors"
              >
                Save Settings
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function EmptyState({ onSend, workspace }: { onSend: (text: string) => void; workspace: string }) {
  const suggestions = [
    { icon: FileText, text: `Read the main file in ${workspace}`, color: 'blue' },
    { icon: Search, text: 'Search for GPIO configuration patterns', color: 'purple' },
    { icon: Globe, text: 'Look up STM32F407 EXTI documentation', color: 'green' },
    { icon: Code, text: 'List all source files in the project', color: 'yellow' },
  ];

  const colorMap: Record<string, string> = {
    blue: 'bg-[#2f6bff] border-[3px] border-black text-white hover:bg-[#2456d6] shadow-[4px_4px_0_#000] hover:shadow-[6px_6px_0_#000]',
    purple: 'bg-[#a855f7] border-[3px] border-black text-white hover:bg-[#9333ea] shadow-[4px_4px_0_#000] hover:shadow-[6px_6px_0_#000]',
    green: 'bg-[#22c55e] border-[3px] border-black text-black hover:bg-[#16a34a] shadow-[4px_4px_0_#000] hover:shadow-[6px_6px_0_#000]',
    yellow: 'bg-[#facc15] border-[3px] border-black text-black hover:bg-[#eab308] shadow-[4px_4px_0_#000] hover:shadow-[6px_6px_0_#000]',
  };

  return (
    <div className="flex flex-col items-center justify-center h-full min-h-[400px] md:min-h-[500px] text-center px-4">
      {/* eslint-disable-next-line @next/next/no-img-element */}
      <img
        src="/logo-w-128.png"
        alt="Firment"
        className="w-20 h-20 rounded-md object-contain bg-[#2f6bff] p-2 mb-6 border-[3px] border-black shadow-[6px_6px_0_#000]"
      />
      <h2 className="text-3xl font-extrabold text-white mb-3 tracking-tight">WELCOME TO FIRMENT</h2>
      <p className="text-gray-400 max-w-md mb-8 text-base leading-relaxed">
        A web-based coding agent for firmware and embedded development.
        Read files, search code, and research documentation.
      </p>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 w-full max-w-2xl">
        {suggestions.map(({ icon: Icon, text, color }, i) => (
          <button
            key={i}
            onClick={() => onSend(text)}
            className={`flex items-center gap-3 px-4 py-3.5 border-[3px] border-black text-left font-bold transition-all duration-100 active:translate-x-[2px] active:translate-y-[2px] active:shadow-none ${colorMap[color]}`}
          >
            <Icon className="w-5 h-5 shrink-0" />
            <span className="text-sm">{text}</span>
          </button>
        ))}
      </div>
      <div className="flex items-center gap-6 mt-10 text-xs text-gray-500">
        <span className="flex items-center gap-1.5">
          <Zap className="w-3.5 h-3.5 text-yellow-500" />
          Streaming responses
        </span>
        <span className="flex items-center gap-1.5">
          <Search className="w-3.5 h-3.5 text-blue-500" />
          File search
        </span>
        <span className="flex items-center gap-1.5">
          <Globe className="w-3.5 h-3.5 text-green-500" />
          Web research
        </span>
      </div>
    </div>
  );
}

function MessageBubble({ message, streaming }: { message: ChatMessage; streaming?: boolean }) {
  const isUser = message.role === 'user';
  const toolNames = message.tool_calls?.map((tc) => tc.name).filter(Boolean) || [];

  return (
    <div className={`flex items-start gap-3 ${isUser ? 'flex-row-reverse' : ''}`}>
      {isUser ? (
        <div className="w-8 h-8 flex items-center justify-center shrink-0 bg-gray-700 border-[2px] border-black">
          <User className="w-4 h-4 text-gray-300" />
        </div>
      ) : (
        // eslint-disable-next-line @next/next/no-img-element
        <img
          src="/logo-w-32.png"
          alt="Firment"
          className="w-8 h-8 shrink-0 object-contain bg-[#2f6bff] p-0.5 border-[2px] border-black"
        />
      )}
      <div
        style={{ width: 'fit-content', maxWidth: '85%' }}
        className={`px-3 py-2 border-[3px] border-black ${
          isUser
            ? 'bg-[#2f6bff] text-white shadow-[4px_4px_0_#000]'
            : 'bg-gray-800 text-gray-100 border-[2px]'
        }`}
      >
        <div className="text-sm leading-relaxed whitespace-pre-wrap">{(message.content ?? '').replace(/^[\s\n]+|[\s\n]+$/g, '')}</div>
        {toolNames.length > 0 && (
          <div className="flex flex-wrap gap-1 mt-2">
            {toolNames.map((name, idx) => (
              <span
                key={idx}
                className="inline-flex items-center gap-1 px-2 py-0.5 bg-gray-700/70 text-gray-300 text-xs font-mono border-[1px] border-black"
              >
                <Wrench className="w-3 h-3" />
                {name}
              </span>
            ))}
          </div>
        )}
        {!isUser && streaming && (
          <p className="text-xs text-gray-500 mt-2">generating…</p>
        )}
      </div>
    </div>
  );
}

function LoadingBubble() {
  return (
    <div className="flex items-start gap-3">
      {/* eslint-disable-next-line @next/next/no-img-element */}
      <img
        src="/logo-w-32.png"
        alt="Firment"
        className="w-8 h-8 shrink-0 object-contain bg-[#2f6bff] p-0.5 border-[2px] border-black"
      />
      <div className="bg-gray-800 border-[2px] border-black px-4 py-3 max-w-[80%]">
        <div className="flex items-center gap-2">
          <div className="flex gap-1">
            <span className="w-2 h-2 bg-[#2f6bff] animate-bounce" style={{ animationDelay: '0ms' }} />
            <span className="w-2 h-2 bg-[#a855f7] animate-bounce" style={{ animationDelay: '150ms' }} />
            <span className="w-2 h-2 bg-[#facc15] animate-bounce" style={{ animationDelay: '300ms' }} />
          </div>
          <span className="text-xs text-gray-400 font-bold">THINKING...</span>
        </div>
      </div>
    </div>
  );
}
