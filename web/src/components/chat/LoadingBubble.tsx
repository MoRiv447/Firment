'use client';

/**
 * The "waiting for the first token" bubble.
 *
 * Extracted from `ChatPage.tsx` verbatim. It exists so a slow provider looks
 * like work in progress rather than a dead tab: the three dots are staggered by
 * `animationDelay`, and the three colours are brand acid, info and warn, so the
 * animation reads as the product rather than as a generic spinner.
 */
export function LoadingBubble() {
  return (
    <div className="flex items-start gap-3">
      {/* eslint-disable-next-line @next/next/no-img-element */}
      <img
        src="/logo-w-32.png"
        alt="Firment"
        className="w-8 h-8 shrink-0 object-contain surface-brand p-0.5 border-[2px] border-black"
      />
      <div className="bg-gray-800 border-[2px] border-black px-4 py-3 max-w-[80%]">
        <div className="flex items-center gap-2">
          <div className="flex gap-1">
            <span
              className="w-2 h-2 bg-brand-acid animate-bounce"
              style={{ animationDelay: '0ms' }}
            />
            <span className="w-2 h-2 bg-info-ink animate-bounce" style={{ animationDelay: '150ms' }} />
            <span className="w-2 h-2 bg-warn-ink animate-bounce" style={{ animationDelay: '300ms' }} />
          </div>
          <span className="text-xs text-gray-400 font-bold">THINKING...</span>
        </div>
      </div>
    </div>
  );
}
