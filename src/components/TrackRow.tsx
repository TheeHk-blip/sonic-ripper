import { Track } from '../types';
import { motion } from 'motion/react';
import { Download, CheckCircle, XCircle, Clock, Play } from 'lucide-react';

interface TrackRowProps {
  track: Track;
  index: number;
  onDownloadSingle: (track: Track) => void;
  onPlayTrack: (track: Track) => void;
  activeTrackId?: string | null;
  isBatchDownloading?: boolean;
  isFolderDownloading?: boolean;
}

export default function TrackRow({
  track,
  index,
  onDownloadSingle,
  onPlayTrack,
  activeTrackId,
  isBatchDownloading,
  isFolderDownloading,
}: TrackRowProps) {
  const formatTime = (secs: number) => {
    const minutes = Math.floor(secs / 60);
    const remainingSecs = secs % 60;
    return `${minutes}:${remainingSecs < 10 ? '0' : ''}${remainingSecs}`;
  };

  const isProcessing =
    track.status === 'scraping' ||
    track.status === 'downloading' ||
    track.status === 'transcoding' ||
    track.status === 'tagging';

  const trackNumStr = String(index + 1).padStart(2, '0');

  return (
    <motion.div
      id={`track-item-${track.id}`}
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2 }}
      className="flex flex-col border-b border-olive py-1 md:py-3"
    >
      <div className="flex flex-row items-center w-full gap-3">
        {/* Track No & Album Cover */}
        <div className="flex flex-row w-20 md:w-30 gap-2 items-center mr-3">
          <span>{trackNumStr}</span>
          <div
            onClick={() => onPlayTrack(track)}
            title="Click to Preview"
            className="relative group shrink-0 size-16 md:size-20 cursor-pointer select-none"
          >
            <img
              src={track.coverUrl}
              alt={track.title}
              referrerPolicy="no-referrer"
              className="w-full h-full aspect-square object-cover rounded-sm group-hover:scale-102 transition-transform duration-300"
            />
            <div className="absolute inset-0 flex items-center justify-center transition-all duration-300">
              {activeTrackId === track.id ? (
                <div className="flex items-end gap-0.5 size-4 justify-center">
                  <div
                    className="w-0.5 bg-rust h-2.5 animate-bounce"
                    style={{ animationDelay: '0s' }}
                  />
                  <div
                    className="w-0.5 bg-rust h-4 animate-bounce"
                    style={{ animationDelay: '0.2s' }}
                  />
                  <div
                    className="w-0.5 bg-rust h-1.5 animate-bounce"
                    style={{ animationDelay: '0.4s' }}
                  />
                </div>
              ) : (
                <Play className="size-4 transition-colors" />
              )}
            </div>
          </div>
        </div>

        {/* Track Metadata */}
        <div className="flex flex-col min-w-0 justify-between md:gap-2 w-full">
          <div className="flex flex-col">
            <span className="text-[14px] md:text-lg truncate">{track.title}</span>
            <span className="text-xs md:text-sm italic truncate">{track.artist}</span>
          </div>
          <div className="flex flex-col md:flex-row w-full">
            <div className="flex gap-1 text-sm">
              <span className="truncate">{track.album}</span>
              <span>•</span>
              <span>{track.year}</span>
              <span>•</span>
              <span className="flex flex-row gap-2 ml-2 items-center text-rust">
                <Clock className="size-4" />
                {formatTime(track.duration)}
              </span>
            </div>
          </div>
        </div>

        {/* Action Button */}
        <div className="flex items-center w-20 ml-auto transition-all duration-300">
          <button
            type="button"
            onClick={() => onDownloadSingle(track)}
            disabled={isProcessing || isBatchDownloading || isFolderDownloading}
            className={`rounded-sm p-2 hover:scale-105 cursor-pointer ${
              track.status === 'completed' ? 'bg-gold' : 'bg-charcoal'
            }`}
            title="Download Song"
          >
            {track.status === 'completed' ? (
              <CheckCircle className="size-5" />
            ) : (
              <Download className="size-5 text-cream hover:text-olive" />
            )}
          </button>
        </div>
      </div>

      {/* Progress Bar */}
      {isProcessing && track.status !== 'transcoding' && (
        <div className="flex flex-col w-full px-2 my-2">
          <div className="flex items-center text-xs justify-between uppercase">
            <span>
              {track.status === 'scraping'
                ? 'SCRAPING STREAM URL'
                : track.status === 'downloading'
                  ? 'DOWNLOADING'
                  : 'INJECTING TAGS &  ARTWORK'}
            </span>
            <span>{track.progress}%</span>
          </div>
          <div className="w-full bg-cream h-1 overflow-hidden">
            <motion.div
              className="h-full bg-rust"
              initial={{ width: '0%' }}
              animate={{ width: `${track.progress}%` }}
              transition={{ duration: 0.1 }}
            />
          </div>
        </div>
      )}

      {/* Error Banner */}
      {track.error && (
        <div className="flex flex-row px-2.5 py-1 my-1.5 mx-5 rounded-sm gap-2 bg-red-400">
          <XCircle />
          <p>{track.error}</p>
        </div>
      )}
    </motion.div>
  );
}
