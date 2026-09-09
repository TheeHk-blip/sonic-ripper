import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { Track } from '../types';

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
