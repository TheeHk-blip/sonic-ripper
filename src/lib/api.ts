import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import { Track } from '../types';

export async function saveCoverImage(
  coverUrl: string,
  defaultName = 'cover.jpg'
): Promise<boolean> {
  // If running inside Tauri desktop app
  if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
    try {
      const selected = await save({
        defaultPath: defaultName,
        filters: [
          { name: 'JPEG Image', extensions: ['jpg', 'jpeg'] },
          { name: 'All Images', extensions: ['jpg', 'jpeg', 'png', 'webp', 'gif'] },
        ],
      });

      if (!selected) return false;

      await invoke('save_cover_file', {
        coverUrl,
        targetPath: selected,
      });
      return true;
    } catch (err) {
      console.warn('Tauri save dialog failed, falling back to browser download:', err);
    }
  }

  // Fallback for web browser download
  try {
    const res = await fetch(coverUrl);
    const blob = await res.blob();
    const blobUrl = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = blobUrl;
    a.download = defaultName;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    setTimeout(() => URL.revokeObjectURL(blobUrl), 1000);
    return true;
  } catch {
    const a = document.createElement('a');
    a.href = coverUrl;
    a.download = defaultName;
    a.target = '_blank';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    return true;
  }
}

export interface ApiError {
  error: string;
  code?: string;
}

function toError(err: unknown): Error & { code?: string } {
  if (err && typeof err === 'object' && 'error' in err) {
    const apiErr = err as ApiError;
    const e = new Error(apiErr.error) as Error & { code?: string };
    Object.defineProperty(e, 'code', {
      value: apiErr.code,
      enumerable: true,
      writable: true,
      configurable: true,
    });
    return e;
  }
  return new Error(typeof err === 'string' ? err : 'Unknown error.');
}

export type AnalyzeResponse =
  | { type: 'track'; track: Track }
  | { type: 'playlist'; playlistName: string; isAlbum: boolean; tracks: Track[] };

export async function analyzeSpotify(url: string): Promise<AnalyzeResponse> {
  try {
    return await invoke<AnalyzeResponse>('analyze', { url });
  } catch (err) {
    throw toError(err);
  }
}

export interface AppSettings {
  downloadFolder: string | null;
  namingPattern?: string | null;
}

export async function getSettings(): Promise<AppSettings> {
  try {
    return await invoke<AppSettings>('get_settings');
  } catch (err) {
    throw toError(err);
  }
}

export async function setDownloadFolder(folder: string): Promise<AppSettings> {
  try {
    return await invoke<AppSettings>('set_download_folder', { folder });
  } catch (err) {
    throw toError(err);
  }
}

export async function setNamingPattern(pattern: string): Promise<AppSettings> {
  try {
    return await invoke<AppSettings>('set_naming_pattern', { pattern });
  } catch (err) {
    throw toError(err);
  }
}

export async function pickDownloadFolder(): Promise<AppSettings | null> {
  const selected = await open({ directory: true, multiple: false });
  if (!selected || Array.isArray(selected)) return null;
  return setDownloadFolder(selected);
}

export interface DownloadOptions {
  format: string;
  bitrate: string;
  youtubeCookies?: string;
  cookiesFromBrowser?: string;
  sampleRate?: string;
  videoQuality?: string;
  namingPattern: string;
  embedId3Tags: boolean;
  albumFolder?: string;
  playlistName?: string;
}

export async function downloadTrack(track: Track, opts: DownloadOptions): Promise<string> {
  try {
    return await invoke<string>('download_track', { track, opts });
  } catch (err) {
    throw toError(err);
  }
}

export interface DownloadBatchOptions extends DownloadOptions {
  playlistName: string;
  saveInFolder?: boolean;
  skipMissingTracks?: boolean;
}

export async function downloadBatch(tracks: Track[], opts: DownloadBatchOptions): Promise<string> {
  try {
    return await invoke<string>('download_batch', { tracks, args: opts });
  } catch (err) {
    throw toError(err);
  }
}

export async function startPreview(
  url: string,
  kind: 'video',
  opts?: {
    youtubeCookies?: string;
    cookiesFromBrowser?: string;
  }
): Promise<string> {
  try {
    return await invoke<string>('start_preview', {
      url,
      kind,
      youtubeCookies: opts?.youtubeCookies ?? null,
      cookiesFromBrowser: opts?.cookiesFromBrowser ?? null,
    });
  } catch (err) {
    throw toError(err);
  }
}

export async function stopPreview(): Promise<void> {
  try {
    await invoke('stop_preview');
  } catch (err) {
    throw toError(err);
  }
}

export async function generateTrackSpectrogram(
  track: Track,
  youtubeCookies?: string,
  cookiesFromBrowser?: string
): Promise<string> {
  try {
    return await invoke<string>('generate_track_spectrogram', {
      track,
      youtubeCookies: youtubeCookies || null,
      cookiesFromBrowser: cookiesFromBrowser || null,
    });
  } catch (err) {
    throw toError(err);
  }
}
