export interface Track {
  id: string;
  title: string;
  artist: string;
  album: string;
  year: string;
  trackNumber?: number;
  totalTracks?: number;
  duration: number;
  coverUrl: string;
  previewUrl: string | null;
  status:
    | 'idle'
    | 'scraping'
    | 'downloading'
    | 'transcoding'
    | 'tagging'
    | 'completed'
    | 'failed'
    | 'cancelled';
  progress: number;
  error?: string;
}

export interface Playlist {
  playlistName: string;
  tracks: Track[];
}

export type AudioFormat = 'mp3' | 'flac' | 'm4a' | 'wav' | 'mp4' | 'opus';

export type Bitrate = '128k' | '192k' | '256k' | '320k' | 'lossless';

export interface DownloadSettings {
  format: AudioFormat;
  bitrate: Bitrate;
  youtubeCookies?: string;
  cookiesFromBrowser?: string;
  sampleRate?: '44100' | '48000';
  videoQuality?: '2160p' | '1440p' | '1080p' | '720p' | '480p' | '360p' | 'best';
  saveInFolder?: boolean;
  skipMissingTracks?: boolean;
  namingPattern?: 'number_artist_title' | 'number_title' | 'artist_title' | 'title';
  embedId3Tags?: boolean;
  folderNamingPattern?: 'album_artist' | 'year_album' | 'album';
}

export interface AppErrorPayload {
  error: string;
  code?: string;
}

export function isAppErrorPayload(value: unknown): value is AppErrorPayload {
  return (
    typeof value === 'object' &&
    value !== null &&
    'error' in value &&
    typeof (value as Record<string, unknown>).error === 'string'
  );
}

export function getErrorCode(err: unknown): string | undefined {
  if (isAppErrorPayload(err)) return err.code;
  if (err instanceof Error && 'code' in err) {
    const code = (err as Error & { code?: unknown }).code;
    return typeof code === 'string' ? code : undefined;
  }
  return undefined;
}
