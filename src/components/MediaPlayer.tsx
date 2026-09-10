import { useState, useRef } from 'react';
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

export default function MediaPlayer({ track, onClose }: MediaPlayerProps) {
  const [isVideoExpanded, setIsVideoExpanded] = useState(true);

  const audioRef = useRef<HTMLAudioElement | null>(null);

  const isYouTube =
    !!track?.previewUrl &&
    (track.previewUrl.includes('youtube.com') || track.previewUrl.includes('youtu.be'));

  const youtubeVideoId = isYouTube ? extractYouTubeId(track?.previewUrl) : null;
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
      className={`${isVideoExpanded ? '' : 'mt-60'} fixed bottom-0 left-0 right-0 z-10  bg-brown /95 backdrop-blur-md shadow-2xl p-4 sm:p-6`}
    >
      <div className="max-w-7xl mx-auto flex flex-col gap-4">
        {/* Video frame (YouTube only) */}
        {isYouTube && workerEmbedSrc && import.meta.env.VITE_EMBED_WORKER_URL && (
          <motion.div
            initial={false}
            animate={{
              height: isVideoExpanded ? 'auto' : 0,
              opacity: isVideoExpanded ? 1 : 0,
            }}
            transition={{ duration: 0.2 }}
            className={`w-full max-w-2xl mx-auto aspect-video relative overflow-hidden shadow-2xl ${
              !isVideoExpanded ? 'pointer-events-none invisible h-0' : ''
            }`}
          >
            <iframe
              title={track.title}
              src={workerEmbedSrc}
              className="w-full h-full border-0"
              allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share; microphone; camera; speaker-selection"
              allowFullScreen
              referrerPolicy="strict-origin-when-cross-origin"
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
