import React, { useState, useEffect, useMemo } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import {
  AlertCircle,
  Sparkles,
  RefreshCw,
  FolderPlus,
  FolderCheck,
  Archive,
  AudioLines,
  Search,
  ArrowRight,
  X,
} from 'lucide-react';
import SettingsPanel from './components/SettingsPanel';
import MediaPlayer from './components/MediaPlayer';
import StepProgress, { FlowStep } from './components/StepProgress';
import { Track, DownloadSettings, getErrorCode, isAppErrorPayload } from './types';
import { listen } from '@tauri-apps/api/event';
import {
  analyzeSpotify as analyzeLink,
  downloadTrack,
  downloadBatch,
  cancelDownload,
  cancelBatch,
  getSettings,
  pickDownloadFolder,
  setYoutubeCookies,
  setYoutubeCookiesFromBrowser,
} from './lib/api';
import { useVirtualizer } from './lib/useVirtualizer';
import TrackRow from './components/TrackRow';
import TrackListHeader from './components/TrackListHeader';
import { SpotifyPathfinderSettings } from './components/PathFinderSettings';

function friendlyError(err: unknown): string {
  const code = getErrorCode(err);
  const raw = isAppErrorPayload(err)
    ? err.error
    : err instanceof Error
      ? err.message
      : String(err ?? '');

  if (code === 'YOUTUBE_BOT_DETECTED') {
    return 'YouTube blocked this as a bot check. Try turning on cookies (browser profile or pasted) in Settings, then retry.';
  }
  if (code === 'TRACK_NOT_FOUND') {
    return "Couldn't find a YouTube match for this track.";
  }
  if (code === 'FORBIDDEN') {
    return "You're not authenticated. Extract cookies or paste in settings";
  }
  if (code === 'UNSUPPORTED_LINK') {
    return raw;
  }
  if (raw.includes('No download folder is set')) {
    return 'Choose a download folder in Settings before downloading.';
  }
  if (raw.includes('Failed to parse Spotify URL') || raw.includes('SpotifyParseFailed')) {
    return "That doesn't look like a valid link — double-check the URL.";
  }
  if (raw.includes('Network error')) {
    return 'Network error — check your connection and try again.';
  }
  if (raw.toLowerCase().includes('ffmpeg')) {
    return 'Something went wrong converting or tagging this file.';
  }
  if (raw.toLowerCase().includes('yt-dlp')) {
    return 'The downloader ran into an unexpected error fetching this track. Try again later';
  }
  return 'Something wrong happened. Try again';
}

const PROGRESS_PHASE_ORDER: Record<string, number> = {
  downloading: 0,
  transcoding: 1,
  tagging: 2,
};

function overallTrackProgress(status: string, percent: number): number {
  if (status === 'completed') return 100;
  const phaseIndex = PROGRESS_PHASE_ORDER[status];
  if (phaseIndex === undefined) return percent;
  const phaseSpan = 100 / 3;
  return Math.min(100, phaseIndex * phaseSpan + (percent / 100) * phaseSpan);
}

export default function App() {
  const [step, setStep] = useState<FlowStep>('source');
  const [sourceInput, setSourceInput] = useState('');
  const [isAnalyzing, setIsAnalyzing] = useState(false);
  const [analyzeProgress, setAnalyzeProgress] = useState<{
    completed: number;
    total: number;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tracks, setTracks] = useState<Track[]>([]);
  const [playlistName, setPlaylistName] = useState<string>('');
  const [isAlbum, setIsAlbum] = useState<boolean>(false);
  const [isPlaylist, setIsPlaylist] = useState(false);
  const [playingTrack, setPlayingTrack] = useState<Track | null>(null);
  const [settings, setSettings] = useState<DownloadSettings>({
    format: 'flac',
    bitrate: 'lossless',
    saveInFolder: true,
    skipMissingTracks: true,
    namingPattern: 'number_artist_title',
    embedId3Tags: true,
    folderNamingPattern: 'album_artist',
  });
  const [isBatchDownloading, setIsBatchDownloading] = useState(false);
  const analyzePercent = useMemo(() => {
    if (!analyzeProgress || analyzeProgress.total <= 1) return null;
    return Math.min(100, Math.round((analyzeProgress.completed / analyzeProgress.total) * 100));
  }, [analyzeProgress]);

  const batchProgress = useMemo(() => {
    if (!isBatchDownloading || tracks.length === 0) return 0;
    const avg =
      tracks.reduce((sum, t) => sum + overallTrackProgress(t.status, t.progress), 0) /
      tracks.length;
    return Math.round(avg);
  }, [tracks, isBatchDownloading]);

  const [isFolderDownloading, setIsFolderDownloading] = useState(false);
  const [folderProgressText, setFolderProgressText] = useState('');
  const [isCancellingBatch, setIsCancellingBatch] = useState(false);

  const estimatedItemHeight = 110;
  const { containerRef, virtualItems, paddingTop, paddingBottom, measureElement } = useVirtualizer({
    count: tracks.length,
    itemHeight: estimatedItemHeight,
  });

  useEffect(() => {
    const unlistenPromise = listen<{
      trackId: string;
      phase: 'downloading' | 'transcoding' | 'tagging';
      percent: number;
      stream: 'video' | 'audio';
    }>('track-progress', event => {
      const { trackId, phase, percent } = event.payload;
      setTracks(prev =>
        prev.map(t => (t.id === trackId ? { ...t, status: phase, progress: percent } : t))
      );
    }).catch(err => {
      console.error('[track-progress] failed to subscribe to progress events:', err);
      return undefined;
    });

    return () => {
      unlistenPromise.then(unlisten => unlisten?.());
    };
  }, []);

  useEffect(() => {
    const unlistenPromise = listen<{ completed: number; total: number }>(
      'analyze-progress',
      event => {
        setAnalyzeProgress(event.payload);
      }
    ).catch(err => {
      console.error('[analyze-progress] failed to subscribe to progress events:', err);
      return undefined;
    });

    return () => {
      unlistenPromise.then(unlisten => unlisten?.());
    };
  }, []);

  useEffect(() => {
    getSettings()
      .then(s => {
        if (s.youtubeCookies || s.cookiesFromBrowser) {
          setSettings(prev => ({
            ...prev,
            youtubeCookies: s.youtubeCookies ?? prev.youtubeCookies,
            cookiesFromBrowser: s.cookiesFromBrowser ?? prev.cookiesFromBrowser,
          }));
        }
      })
      .catch(err => console.error('[App] failed to load persisted cookies:', err));
  }, []);

  useEffect(() => {
    const timer = setTimeout(() => {
      setYoutubeCookies(settings.youtubeCookies).catch(err =>
        console.error('[App] failed to sync YouTube cookies:', err)
      );
      setYoutubeCookiesFromBrowser(settings.cookiesFromBrowser).catch(err =>
        console.error('[App] failed to sync cookies-from-browser:', err)
      );
    }, 400);
    return () => clearTimeout(timer);
  }, [settings.youtubeCookies, settings.cookiesFromBrowser]);

  const handlePlayTrack = (track: Track) => {
    if (playingTrack?.id === track.id) {
      setPlayingTrack(null);
    } else {
      setPlayingTrack(track);
    }
  };

  const handleAnalyze = async (e: React.SubmitEvent) => {
    e.preventDefault();
    if (!sourceInput) {
      setError('Please paste a link or enter song search terms first.');
      return;
    }

    setIsAnalyzing(true);
    setError(null);
    setTracks([]);
    setPlaylistName('');
    setIsAlbum(false);
    setAnalyzeProgress(null);

    try {
      await Promise.all([
        setYoutubeCookies(settings.youtubeCookies),
        setYoutubeCookiesFromBrowser(settings.cookiesFromBrowser),
      ]);
    } catch (err) {
      console.error('[App] failed to sync cookies before analyze:', err);
    }

    try {
      const data = await analyzeLink(sourceInput);

      if (data.type === 'playlist') {
        setIsPlaylist(true);
        setPlaylistName(data.playlistName);
        setIsAlbum(data.isAlbum);
        setTracks(
          data.tracks.map(t => ({
            ...t,
            status: 'idle',
            progress: 0,
          }))
        );
      } else {
        setIsPlaylist(false);
        setTracks([
          {
            ...data.track,
            status: 'idle',
            progress: 0,
          },
        ]);
      }
      setStep('configure');
    } catch (err) {
      console.error(err);
      setError(friendlyError(err) || 'Something went wrong. Please check your link and try again.');
    } finally {
      setIsAnalyzing(false);
    }
  };

  const handleDownloadSingle = async (trackToDownload: Track) => {
    setTracks(prev =>
      prev.map(t =>
        t.id === trackToDownload.id
          ? { ...t, status: 'scraping', progress: 0, error: undefined }
          : t
      )
    );

    try {
      await downloadTrack(trackToDownload, {
        format: settings.format,
        bitrate: settings.bitrate,
        youtubeCookies: settings.youtubeCookies,
        cookiesFromBrowser: settings.cookiesFromBrowser,
        sampleRate: settings.sampleRate,
        videoQuality: settings.videoQuality,
        namingPattern: settings.namingPattern || 'artist_title',
        embedId3Tags: settings.embedId3Tags !== false,
      });

      setTracks(prev =>
        prev.map(t =>
          t.id === trackToDownload.id ? { ...t, status: 'completed', progress: 100 } : t
        )
      );
    } catch (err) {
      console.error(`Download failed for "${trackToDownload.title}":`, err);
      if (getErrorCode(err) === 'CANCELLED') {
        setTracks(prev =>
          prev.map(t =>
            t.id === trackToDownload.id ? { ...t, status: 'cancelled', progress: 0 } : t
          )
        );
        return;
      }
      const message = friendlyError(err);
      setTracks(prev =>
        prev.map(t =>
          t.id === trackToDownload.id
            ? { ...t, status: 'failed', progress: 0, error: message || 'Failed to process track.' }
            : t
        )
      );
    }
  };

  const handleCancelSingle = async (track: Track) => {
    try {
      await cancelDownload(track.id);
    } catch (err) {
      console.error(`Failed to cancel "${track.title}":`, err);
    }
  };

  const handleDownloadAll = async () => {
    if (tracks.length === 0) return;

    setIsBatchDownloading(true);
    setTracks(prev =>
      prev.map(t => ({
        ...t,
        status: 'scraping',
        progress: 0,
        error: undefined,
      }))
    );

    try {
      await downloadBatch(tracks, {
        format: settings.format,
        bitrate: settings.bitrate,
        playlistName: playlistName,
        youtubeCookies: settings.youtubeCookies,
        cookiesFromBrowser: settings.cookiesFromBrowser,
        sampleRate: settings.sampleRate,
        videoQuality: settings.videoQuality,
        skipMissingTracks: settings.skipMissingTracks,
        namingPattern: settings.namingPattern || 'artist_title',
        embedId3Tags: settings.embedId3Tags !== false,
        folderNamingPattern: isAlbum ? settings.folderNamingPattern || 'album_artist' : undefined,
        isAlbum: isAlbum,
      });

      setTracks(prev => prev.map(t => ({ ...t, status: 'completed', progress: 100 })));
    } catch (err) {
      console.error('Batch download failed:', err);
      if (getErrorCode(err) === 'CANCELLED') {
        setTracks(prev =>
          prev.map(t => (t.status === 'completed' ? t : { ...t, status: 'cancelled', progress: 0 }))
        );
      } else {
        const message = friendlyError(err);
        setError(`Batch download failed: ${message}`);
        setTracks(prev => prev.map(t => ({ ...t, status: 'failed', progress: 0 })));
      }
    } finally {
      setIsBatchDownloading(false);
      setIsCancellingBatch(false);
    }
  };

  const handleCancelBatch = async () => {
    setIsCancellingBatch(true);
    try {
      await cancelBatch(tracks.map(t => t.id));
    } catch (err) {
      console.error('Failed to cancel batch download:', err);
      setIsCancellingBatch(false);
    }
  };

  const handleDownloadToFolder = async () => {
    if (tracks.length === 0) return;

    try {
      const current = await getSettings();
      if (!current.downloadFolder) {
        const picked = await pickDownloadFolder();
        if (!picked || !picked.downloadFolder) return;
      }
    } catch (err) {
      setError('Could not read download settings: ' + (friendlyError(err) || 'unknown error.'));
      return;
    }

    setIsFolderDownloading(true);
    setError(null);

    const total = tracks.length;
    let completedCount = 0;

    const collectionName =
      playlistName ||
      tracks[0]?.album ||
      tracks[0]?.year ||
      (tracks[0]?.artist ? `${tracks[0].artist} - Album` : 'Unknown Album');
    const FOLDER_DOWNLOAD_CONCURRENCY = 5;

    let nextIndex = 0;
    let stopRequested = false;

    const runNext = async (): Promise<void> => {
      while (!stopRequested) {
        const i = nextIndex++;
        if (i >= total) return;

        const track = tracks[i];
        setTracks(prev =>
          prev.map(t => (t.id === track.id ? { ...t, status: 'scraping', progress: 0 } : t))
        );

        try {
          await downloadTrack(track, {
            format: settings.format,
            bitrate: settings.bitrate,
            youtubeCookies: settings.youtubeCookies,
            cookiesFromBrowser: settings.cookiesFromBrowser,
            sampleRate: settings.sampleRate,
            videoQuality: settings.videoQuality,
            namingPattern: settings.namingPattern || 'artist_title',
            embedId3Tags: settings.embedId3Tags !== false,
            albumFolder: total > 1 ? collectionName : undefined,
            folderNamingPattern: isAlbum
              ? settings.folderNamingPattern || 'album_artist'
              : undefined,
            albumName: isAlbum ? collectionName : undefined,
            isAlbum: isAlbum,
          });

          completedCount++;
          setTracks(prev =>
            prev.map(t => (t.id === track.id ? { ...t, status: 'completed', progress: 100 } : t))
          );
        } catch (err) {
          console.error(`Folder download error on track ${track.title}:`, err);
          if (getErrorCode(err) === 'CANCELLED') {
            setTracks(prev =>
              prev.map(t => (t.id === track.id ? { ...t, status: 'cancelled', progress: 0 } : t))
            );
            stopRequested = true;
            return;
          }
          const message = friendlyError(err);
          setTracks(prev =>
            prev.map(t =>
              t.id === track.id ? { ...t, status: 'failed', progress: 0, error: message } : t
            )
          );
          if (!settings.skipMissingTracks) {
            stopRequested = true;
            setError(`Folder download stopped: ${message}`);
            return;
          }
        }

        setFolderProgressText(`Downloaded ${completedCount}/${total} tracks…`);
      }
    };

    const workerCount = Math.min(FOLDER_DOWNLOAD_CONCURRENCY, total);
    await Promise.all(Array.from({ length: workerCount }, runNext));

    setIsCancellingBatch(false);
    setFolderProgressText(
      `Saved ${completedCount} of ${total} tracks to your configured download folder.`
    );
    setTimeout(() => {
      setIsFolderDownloading(false);
      setFolderProgressText('');
    }, 3000);
  };

  const [loadingPhraseIndex, setLoadingPhraseIndex] = useState(0);
  const loadingPhrases = [
    'Querying official registry database...',
    'Matching stream frequencies with metadata...',
    'Syncing high-fidelity 600x600px album art...',
    'Injecting catalog tagging descriptors...',
    'Finalizing raw audio buffers...',
    'Hang tight, large playlists take a while!',
  ];

  useEffect(() => {
    let interval: ReturnType<typeof setInterval>;
    if (isAnalyzing) {
      interval = setInterval(() => {
        setLoadingPhraseIndex(prev => (prev + 1) % loadingPhrases.length);
      }, 2500);
    }
    return () => clearInterval(interval);
  }, [isAnalyzing, loadingPhrases.length]);

  const goToSource = () => {
    setStep('source');
    setTracks([]);
    setPlaylistName('');
    setIsAlbum(false);
    setError(null);
  };

  const goToConfigure = () => setStep('configure');

  const goToResults = () => setStep('results');

  return (
    <div className="flex min-h-screen w-full overflow-hidden p-2.5">
      <div className="flex flex-col justify-center h-full w-full relative z-10">
        <header className="flex flex-col items-center md:items-start mb-3 px-2">
          <div className="flex items-center gap-3">
            <AudioLines className="w-8 h-8 text-rust animate-pulse" />
            <h1 className="text-3xl text-olive sm:text-4xl uppercase">
              SONIC<span className="">·</span>RIPPER
            </h1>
          </div>
          <p className="tracking-wider text-xs font-bold uppercase mt-2">Audio & Video Extractor</p>
        </header>

        <StepProgress current={step} />

        <AnimatePresence mode="wait">
          {step === 'source' && (
            <motion.div
              key="step-source"
              initial={{ opacity: 0, x: 24 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: -24 }}
              transition={{ duration: 0.35, ease: 'easeOut' }}
              className="w-full h-full px-3 py-4 bg-charcoal/40 border-b-2 border-olive/55 rounded-md"
            >
              <section className="shadow-xl h-full my-auto relative">
                <span className="uppercase font-display text-rust font-bold flex items-center mb-2">
                  Add a source
                </span>

                <form onSubmit={handleAnalyze} className="space-y-4 w-full">
                  <div className="flex flex-row items-center justify-between w-full gap-5">
                    <div className="flex flex-1 flex-row items-center border-2 border-rust rounded-sm">
                      <div className="h-full bg-rust p-2 justify-start">
                        <Search className="size-7" />
                      </div>
                      <input
                        type="text"
                        name="sourceInput"
                        value={sourceInput}
                        onChange={e => {
                          setSourceInput(e.target.value);
                          setError(null);
                        }}
                        placeholder="Paste a Spotify/Youtube link or search directly..."
                        className="justify-start flex-1 py-2 px-3 outline-none transition-all"
                        disabled={isAnalyzing || isBatchDownloading}
                      />
                      {sourceInput && (
                        <button
                          type="button"
                          onClick={() => {
                            setSourceInput('');
                            setError(null);
                          }}
                          disabled={isAnalyzing || isBatchDownloading}
                          className="flex justify-end h-full bg-rust/30 hover:rust/90 active:scale-102 p-2 transition-all duration-300 items-center cursor-pointer disabled:opacity-40"
                          title="Clear"
                        >
                          <X className="size-7" />
                        </button>
                      )}
                    </div>

                    <button
                      type="submit"
                      disabled={isAnalyzing || isBatchDownloading || !sourceInput}
                      className="px-5 py-2.5 w-28.75 font-medium bg-olive/60 hover:bg-olive rounded-sm active:scale-98 transition-all duration-300 flex items-center justify-center gap-2 cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed"
                    >
                      {isAnalyzing ? (
                        <RefreshCw className="size-5 animate-spin" />
                      ) : (
                        <>
                          Search
                          <ArrowRight className="w-3.5 h-3.5" />
                        </>
                      )}
                    </button>
                  </div>
                </form>

                {/* Standard Errors Display */}
                <AnimatePresence>
                  {error &&
                    !(
                      error.includes('429') ||
                      error.includes('quota') ||
                      error.includes('Quota') ||
                      error.includes('limit') ||
                      error.includes('EXHAUSTED')
                    ) && (
                      <motion.div
                        initial={{ opacity: 0, y: -8 }}
                        animate={{ opacity: 1, y: 0 }}
                        exit={{ opacity: 0, y: -8 }}
                        id="error-block"
                        className="mt-4 p-4 text-xs flex items-start gap-3"
                      >
                        <AlertCircle className="w-5 h-5 shrink-0 mt-0.5 text-status-failed" />
                        <p className="leading-normal">{error}</p>
                      </motion.div>
                    )}
                </AnimatePresence>

                {/* Analyzing state */}
                <AnimatePresence>
                  {isAnalyzing && (
                    <motion.div
                      initial={{ opacity: 0, y: 12 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0 }}
                      className="mt-6 p-8 text-center flex flex-col items-center justify-center"
                    >
                      <div className="relative mb-5">
                        <div className="w-12 h-12 rounded-full animate-spin" />
                        <Sparkles className="size-10 absolute inset-0 m-auto animate-pulse" />
                      </div>
                      <h3 className="font-display uppercase tracking-[0.25em] mb-2">
                        Analyzing Catalog Metadata
                      </h3>
                      {analyzePercent !== null ? (
                        <div className="w-full max-w-md">
                          <div className="w-full h-2 rounded-sm overflow-hidden bg-olive/30">
                            <motion.div
                              className="bg-gold h-full"
                              initial={{ width: '0%' }}
                              animate={{ width: `${analyzePercent}%` }}
                              transition={{ duration: 0.2 }}
                            />
                          </div>
                        </div>
                      ) : (
                        <p className="h-5 transition-all duration-300">
                          {loadingPhrases[loadingPhraseIndex]}
                        </p>
                      )}
                    </motion.div>
                  )}
                </AnimatePresence>

                <SpotifyPathfinderSettings />
              </section>
            </motion.div>
          )}

          {step === 'configure' && (
            <motion.div
              key="step-configure"
              initial={{ opacity: 0, x: 24 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: -24 }}
              transition={{ duration: 0.35, ease: 'easeOut' }}
              className="grid grid-cols-1 md:grid-cols-2 md:max-h-[75vh] items-start bg-charcoal/60 px-2.5 py-3 rounded-2xl"
            >
              <div className="space-y-6">
                <section className="md:border-b md:border-olive">
                  <span className="font-display uppercase font-bold flex items-center gap-2 mb-5">
                    <span className="w-1.5 h-1.5" /> Found and ready
                  </span>

                  {isPlaylist ? (
                    <div className="flex items-center gap-4">
                      <div className="w-16 h-16 flex items-center justify-center shrink-0">
                        <AudioLines className="size-5 text-rust" />
                      </div>
                      <div className="min-w-0">
                        <p className="text-lg font-semibold text-cream truncate">{playlistName}</p>
                        <p className="text-[13px] mt-1">
                          {tracks.length} tracks parsed successfully
                        </p>
                      </div>
                    </div>
                  ) : (
                    <div className="flex items-center gap-4">
                      <img
                        src={tracks[0]?.coverUrl}
                        alt={tracks[0]?.title}
                        referrerPolicy="no-referrer"
                        className="w-16 h-16 object-cover shrink-0"
                      />
                      <div className="min-w-0">
                        <p className="text-lg font-semibold truncate">{tracks[0]?.title}</p>
                        <p className="text-[12px] font-display md:text-[14px] mt-0.5 truncate">
                          {tracks[0]?.artist}
                        </p>
                      </div>
                    </div>
                  )}

                  <div className="flex flex-row items-center justify-between md:px-3 my-6">
                    <button
                      type="button"
                      onClick={goToSource}
                      className="p-2 text-xs md:text-[13px] bg-olive rounded-md uppercase transition-colors cursor-pointer"
                    >
                      ← Back
                    </button>

                    <button
                      type="button"
                      onClick={goToResults}
                      className="flex text-xs md:text-[13px] items-center gap-2.5 p-2 text-rust bg-charcoal hover:ring-2 hover:ring-rust rounded-md uppercase tracking-wide transition-all duration-300 cursor-pointer"
                    >
                      <span>Continue to Download</span>
                      <ArrowRight className="size-5" />
                    </button>
                  </div>
                </section>
              </div>

              <div
                className="md:h-[98%] overflow-y-auto pr-1 md:border-l md:border-olive"
                style={{ scrollbarWidth: 'thin', scrollbarColor: 'var(--color-gold)' }}
              >
                <SettingsPanel settings={settings} onChange={setSettings} />
              </div>
            </motion.div>
          )}

          {step === 'results' && (
            <motion.div
              key="step-results"
              initial={{ opacity: 0, x: 24 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: -24 }}
              transition={{ duration: 0.35, ease: 'easeOut' }}
              className="px-2.5"
            >
              <div className="flex items-center justify-between my-2.5">
                <button
                  type="button"
                  onClick={goToConfigure}
                  className="text-xs bg-olive p-2 scale-98 hover:scale-100 rounded-md uppercase tracking-[0.2em] transition-all duration-300 cursor-pointer"
                >
                  ← Back
                </button>
                <button
                  type="button"
                  onClick={goToSource}
                  className="text-xs p-2 bg-rust scale-98 hover:scale-100 rounded-md uppercase tracking-[0.2em] transition-all duration-300 cursor-pointer"
                >
                  Start over
                </button>
              </div>

              <motion.section
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                id="results-section"
                className="mt-5"
              >
                {/* Playlist Batch header */}
                {isPlaylist && (
                  <div
                    id="batch-actions-header"
                    className="rounded-none flex flex-col md:flex-row md:items-center md:justify-between gap-4"
                  >
                    <div className="text-left md:items-start">
                      <h4 className="text uppercase tracking-[0.3em]">Batch Download Options</h4>
                      <p className="text-[11px] md:text-sm text-rust mt-1">
                        Export all {tracks.length} tracks with embedded ID3 tags, artwork, and your
                        custom naming pattern.
                      </p>
                    </div>

                    <div className="flex flex-row items-start justify-between gap-3">
                      {/* Save directly to folder button */}
                      <button
                        type="button"
                        onClick={handleDownloadToFolder}
                        disabled={isBatchDownloading || isFolderDownloading}
                        className="px-4 py-3 border-2 border-gold hover:bg-gold rounded-md text-xs uppercase tracking-[0.15em] transition-colors duration-300 flex items-center justify-center gap-2 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-transparent"
                        title="Select a directory on your machine to save all tagged audio files directly into that folder"
                      >
                        {isFolderDownloading ? (
                          <RefreshCw className="size-3 animate-spin" />
                        ) : (
                          <FolderPlus className="size-4 text-cream" />
                        )}
                        <span>Save to Folder</span>
                      </button>

                      {/* Download as ZIP button */}
                      <button
                        type="button"
                        onClick={handleDownloadAll}
                        disabled={isBatchDownloading || isFolderDownloading}
                        className="px-4 py-3 border-2 border-rust hover:bg-rust text-xs rounded-md uppercase tracking-[0.2em] transition-colors duration-300 flex items-center justify-center gap-2 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-transparent"
                        title="Download a single .ZIP archive containing all tagged tracks and folders"
                      >
                        {isBatchDownloading ? (
                          <RefreshCw className="size-3 animate-spin" />
                        ) : (
                          <Archive className="size-4 text-cream" />
                        )}
                        <span>Save as ZIP</span>
                      </button>
                    </div>
                  </div>
                )}

                {/* Folder Download Progress Card */}
                {isFolderDownloading && (
                  <div className="bg-olive/35 p-2 md:p-5 rounded-md my-2 items-center">
                    <div className="flex items-center justify-between text-xs font-bold tracking-widest text-brand uppercase my-0.5">
                      <span className="flex items-center text-charcoal gap-2">
                        <FolderCheck className="size-5 animate-pulse" />
                        Saving directly into your selected folder...
                      </span>
                      <button
                        type="button"
                        onClick={handleCancelBatch}
                        disabled={isCancellingBatch}
                        className="flex items-center gap-1 text-[10px] px-2 py-1 rounded-sm bg-charcoal text-cream hover:bg-rust transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-charcoal"
                      >
                        <X className="size-3" />
                        {isCancellingBatch ? 'Cancelling…' : 'Cancel'}
                      </button>
                    </div>
                    <p className="text-xs text-gold">{folderProgressText}</p>
                  </div>
                )}

                {/* Batch Download Progress Bar */}
                {isBatchDownloading && (
                  <div className="bg-olive/35 p-2 md:p-5 rounded-md my-2 items-center">
                    <div className="flex items-center justify-between text-xs font-bold tracking-widest text-brand uppercase my-0.5">
                      <span className="flex items-center text-charcoal gap-2">
                        <RefreshCw className="size-5 animate-spin" />
                        Transcoding, tagging, and archiving tracks into ZIP...
                      </span>
                      <div className="flex items-center gap-3">
                        <span>{batchProgress}%</span>
                        <button
                          type="button"
                          onClick={handleCancelBatch}
                          disabled={isCancellingBatch}
                          className="flex items-center gap-1 text-[10px] px-2 py-1 rounded-sm bg-charcoal text-cream hover:bg-rust transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-charcoal"
                        >
                          <X className="size-3" />
                          {isCancellingBatch ? 'Cancelling…' : 'Cancel'}
                        </button>
                      </div>
                    </div>
                    <div className="w-full h-2 bg-charcoal rounded-none overflow-hidden">
                      <motion.div
                        className="bg-rust h-full"
                        initial={{ width: '0%' }}
                        animate={{ width: `${batchProgress}%` }}
                        transition={{ duration: 0.1 }}
                      />
                    </div>
                    <p className="text-[9px] mt-2 uppercase tracking-wide">
                      Tracks are tagged with metadata, covers, and organized inside your archive.
                    </p>
                  </div>
                )}

                {/* Track list */}
                <TrackListHeader trackCount={tracks.length} playlistName={playlistName} />
                <div ref={containerRef}>
                  <div style={{ height: paddingTop, flexShrink: 0 }} />
                  {virtualItems.map(index => {
                    const track = tracks[index];
                    if (!track) return null;
                    return (
                      <div key={track.id} data-index={index} ref={measureElement}>
                        <TrackRow
                          track={track}
                          index={index}
                          onDownloadSingle={handleDownloadSingle}
                          onCancelSingle={handleCancelSingle}
                          onPlayTrack={handlePlayTrack}
                          activeTrackId={playingTrack?.id}
                          isBatchDownloading={isBatchDownloading}
                          isFolderDownloading={isFolderDownloading}
                        />
                      </div>
                    );
                  })}
                  <div style={{ height: paddingBottom, flexShrink: 0 }} />
                </div>
              </motion.section>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      {/* Immersive Floating Media Workstation Preview Player */}
      <AnimatePresence>
        {playingTrack && <MediaPlayer track={playingTrack} onClose={() => setPlayingTrack(null)} />}
      </AnimatePresence>
    </div>
  );
}
