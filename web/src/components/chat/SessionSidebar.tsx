'use client';

import { MessageSquare, Plus, Settings, Trash2 } from 'lucide-react';
import { LocalSession } from '@/lib/localSessions';

/**
 * The session list, and the drawer it becomes on a narrow screen.
 *
 * Extracted from `ChatPage.tsx`. The mobile backdrop belongs to this component
 * rather than to the page: it exists only to dismiss the drawer, so it has no
 * meaning without it.
 *
 * Two details that look like styling and are not:
 *
 *   * the selected row is brand-filled and the others are transparent, which is
 *     the only thing that says which session you are typing into.
 *   * the delete control is `opacity-0 group-hover:opacity-100` -- present but
 *     invisible until the row is hovered, so a mis-click does not delete a
 *     session. Its click also stops propagation, because otherwise the same
 *     click would select the row it just deleted.
 */
export function SessionSidebar({
  open,
  onClose,
  sessions,
  currentId,
  onNew,
  onSelect,
  onDelete,
  onOpenSettings,
}: {
  open: boolean;
  onClose: () => void;
  sessions: LocalSession[];
  currentId: string | null;
  onNew: () => void;
  onSelect: (id: string) => void;
  onDelete: (id: string) => void;
  onOpenSettings: () => void;
}) {
  return (
    <>
      {/* 移动端遮罩 */}
      {open && (
        <div className="fixed inset-0 bg-black/60 z-30 md:hidden" onClick={onClose} />
      )}

      {/* Sidebar：桌面常驻；移动端抽屉（汉堡展开） */}
      <aside
        className={`w-64 md:w-72 bg-gray-900 border-r-[3px] border-black flex flex-col shrink-0
          fixed inset-y-0 left-0 z-40 transform transition-transform duration-200 md:static md:translate-x-0
          ${open ? 'translate-x-0' : '-translate-x-full'}`}
      >
        <div className="p-4 border-b-[3px] border-black flex items-center justify-between">
          <div className="flex items-center gap-3">
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img
              src="/logo-w-64.png"
              alt="Firment"
              className="w-10 h-10 rounded-md object-contain surface-brand p-1 shadow-[3px_3px_0_#000]"
            />
            <div>
              <h1 className="font-extrabold text-white text-lg leading-tight tracking-wide">
                FIRMENT
              </h1>
              <p className="text-[10px] text-gray-500 tracking-[1.5px]">FIRMWARE + AGENT</p>
            </div>
          </div>
          {/* 移动端关闭按钮 */}
          <button
            onClick={onClose}
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
            onClick={onNew}
            className="w-full flex items-center justify-center gap-2 px-4 py-3 btn-brand font-bold border-[3px] border-black shadow-[4px_4px_0_#000] transition-all duration-100 active:shadow-none"
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
                      ? 'surface-brand border-[3px] border-black shadow-[3px_3px_0_#000]'
                      : 'hover:bg-gray-800 border-[3px] border-transparent'
                  }`}
                  onClick={() => onSelect(session.id)}
                >
                  {/* The selected row is `.surface-brand`, which already sets
                      `color: var(--on-acid)` (13.28:1 on the acid). Overriding
                      it with `text-white` here was 1.27:1 -- the exact ratio
                      docs/design/tokens.md names when it says the acid is a
                      fill, never a text colour. Inherit instead. */}
                  <MessageSquare
                    className={`w-4 h-4 shrink-0 ${
                      currentId === session.id ? '' : 'text-gray-400'
                    }`}
                  />
                  <div className="flex-1 min-w-0">
                    <p
                      className={`text-sm truncate ${
                        currentId === session.id ? 'font-bold' : 'text-gray-200'
                      }`}
                    >
                      {session.title || 'Empty session'}
                    </p>
                    <p
                      // Dimmed by opacity rather than a second colour: the
                      // on-acid ink at 70% is 5.50:1, and a named grey would
                      // have to be re-measured against the acid, not the page.
                      className={`text-xs ${
                        currentId === session.id ? 'opacity-70' : 'text-gray-500'
                      }`}
                    >
                      {session.messages.length} messages
                    </p>
                  </div>
                  <button
                    aria-label={`Delete session: ${session.title || 'Untitled'}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      onDelete(session.id);
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
            onClick={onOpenSettings}
            className="w-full flex items-center gap-2 px-3 py-2 text-sm text-gray-300 hover:text-white hover:bg-gray-800 border-[2px] border-black transition-colors"
          >
            <Settings className="w-4 h-4" />
            Settings
          </button>
        </div>
      </aside>
    </>
  );
}
