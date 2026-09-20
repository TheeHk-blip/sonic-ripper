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
import LanguageSelector from './components/LanguageSelector';
import AlbumCoverDropzone from './components/AlbumCoverDropzone';
import { useI18n, Translations } from './lib/i18n';
import { Track, DownloadSettings } from './types';
import { listen } from '@tauri-apps/api/event';
import {
  analyzeSpotify as analyzeLink,
  downloadTrack,
  downloadBatch,
  getSettings,
  pickDownloadFolder,
  generateTrackSpectrogram,
} from './lib/api';
import { useVirtualizer } from './lib/useVirtualizer';
import TrackRow from './components/TrackRow';
import TrackListHeader from './components/TrackListHeader';

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function friendlyError(err: any, t: Translations): string {
  const code = err?.code as string | undefined;
  const raw = String(err?.message ?? err?.error ?? err ?? '');

  if (code === 'YOUTUBE_BOT_DETECTED') {
    return t.errYoutubeBot;
  }
  if (code === 'TRACK_NOT_FOUND') {
    return t.errTrackNotFound;
  }
  if (code === 'FORBIDDEN') {
    return t.errForbidden;
  }
  if (raw.includes('No download folder is set')) {
    return t.errNoDownloadFolder;
  }
  if (raw.includes('Failed to parse Spotify URL') || raw.includes('SpotifyParseFailed')) {
    return t.errInvalidUrl;
  }
  if (raw.includes('Network error')) {
    return t.errNetwork;
  }
  if (raw.toLowerCase().includes('ffmpeg')) {
    return t.errFfmpeg;
  }
  if (raw.toLowerCase().includes('yt-dlp')) {
    return t.errYtDlp;
  }
  return t.errGeneric;
}

export default function App() {
  const { t } = useI18n();
  const [step, setStep] = useState<FlowStep>('source');
  const [sourceInput, setSourceInput] = useState('');
  const [isAnalyzing, setIsAnalyzing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tracks, setTracks] = useState<Track[]>([]);
  const [playlistName, setPlaylistName] = useState<string>('');
  const [isAlbum, setIsAlbum] = useState<boolean>(false);
  const [isPlaylist, setIsPlaylist] = useState(false);
  const [selectedTrackIndex, setSelectedTrackIndex] = useState<number>(0);
  const [applyCoverToAll, setApplyCoverToAll] = useState<boolean>(false);
  const [spectrogramProgress, setSpectrogramProgress] = useState<{
    current: number;
    total: number;
  } | null>(null);
  const [playingTrack, setPlayingTrack] = useState<Track | null>(null);
  const [settings, setSettings] = useState<DownloadSettings>({
    format: 'flac',
    bitrate: 'lossless',
    saveInFolder: true,
    skipMissingTracks: true,
    namingPattern: '{artist}/{year} - {album}/{trackNumber} - {title}',
    embedId3Tags: true,
    downloadLyrics: false,
  });
  const [isBatchDownloading, setIsBatchDownloading] = useState(false);
  const batchProgress = useMemo(() => {
    if (!isBatchDownloading || tracks.length === 0) return 0;
    const avg =
      tracks.reduce((sum, t) => sum + (t.status === 'completed' ? 100 : t.progress), 0) /
      tracks.length;
    return Math.round(avg);
  }, [tracks, isBatchDownloading]);
  const [isFolderDownloading, setIsFolderDownloading] = useState(false);
  const [folderProgressText, setFolderProgressText] = useState('');

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
    getSettings()
      .then(s => {
        if (s.namingPattern) {
          setSettings(prev => ({ ...prev, namingPattern: s.namingPattern || prev.namingPattern }));
        }
      })
      .catch(() => {});
  }, []);

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
      setError(t.searchEmptyError);
      return;
    }

    setIsAnalyzing(true);
    setError(null);
    setTracks([]);
    setPlaylistName('');
    setIsAlbum(false);
    setSelectedTrackIndex(0);
    setApplyCoverToAll(false);

    try {
      const data = await analyzeLink(sourceInput);

      if (data.type === 'playlist') {
        setIsPlaylist(true);
        setPlaylistName(data.playlistName);
        setIsAlbum(data.isAlbum);
        setTracks(
          data.tracks.map(t => ({
            ...t,
            originalCoverUrl: t.coverUrl,
            status: 'idle',
            progress: 0,
          }))
        );
      } else {
        setIsPlaylist(false);
        setTracks([
          {
            ...data.track,
            originalCoverUrl: data.track.coverUrl,
            status: 'idle',
            progress: 0,
          },
        ]);
      }
      setStep('configure');
    } catch (err) {
      console.error(err);
      setError(friendlyError(err, t) || t.errGeneric);
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
        namingPattern:
          settings.namingPattern || '{artist}/{year} - {album}/{trackNumber} - {title}',
        embedId3Tags: settings.embedId3Tags !== false,
        downloadLyrics: Boolean(settings.downloadLyrics),
      });

      setTracks(prev =>
        prev.map(t =>
          t.id === trackToDownload.id ? { ...t, status: 'completed', progress: 100 } : t
        )
      );
    } catch (err) {
      console.error(`Download failed for "${trackToDownload.title}":`, err);
      const message = friendlyError(err, t);
      setTracks(prev =>
        prev.map(trk =>
          trk.id === trackToDownload.id
            ? { ...trk, status: 'failed', progress: 0, error: message || t.errGeneric }
            : trk
        )
      );
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
        playlistName: playlistName || t.playlistDefault,
        youtubeCookies: settings.youtubeCookies,
        cookiesFromBrowser: settings.cookiesFromBrowser,
        sampleRate: settings.sampleRate,
        videoQuality: settings.videoQuality,
        skipMissingTracks: settings.skipMissingTracks,
        namingPattern:
          settings.namingPattern || '{artist}/{year} - {album}/{trackNumber} - {title}',
        embedId3Tags: settings.embedId3Tags !== false,
        downloadLyrics: Boolean(settings.downloadLyrics),
      });

      setTracks(prev => prev.map(t => ({ ...t, status: 'completed', progress: 100 })));
    } catch (err) {
      console.error('Batch download failed:', err);
      const message = friendlyError(err, t);
      setError(`${t.batchFailed}: ${message}`);
      setTracks(prev => prev.map(t => ({ ...t, status: 'failed', progress: 0 })));
    } finally {
      setIsBatchDownloading(false);
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
      setError(`${t.errFolderSettingsRead}: ${friendlyError(err, t) || t.errGeneric}`);
      return;
    }

    setIsFolderDownloading(true);
    setError(null);

    const total = tracks.length;
    let completedCount = 0;

    const collectionName = playlistName || tracks[0]?.album || t.playlistDefault;
    const folderName =
      isAlbum && tracks[0]?.artist ? `${collectionName} - ${tracks[0].artist}` : collectionName;
    const activePattern =
      settings.namingPattern || '{artist}/{year} - {album}/{trackNumber} - {title}';
    const hasCustomFolders = activePattern.includes('/') || activePattern.includes('\\');
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
            namingPattern: activePattern,
            embedId3Tags: settings.embedId3Tags !== false,
            downloadLyrics: Boolean(settings.downloadLyrics),
            albumFolder: hasCustomFolders ? undefined : total > 1 ? folderName : undefined,
            playlistName: playlistName || tracks[0]?.album,
          });

          completedCount++;
          setTracks(prev =>
            prev.map(t => (t.id === track.id ? { ...t, status: 'completed', progress: 100 } : t))
          );
        } catch (err) {
          console.error(`Folder download error on track ${track.title}:`, err);
          const message = friendlyError(err, t);
          setTracks(prev =>
            prev.map(t =>
              t.id === track.id ? { ...t, status: 'failed', progress: 0, error: message } : t
            )
          );
          if (!settings.skipMissingTracks) {
            stopRequested = true;
            setError(`${t.errFolderStopped}: ${message}`);
            return;
          }
        }

        setFolderProgressText(t.downloadedCount(completedCount, total));
      }
    };

    const workerCount = Math.min(FOLDER_DOWNLOAD_CONCURRENCY, total);
    await Promise.all(Array.from({ length: workerCount }, runNext));

    setFolderProgressText(t.savedToFolderText(completedCount, total));
    setTimeout(() => {
      setIsFolderDownloading(false);
      setFolderProgressText('');
    }, 3000);
  };

  const [loadingPhraseIndex, setLoadingPhraseIndex] = useState(0);
  const loadingPhrases = t.loadingPhrases;

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
    setSelectedTrackIndex(0);
    setApplyCoverToAll(false);
    setError(null);
  };

  const goToConfigure = () => setStep('configure');

  const goToResults = () => setStep('results');

  const activeTrack = tracks[selectedTrackIndex] || tracks[0];
  const isSelectedTrackCustom = activeTrack
    ? activeTrack.coverUrl !== (activeTrack.originalCoverUrl || '')
    : false;

  const handleCoverChange = (newCover: string) => {
    if (applyCoverToAll) {
      setTracks(prev => prev.map(t => ({ ...t, coverUrl: newCover })));
    } else {
      setTracks(prev =>
        prev.map((t, idx) => (idx === selectedTrackIndex ? { ...t, coverUrl: newCover } : t))
      );
    }
  };

  const handleResetCover = () => {
    if (applyCoverToAll) {
      setTracks(prev =>
        prev.map(t => ({
          ...t,
          coverUrl: t.originalCoverUrl || t.coverUrl,
        }))
      );
    } else {
      setTracks(prev =>
        prev.map((t, idx) =>
          idx === selectedTrackIndex ? { ...t, coverUrl: t.originalCoverUrl || t.coverUrl } : t
        )
      );
    }
  };

  const handlePrevTrack = () => {
    if (tracks.length === 0) return;
    setSelectedTrackIndex(prev => (prev > 0 ? prev - 1 : tracks.length - 1));
  };

  const handleNextTrack = () => {
    if (tracks.length === 0) return;
    setSelectedTrackIndex(prev => (prev < tracks.length - 1 ? prev + 1 : 0));
  };

  const handleGenerateSpectrogram = async (palette = 'magma') => {
    if (tracks.length === 0) return;

    try {
      setError(null);

      if (!applyCoverToAll) {
        if (!activeTrack) return;
        const specDataUrl = await generateTrackSpectrogram(
          activeTrack,
          palette,
          settings.youtubeCookies,
          settings.cookiesFromBrowser
        );
        setTracks(prev =>
          prev.map((t, idx) => (idx === selectedTrackIndex ? { ...t, coverUrl: specDataUrl } : t))
        );
      } else {
        setSpectrogramProgress({ current: 0, total: tracks.length });
        let completed = 0;
        const concurrency = Math.min(2, tracks.length);
        let nextIndex = 0;

        const worker = async () => {
          while (nextIndex < tracks.length) {
            const idx = nextIndex++;
            const track = tracks[idx];
            if (!track.previewUrl) {
              completed++;
              setSpectrogramProgress({ current: completed, total: tracks.length });
              continue;
            }

            try {
              const specDataUrl = await generateTrackSpectrogram(
                track,
                palette,
                settings.youtubeCookies,
                settings.cookiesFromBrowser
              );
              setTracks(prev =>
                prev.map((t, i) => (i === idx ? { ...t, coverUrl: specDataUrl } : t))
              );
            } catch (err) {
              console.warn(`Spectrogram generation failed for "${track.title}":`, err);
            } finally {
              completed++;
              setSpectrogramProgress({ current: completed, total: tracks.length });
            }
          }
        };

        await Promise.all(Array.from({ length: concurrency }, worker));
      }
    } catch (err) {
      console.error('Failed to generate spectrogram:', err);
      setError(friendlyError(err, t) || t.errGeneric);
    } finally {
      setSpectrogramProgress(null);
    }
  };

  return (
    <div className="flex min-h-screen w-full overflow-hidden p-2.5">
      <div className="flex flex-col justify-center h-full w-full relative z-10">
        <header className="flex flex-col sm:flex-row items-center justify-between mb-3 px-2">
          <div className="flex flex-col items-center sm:items-start">
            <div className="flex items-center gap-3">
              <AudioLines className="w-8 h-8 text-rust animate-pulse" />
              <h1 className="text-3xl text-olive sm:text-4xl uppercase">
                SONIC<span className="">·</span>RIPPER
              </h1>
            </div>
            <p className="tracking-wider text-xs uppercase mt-2">{t.subtitle}</p>
          </div>
          <AnimatePresence>
            {step === 'source' && (
              <motion.div
                initial={{ opacity: 0, scale: 0.95 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.95 }}
                transition={{ duration: 0.2 }}
                className="mt-3 sm:mt-0"
              >
                <LanguageSelector />
              </motion.div>
            )}
          </AnimatePresence>
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
              className="mx-auto px-2.5 justify-center w-full h-full bg-charcoal/40 border-b-2 border-olive/55 rounded-md"
            >
              <section className="px-6 py-3 sm:px-8 sm:py-4 shadow-xl h-full my-auto relative">
                <div className="flex items-center justify-between mb-5">
                  <span className="uppercase font-display text-rust font-bold flex items-center gap-2">
                    <span className="w-1.5 h-1.5" /> {t.addSourceTitle}
                  </span>
                </div>

                <form onSubmit={handleAnalyze} className="space-y-4">
                  <div className="flex flex-col sm:flex-row items-stretch gap-3">
                    <div className="flex items-stretch grow rounded-sm border-2 border-rust w-full overflow-hidden">
                      <div className="flex shrink-0 w-10 md:w-12 bg-rust items-center justify-center pointer-events-none">
                        <Search className="size-5 md:size-7" />
                      </div>
                      <input
                        type="text"
                        name="sourceInput"
                        value={sourceInput}
                        onChange={e => {
                          setSourceInput(e.target.value);
                          setError(null);
                        }}
                        placeholder={t.searchPlaceholder}
                        className="flex-1 min-w-0 py-4 px-2 outline-none transition-all"
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
                          className="flex shrink-0 bg-rust/30 hover:rust/90 active:scale-102 w-10 md:w-12 px-2 transition-all duration-300 items-center cursor-pointer disabled:opacity-40"
                          title={t.clearInput}
                        >
                          <X className="size-5" />
                        </button>
                      )}
                    </div>

                    <button
                      type="submit"
                      disabled={isAnalyzing || isBatchDownloading || !sourceInput}
                      className="sm:w-44 py-4 font-medium bg-olive/60 hover:bg-olive rounded-sm active:scale-98 transition-all duration-300 flex items-center justify-center gap-2 cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed"
                    >
                      {isAnalyzing ? (
                        <span className="flex items-center gap-2">
                          <RefreshCw className="size-5 animate-spin" />
                          <span>{t.analyzingBtn}</span>
                        </span>
                      ) : (
                        <>
                          {t.searchBtn}
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
                      id="analyzer-loader-card"
                      className="mt-6 p-8 text-center flex flex-col items-center justify-center"
                    >
                      <div className="relative mb-5">
                        <div className="w-12 h-12 rounded-full animate-spin" />
                        <Sparkles className="w-5 h-5 absolute inset-0 m-auto animate-pulse" />
                      </div>
                      <h3 className="font-display uppercase tracking-[0.25em] mb-2">
                        {t.analyzingCatalogTitle}
                      </h3>
                      <p className="h-5 transition-all duration-300">
                        {loadingPhrases[loadingPhraseIndex]}
                      </p>
                    </motion.div>
                  )}
                </AnimatePresence>
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
                    <span className="w-1.5 h-1.5" /> {t.foundAndReady}
                  </span>

                  {isPlaylist ? (
                    <div className="flex items-center gap-4">
                      <div className="w-16 h-16 flex items-center justify-center shrink-0">
                        <AudioLines className="size-5 text-rust" />
                      </div>
                      <div className="min-w-0">
                        <p className="text-lg font-semibold text-cream truncate">
                          {playlistName || t.playlistDefault}
                        </p>
                        <p className="text-[13px] mt-1">
                          {tracks.length > 1
                            ? t.songsParsed(tracks.length)
                            : t.songParsed(tracks.length)}
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
                      {t.btnBack}
                    </button>

                    <button
                      type="button"
                      onClick={goToResults}
                      className="flex text-xs md:text-[13px] items-center gap-2.5 p-2 text-rust bg-charcoal hover:ring-2 hover:ring-rust rounded-md uppercase tracking-wide transition-all duration-300 cursor-pointer"
                    >
                      <span>{t.btnContinueToDownload}</span>
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
                  className="text-xs bg-olive p-2 rounded-md uppercase tracking-[0.2em] transition-colors cursor-pointer"
                >
                  {t.btnBack}
                </button>
                <button
                  type="button"
                  onClick={goToSource}
                  className="text-xs p-2 bg-rust rounded-md uppercase tracking-[0.2em] transition-colors cursor-pointer"
                >
                  {t.btnStartOver}
                </button>
              </div>

              <motion.section
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                id="results-section"
                className="mt-5"
              >
                {/* Album Cover Dropzone Window */}
                <AlbumCoverDropzone
                  currentCover={activeTrack?.coverUrl}
                  isCustom={isSelectedTrackCustom}
                  onCoverChange={handleCoverChange}
                  onResetCover={handleResetCover}
                  onGenerateSpectrogram={handleGenerateSpectrogram}
                  spectrogramProgress={spectrogramProgress}
                  disabled={isBatchDownloading || isFolderDownloading}
                  trackTitle={activeTrack?.title}
                  trackArtist={activeTrack?.artist}
                  selectedTrackIndex={selectedTrackIndex}
                  totalTracks={tracks.length}
                  applyToAll={applyCoverToAll}
                  onToggleApplyToAll={setApplyCoverToAll}
                  onPrevTrack={handlePrevTrack}
                  onNextTrack={handleNextTrack}
                />

                {/* Playlist Batch header */}
                {isPlaylist && (
                  <div
                    id="batch-actions-header"
                    className="rounded-none flex flex-col md:flex-row md:items-center md:justify-between gap-4"
                  >
                    <div className="text-left md:items-start">
                      <h4 className="text uppercase tracking-[0.3em]">{t.batchOptionsTitle}</h4>
                      <p className="text-[11px] md:text-sm text-rust mt-1">
                        {t.batchOptionsDesc(tracks.length)}
                      </p>
                    </div>

                    <div className="flex flex-row items-start justify-between gap-3">
                      {/* Save directly to folder button */}
                      <button
                        type="button"
                        onClick={handleDownloadToFolder}
                        disabled={isBatchDownloading || isFolderDownloading}
                        className="px-4 py-3 border-2 border-gold rounded-md text-xs uppercase tracking-[0.15em] transition-all duration-300 flex items-center justify-center gap-2 cursor-pointer disabled:opacity-40"
                        title={t.saveToFolderTitle}
                      >
                        {isFolderDownloading ? (
                          <RefreshCw className="size-3 animate-spin" />
                        ) : (
                          <FolderPlus className="size-4 text-gold" />
                        )}
                        <span>{t.btnSaveToFolder}</span>
                      </button>

                      {/* Download as ZIP button */}
                      <button
                        type="button"
                        onClick={handleDownloadAll}
                        disabled={isBatchDownloading || isFolderDownloading}
                        className="px-4 py-3 border-2 border-rust text-xs rounded-md uppercase tracking-[0.2em] transition-colors duration-300 flex items-center justify-center gap-2 cursor-pointer disabled:opacity-40"
                        title={t.saveAsZipTitle}
                      >
                        {isBatchDownloading ? (
                          <RefreshCw className="size-3 animate-spin" />
                        ) : (
                          <Archive className="size-4 text-rust" />
                        )}
                        <span>{t.btnSaveAsZip}</span>
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
                        {t.savingDirectlyToFolder}
                      </span>
                    </div>
                    <p className="text-xs text-gold">{folderProgressText}</p>
                  </div>
                )}

                {/* Batch Download Progress Bar */}
                {isBatchDownloading && (
                  <div id="batch-progress-bar-card" className="p-5 rounded-none">
                    <div className="flex items-center justify-between text-[10px] font-mono font-bold tracking-widest text-brand uppercase mb-2">
                      <span className="flex items-center gap-1.5">
                        <RefreshCw className="w-3.5 h-3.5 text-brand animate-spin" />
                        {t.batchArchivingProgress}
                      </span>
                      <span>{batchProgress}%</span>
                    </div>
                    <div className="w-full h-2 rounded-none overflow-hidden">
                      <motion.div
                        className="bg-brand h-full"
                        initial={{ width: '0%' }}
                        animate={{ width: `${batchProgress}%` }}
                        transition={{ duration: 0.1 }}
                      />
                    </div>
                    <p className="text-[9px] mt-2 uppercase tracking-wide">
                      {t.batchArchivingSubtext}
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
                          onPlayTrack={handlePlayTrack}
                          activeTrackId={playingTrack?.id}
                          isBatchDownloading={isBatchDownloading}
                          isFolderDownloading={isFolderDownloading}
                          isSelected={index === selectedTrackIndex}
                          onSelectTrack={setSelectedTrackIndex}
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
