import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { Minus, Square, Copy, X, AudioLines } from 'lucide-react';
import { UpdateBanner } from './UpdateBanner';

const appWindow = getCurrentWindow();

export default function TitleBar() {
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    (async () => {
      unlisten = await appWindow.onResized(async () => {
        setIsMaximized(await appWindow.isMaximized());
      });
    })();
    return () => {
      unlisten?.();
    };
  }, []);

  const handleMinimize = () => appWindow.minimize();

  const handleMaximizeToggle = async () => {
    await appWindow.toggleMaximize();
    setIsMaximized(await appWindow.isMaximized());
  };

  const handleClose = () => appWindow.close();

  return (
    <div
      data-tauri-drag-region
      onDoubleClick={handleMaximizeToggle}
      className="flex items-center justify-between h-15 px-2.5 py-2.5 bg-brown fixed w-full z-30"
    >
      <div className="flex flex-row items-center gap-2">
        <AudioLines className="size-7 animate-pulse text-rust" />
        <span className="pointer-events-none font-bold font-display text-lg tracking-wider">
          Sonic Ripper
        </span>
      </div>

      <UpdateBanner />

      <div className="flex gap-3.5">
        <button
          type="button"
          aria-label="Minimize"
          onClick={handleMinimize}
          onMouseDown={e => e.stopPropagation()}
          className="flex h-6 w-8 items-center justify-center rounded hover:bg-white/10"
        >
          <Minus size={16} />
        </button>

        <button
          type="button"
          aria-label={isMaximized ? 'Restore' : 'Maximize'}
          onClick={handleMaximizeToggle}
          onMouseDown={e => e.stopPropagation()}
          className="flex h-6 w-8 items-center justify-center rounded hover:bg-gold"
        >
          {isMaximized ? <Copy size={14} /> : <Square size={14} />}
        </button>

        <button
          type="button"
          aria-label="Close"
          onClick={handleClose}
          onMouseDown={e => e.stopPropagation()}
          className="flex h-6 w-8 items-center justify-center rounded hover:bg-red-600"
        >
          <X size={16} />
        </button>
      </div>
    </div>
  );
}
