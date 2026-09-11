'use client';

import { Code, FileText, Globe, Search, Zap } from 'lucide-react';

/**
 * The empty chat's welcome screen.
 *
 * Extracted from `ChatPage.tsx` verbatim, so the split is verifiable: the only
 * change is the file it lives in. It is presentational -- `onSend` is the single
 * way out -- which is what makes it worth its own test: the suggestions are
 * what a first-time visitor clicks, and each one sends the text it shows.
 */
export function EmptyState({
  onSend,
  workspace,
}: {
  onSend: (text: string) => void;
  workspace: string;
}) {
  const suggestions = [
    { icon: FileText, text: `Read the main file in ${workspace}`, color: 'blue' },
    { icon: Search, text: 'Search for GPIO configuration patterns', color: 'purple' },
    { icon: Globe, text: 'Look up STM32F407 EXTI documentation', color: 'green' },
    { icon: Code, text: 'List all source files in the project', color: 'yellow' },
  ];

  const colorMap: Record<string, string> = {
    blue: 'btn-brand border-[3px] border-black shadow-[4px_4px_0_#000] hover:shadow-[6px_6px_0_#000]',
    purple:
      'bg-info-ink border-[3px] border-black text-black hover:brightness-90 shadow-[4px_4px_0_#000] hover:shadow-[6px_6px_0_#000]',
    green:
      'bg-success-ink border-[3px] border-black text-black hover:brightness-90 shadow-[4px_4px_0_#000] hover:shadow-[6px_6px_0_#000]',
    yellow:
      'bg-warn-ink border-[3px] border-black text-black hover:brightness-90 shadow-[4px_4px_0_#000] hover:shadow-[6px_6px_0_#000]',
  };

  return (
    <div className="flex flex-col items-center justify-center h-full min-h-[400px] md:min-h-[500px] text-center px-4">
      {/* eslint-disable-next-line @next/next/no-img-element */}
      <img
        src="/logo-w-128.png"
        alt="Firment"
        className="w-20 h-20 rounded-md object-contain surface-brand p-2 mb-6 border-[3px] border-black shadow-[6px_6px_0_#000]"
      />
      <h2 className="text-3xl font-extrabold text-white mb-3 tracking-tight">WELCOME TO FIRMENT</h2>
      <p className="text-gray-400 max-w-md mb-8 text-base leading-relaxed">
        A web-based coding agent for firmware and embedded development. Read files, search code,
        and research documentation.
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
