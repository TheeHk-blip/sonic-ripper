import { useEffect, useState } from 'react';
import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

export function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [status, setStatus] = useState<'idle' | 'downloading' | 'installing' | 'error'>('idle');
  const [progress, setProgress] = useState(0);

  useEffect(() => {
    check()
      .then(result => {
        if (result !== null) setUpdate(result);
      })
      .catch(err => console.error('Update check failed:', err));
  }, []);

  if (!update) return null;

  const installUpdate = async () => {
    try {
      let downloaded = 0;
      let total = 0;
      await update.downloadAndInstall(event => {
        switch (event.event) {
          case 'Started':
            total = event.data.contentLength ?? 0;
            setStatus('downloading');
            break;
          case 'Progress':
            downloaded += event.data.chunkLength;
            if (total > 0) setProgress(Math.round((downloaded / total) * 100));
            break;
          case 'Finished':
            setStatus('installing');
            break;
        }
      });
      await relaunch();
    } catch (err) {
      console.error('Update failed:', err);
      setStatus('error');
    }
  };

  return (
    <div className="flex flex-row bg-charcoal px-2.5 py-1.5 gap-4 rounded-sm">
      <span className="">
        {status === 'idle' && (
          <span className="text-gold">Version {update.version} available.</span>
        )}
        {status === 'downloading' && `Downloading update… ${progress}%`}
        {status === 'installing' && 'Installing — restarting shortly…'}
        {status === 'error' && 'Update failed. Try again later.'}
      </span>
      {status === 'idle' && (
        <button
          onClick={installUpdate}
          className="bg-gold hover:bg-transparent hover:text-gold hover:ring-2 transition-all duration-300 rounded-sm px-1 py-0.5 cursor-pointer active:scale-98"
        >
          Update Now
        </button>
      )}
    </div>
  );
}
