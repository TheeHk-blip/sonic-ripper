import { useState, useRef, useEffect } from 'react';
import { motion } from 'motion/react';
import { X, Tv, ChevronUp, ChevronDown } from 'lucide-react';
import { Track } from '../types';

interface MediaPlayerProps {
  track: Track | null;
  onClose: () => void;
}

function extractYouTubeId(url: string | null | undefined): string | null {
  if (!url) return null;
  const match = url.match(
    /(?:youtu\.be\/|youtube\.com\/(?:embed\/|v\/|watch\?v=|watch\?.+&v=))([\w-]{11})/
  );
  return match ? match[1] : null;
}

const EMBED_SOFT_TIMEOUT_MS = 15000;

type EmbedStatus = 'loading' | 'ready' | 'error' | 'slow';

function describeYouTubeError(code: number | null): string {
  switch (code) {
    case 2:
      return 'Invalid video ID.';
    case 5:
      return 'The player hit an HTML5 playback error.';
    case 100:
      return 'Video not found or private.';
    case 101:
    case 150:
      return 'The owner disabled embedding for this video.';
    case 153:
      return 'The player rejected the embed (referrer/config error).';
    default:
      return "This preview couldn't load.";
  }
}

function YouTubeEmbedFrame({
  src,
  title,
  videoId,
}: {
  src: string;
  title: string;
  videoId: string;
}) {
  const [status, setStatus] = useState<EmbedStatus>('loading');
  const [errorCode, setErrorCode] = useState<number | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [blocked, setBlocked] = useState(false);
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const workerOrigin = new URL(src).origin;

  // The parent keys this component by video id, so every video gets a fresh
  // player with fresh state. Retry just remounts the iframe for the same video.
  const handleRetry = () => {
    setStatus('loading');
    setErrorCode(null);
    setBlocked(false);
    setAttempt(n => n + 1);
  };

  // Real signals from the Worker bridge
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      if (e.origin !== workerOrigin) return;
      if (e.source !== iframeRef.current?.contentWindow) return;
      const m = e.data;
      if (!m || m.source !== 'sonic-embed') return;

      if (m.type === 'ready') {
        setStatus('ready');
      } else if (m.type === 'blocked') {
        setBlocked(true);
        setStatus('ready');
      } else if (m.type === 'error') {
        setErrorCode(typeof m.data === 'number' ? m.data : null);
        setStatus('error');
      } else if (m.type === 'state' && typeof m.data === 'number') {
        // -1 unstarted, 0 ended, 1 playing, 2 paused, 3 buffering, 5 cued
        setStatus('ready'); // any state message proves the bridge is alive
        if (m.data === 1) setBlocked(false);
      }
    };
    window.addEventListener('message', onMessage);
    return () => window.removeEventListener('message', onMessage);
  }, [workerOrigin]);

  // Soft timeout: only flags "slow", never unmounts the iframe.
  // Restarts on retry.
  useEffect(() => {
    const t = window.setTimeout(() => {
      setStatus(s => (s === 'loading' ? 'slow' : s));
    }, EMBED_SOFT_TIMEOUT_MS);
    return () => clearTimeout(t);
  }, [attempt]);

  const showOverlay = status === 'error' || status === 'slow';

  return (
    <div className="relative w-full h-full pb-4">
      <iframe
        key={attempt}
        ref={iframeRef}
        title={title}
        src={src}
        className="w-full h-full my-2.5 border-2 border-gold rounded-sm"
        allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
        allowFullScreen
        referrerPolicy="strict-origin-when-cross-origin"
      />

      {showOverlay && (
        <div
          className={`absolute inset-0 bg-brown/90 flex flex-col items-center justify-center gap-3 text-center px-6 ${
            status === 'slow' ? 'pointer-events-none' : ''
          }`}
        >
          <p className="text-xs text-white/70">
            {status === 'error'
              ? describeYouTubeError(errorCode)
              : 'Still loading. The player may just be slow.'}
          </p>
          <div className="flex gap-4 text-xs pointer-events-auto">
            <button type="button" className="text-gold underline" onClick={handleRetry}>
              Retry
            </button>
            <a
              className="text-gold underline"
              href={`https://www.youtube.com/watch?v=${videoId}`}
              target="_blank"
              rel="noreferrer"
            >
              Open on YouTube
            </a>
          </div>
        </div>
      )}

      {!showOverlay && blocked && (
        <div className="absolute bottom-2 left-0 right-0 flex justify-center pointer-events-none">
          <span className="text-[10px] tracking-widest font-mono uppercase text-gold bg-black/70 px-2 py-1">
            Tap the video to play
          </span>
        </div>
      )}
    </div>
  );
}

export default function MediaPlayer({ track, onClose }: MediaPlayerProps) {
  const [isVideoExpanded, setIsVideoExpanded] = useState(true);

  const audioRef = useRef<HTMLAudioElement | null>(null);

  const isYouTube =
    !!track?.previewUrl &&
    (track.previewUrl.includes('youtube.com') || track.previewUrl.includes('youtu.be'));

  const youtubeVideoId = isYouTube ? extractYouTubeId(track.previewUrl) : null;
  const workerEmbedSrc = youtubeVideoId
    ? `${import.meta.env.VITE_EMBED_WORKER_URL}/embed?v=${youtubeVideoId}`
    : null;

  const handleClose = () => {
    if (audioRef.current) {
      audioRef.current.pause();
      audioRef.current.src = '';
    }
    onClose();
  };

  if (!track) return null;

  return (
    <motion.div
      initial={{ y: 100, opacity: 0 }}
      animate={{ y: 0, opacity: 1 }}
      exit={{ y: 100, opacity: 0 }}
      transition={{ type: 'spring', damping: 25, stiffness: 120 }}
      className={`${isVideoExpanded ? '' : 'mt-60'} fixed bottom-0 left-0 right-0 z-10 backdrop-blur-md shadow-xl`}
    >
      <div className="flex flex-col w-full h-full gap-4 p-8">
        {/* Video frame (YouTube only) */}
        {isYouTube && workerEmbedSrc && import.meta.env.VITE_EMBED_WORKER_URL && (
          <motion.div
            initial={false}
            animate={{
              height: isVideoExpanded ? 'auto' : 0,
              opacity: isVideoExpanded ? 1 : 0,
            }}
            transition={{ duration: 0.2 }}
            className={`w-[80%] self-center aspect-video relative overflow-hidden shadow-2xl ${
              !isVideoExpanded ? 'pointer-events-none invisible h-0' : ''
            }`}
          >
            <YouTubeEmbedFrame
              key={youtubeVideoId!}
              src={workerEmbedSrc}
              title={track.title}
              videoId={youtubeVideoId!}
            />
          </motion.div>
        )}

        {/* Master row */}
        <div className="flex flex-col md:flex-row md:items-center justify-between gap-4">
          {/* Left: art + meta */}
          <div className="flex items-center gap-4 min-w-60">
            <div className="relative shrink-0 w-14 h-14 overflow-hidden">
              <img
                src={track.coverUrl}
                alt={track.title}
                referrerPolicy="no-referrer"
                className="w-full h-full object-cover"
              />
            </div>
            <div className="min-w-0 flex-1">
              <span className="text-sm font-bold block truncate transition-colors">
                {track.title}
              </span>
              <span className="text-xs text-gold block truncate italic mt-0.5">{track.artist}</span>
              <span className="text-[9px] tracking-widest font-mono uppercase mt-1 block">
                YouTube Stream
              </span>
            </div>
          </div>

          {/* Right: mode + close */}
          <div className="flex items-center justify-end gap-5">
            {isYouTube && <Tv className="size-5 text-olive" />}

            {isYouTube && (
              <button
                type="button"
                onClick={() => setIsVideoExpanded(!isVideoExpanded)}
                className="text-white/60 hover:text-brand transition-colors"
              >
                {isVideoExpanded ? (
                  <ChevronDown className="size-5 text-rust" />
                ) : (
                  <ChevronUp className="size-5 text-gold" />
                )}
              </button>
            )}

            <button
              type="button"
              onClick={handleClose}
              className="p-1 rounded-none hover:bg-white/10 hover:text-brand transition-all border border-transparent hover:border-white/10 cursor-pointer"
              title="Close Preview"
            >
              <X className="size-5" />
            </button>
          </div>
        </div>
      </div>
    </motion.div>
  );
}
