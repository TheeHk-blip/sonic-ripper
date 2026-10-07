import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';

type LogLevel = 'info' | 'warn' | 'error' | 'proc';

interface LogLine {
  level: LogLevel;
  msg: string;
  ts: string;
}

const MAX_LINES = 500;

const LEVEL_STYLES: Record<LogLevel, string> = {
  info: 'text-gold',
  warn: 'text-rust',
  error: 'text-red-400',
  proc: 'text-cream/50',
};

function LogPanel() {
  const [lines, setLines] = useState<LogLine[]>([]);
  const [open, setOpen] = useState(true);
  const [autoScroll, setAutoScroll] = useState(true);
  const bottomRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const unlisten = listen<LogLine>('app-log', event => {
      setLines(prev => {
        const next = [...prev, event.payload];
        return next.length > MAX_LINES ? next.slice(next.length - MAX_LINES) : next;
      });
    });
    return () => {
      unlisten.then(f => f());
    };
  }, []);

  useEffect(() => {
    if (autoScroll) {
      bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [lines, autoScroll]);

  const handleScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
    setAutoScroll(atBottom);
  };

  return (
    <div className="flex flex-col rounded-md border border-cream/10 bg-charcoal mx-2 mt-2 md:h-[90vh] my-auto">
      <div className="flex items-center justify-between border-b border-cream/10 px-3 py-2">
        <button
          type="button"
          onClick={() => setOpen(o => !o)}
          className="label text-cream/70 hover:text-cream transition-colors cursor-pointer"
        >
          {open ? 'Collapse' : 'Open'} Logs
        </button>
        {open && (
          <button
            type="button"
            onClick={() => setLines([])}
            className="label text-cream/40 hover:text-rust transition-colors cursor-pointer"
          >
            Clear
          </button>
        )}
      </div>

      {open && (
        <div
          onScroll={handleScroll}
          className="h-56 md:h-full w-full md:w-100 overflow-y-auto px-3 py-2 font-mono text-xs leading-relaxed"
        >
          {lines.length === 0 ? (
            <p className="text-cream/30 italic">Nothing logged yet.</p>
          ) : (
            lines.map((line, i) => (
              <div key={i} className="whitespace-pre-wrap break-all">
                <span className="text-cream/30">{line.ts} </span>
                <span className={`${LEVEL_STYLES[line.level] ?? 'text-cream'} font-medium`}>
                  [{line.level}]
                </span>{' '}
                <span className="text-cream/90">{line.msg}</span>
              </div>
            ))
          )}
          <div ref={bottomRef} />
        </div>
      )}
    </div>
  );
}

export default LogPanel;
