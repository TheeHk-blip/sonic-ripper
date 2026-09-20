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
  originalCoverUrl?: string;
  previewUrl: string | null;
  status: 'idle' | 'scraping' | 'downloading' | 'transcoding' | 'tagging' | 'completed' | 'failed';
  progress: number;
  error?: string;
}

export interface Playlist {
  playlistName: string;
  tracks: Track[];
}

export type AudioFormat = 'mp3' | 'flac' | 'm4a' | 'wav' | 'mp4' | 'opus';

export type Bitrate = '128k' | '256k' | '320k' | 'lossless';

export interface DownloadSettings {
  format: AudioFormat;
  bitrate: Bitrate;
  youtubeCookies?: string;
  cookiesFromBrowser?: string;
  sampleRate?: '44100' | '48000';
  videoQuality?: '1080p' | '720p' | '480p' | '360p' | 'best';
  saveInFolder?: boolean;
  skipMissingTracks?: boolean;
  namingPattern?: string;
  embedId3Tags?: boolean;
}
