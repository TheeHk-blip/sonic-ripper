import { useState, useRef, DragEvent, ChangeEvent } from 'react';
import {
  Image,
  ImagePlus,
  RotateCcw,
  Download,
  Check,
  RefreshCw,
  ChevronLeft,
  ChevronRight,
  Activity,
} from 'lucide-react';
import { useI18n } from '../lib/i18n';
import { saveCoverImage } from '../lib/api';

interface AlbumCoverDropzoneProps {
  currentCover?: string;
  isCustom: boolean;
  onCoverChange: (newCoverUrl: string) => void;
  onResetCover: () => void;
  disabled?: boolean;
  trackTitle?: string;
  trackArtist?: string;
  selectedTrackIndex?: number;
  totalTracks?: number;
  applyToAll?: boolean;
  onToggleApplyToAll?: (applyToAll: boolean) => void;
  onPrevTrack?: () => void;
  onNextTrack?: () => void;
  onGenerateSpectrogram?: () => Promise<void>;
}

export default function AlbumCoverDropzone({
  currentCover,
  isCustom,
  onCoverChange,
  onResetCover,
  disabled = false,
  trackTitle,
  trackArtist,
  selectedTrackIndex = 0,
  totalTracks = 1,
  applyToAll = true,
  onToggleApplyToAll,
  onPrevTrack,
  onNextTrack,
  onGenerateSpectrogram,
}: AlbumCoverDropzoneProps) {
  const { t } = useI18n();
  const [isDragging, setIsDragging] = useState(false);
  const [isDownloading, setIsDownloading] = useState(false);
  const [downloadSuccess, setDownloadSuccess] = useState(false);
  const [isGeneratingSpectrogram, setIsGeneratingSpectrogram] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const handleSpectrogramClick = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!onGenerateSpectrogram || isGeneratingSpectrogram || disabled) return;
    setIsGeneratingSpectrogram(true);
    try {
      await onGenerateSpectrogram();
    } finally {
      setIsGeneratingSpectrogram(false);
    }
  };

  const handleDownloadImage = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!currentCover || isDownloading) return;

    setIsDownloading(true);
    try {
      const saved = await saveCoverImage(currentCover, 'cover.jpg');
      if (saved) {
        setDownloadSuccess(true);
        setTimeout(() => setDownloadSuccess(false), 2200);
      }
    } catch (err) {
      console.error('Error downloading cover image:', err);
    } finally {
      setIsDownloading(false);
    }
  };

  const processFile = (file: File) => {
    if (!file.type.startsWith('image/')) return;
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === 'string') {
        onCoverChange(reader.result);
      }
    };
    reader.readAsDataURL(file);
  };

  const handleDragOver = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    if (disabled) return;
    setIsDragging(true);
  };

  const handleDragLeave = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragging(false);
  };

  const handleDrop = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragging(false);
    if (disabled) return;

    if (e.dataTransfer.files && e.dataTransfer.files.length > 0) {
      const file = e.dataTransfer.files[0];
      processFile(file);
      return;
    }

    // Direct image drag from another browser window / webpage
    const uri =
      e.dataTransfer.getData('text/uri-list') ||
      e.dataTransfer.getData('text/plain') ||
      e.dataTransfer.getData('URL');

    if (
      uri &&
      (uri.startsWith('http://') || uri.startsWith('https://') || uri.startsWith('data:image/'))
    ) {
      onCoverChange(uri.trim());
    }
  };

  const handleFileInputChange = (e: ChangeEvent<HTMLInputElement>) => {
    if (e.target.files && e.target.files.length > 0) {
      const file = e.target.files[0];
      processFile(file);
      // Reset input value so re-selecting same file triggers change
      e.target.value = '';
    }
  };

  const handleContainerClick = () => {
    if (disabled) return;
    fileInputRef.current?.click();
  };

  const hasMultipleTracks = totalTracks > 1;

  return (
    <div
      onClick={handleContainerClick}
      onDragOver={handleDragOver}
      onDragEnter={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
      className={`relative w-full rounded-xl border-2 transition-all duration-300 p-3.5 md:p-4 mb-4 cursor-pointer select-none ${
        isDragging
          ? 'border-gold bg-gold/15 scale-[1.01] shadow-lg shadow-gold/20'
          : isCustom
            ? 'border-gold/60 bg-charcoal/80 hover:border-gold'
            : 'border-dashed border-olive/60 bg-charcoal/50 hover:border-olive hover:bg-charcoal/70'
      } ${disabled ? 'opacity-50 pointer-events-none' : ''}`}
    >
      <input
        ref={fileInputRef}
        type="file"
        accept="image/png,image/jpeg,image/webp,image/gif,image/avif"
        className="hidden"
        onChange={handleFileInputChange}
      />

      <div className="flex flex-col sm:flex-row items-center gap-4">
        {/* Cover Preview */}
        <div className="relative shrink-0 w-24 h-24 sm:w-28 sm:h-28 rounded-lg overflow-hidden border border-olive/50 bg-black/40 shadow-inner flex items-center justify-center group">
          {currentCover ? (
            <img
              src={currentCover}
              alt="Cover Art"
              referrerPolicy="no-referrer"
              className="w-full h-full object-cover transition-transform duration-300 group-hover:scale-105"
            />
          ) : (
            <Image className="w-10 h-10 text-olive animate-pulse" />
          )}

          {/* Overlay hover cue */}
          <div className="absolute inset-0 bg-black/50 opacity-0 group-hover:opacity-100 transition-opacity flex flex-col items-center justify-center text-[10px] text-cream uppercase font-semibold">
            <ImagePlus className="w-5 h-5 mb-1 text-gold" />
            <span>{t.coverDropzoneChangeBtn}</span>
          </div>
        </div>

        {/* Info & Call-to-action */}
        <div className="flex-1 min-w-0 text-center sm:text-left">
          {/* Header Row: Title, Track Navigator & Badges */}
          <div className="flex items-center justify-center sm:justify-start gap-2 mb-1.5 flex-wrap">
            <span className="text-xs font-semibold uppercase tracking-[0.2em] text-cream">
              {t.coverDropzoneTitle}
            </span>

            {/* Track Selector Navigator (if multiple tracks) */}
            {hasMultipleTracks && (
              <div
                onClick={e => e.stopPropagation()}
                className="flex items-center gap-1 bg-charcoal px-2 py-0.5 rounded-full border border-olive/40"
              >
                <button
                  type="button"
                  onClick={onPrevTrack}
                  title="Pista anterior"
                  className="hover:text-gold text-cream/70 transition-colors p-0.5 cursor-pointer"
                >
                  <ChevronLeft className="w-3.5 h-3.5" />
                </button>
                <span className="text-[11px] font-mono text-cream font-medium px-1">
                  {t.coverDropzoneTrackIndicator(selectedTrackIndex + 1, totalTracks)}
                </span>
                <button
                  type="button"
                  onClick={onNextTrack}
                  title="Siguiente pista"
                  className="hover:text-gold text-cream/70 transition-colors p-0.5 cursor-pointer"
                >
                  <ChevronRight className="w-3.5 h-3.5" />
                </button>
              </div>
            )}

            {/* Custom vs Default Badge */}
            {isCustom ? (
              <span className="text-[10px] uppercase font-bold px-2 py-0.5 rounded-full bg-gold/20 text-gold border border-gold/40">
                {t.coverDropzoneCustomBadge}
              </span>
            ) : (
              <span className="text-[10px] uppercase font-medium px-2 py-0.5 rounded-full bg-olive/30 text-cream/70 border border-olive/40">
                {t.coverDropzoneDefaultBadge}
              </span>
            )}
          </div>

          {/* Active track title & artist */}
          {trackTitle && (
            <p className="text-xs sm:text-[13px] text-gold font-medium truncate mb-0.5">
              <span className="opacity-70 font-mono">
                #{String(selectedTrackIndex + 1).padStart(2, '0')}
              </span>{' '}
              {trackTitle}{' '}
              {trackArtist ? <span className="text-cream/70 italic">• {trackArtist}</span> : ''}
            </p>
          )}

          <p className="text-xs sm:text-[13px] text-cream/90 font-medium">
            {t.coverDropzonePrompt}
          </p>

          <p className="text-[11px] text-rust/90 mt-0.5">
            {isCustom ? t.coverDropzonePrompt : t.coverDropzoneDefaultHint}
          </p>

          {/* Action buttons */}
          <div className="flex items-center justify-center sm:justify-start gap-2.5 mt-2.5 flex-wrap">
            <button
              type="button"
              onClick={e => {
                e.stopPropagation();
                fileInputRef.current?.click();
              }}
              className="flex items-center gap-1.5 px-3 py-1.5 text-[11px] font-semibold uppercase tracking-wider rounded-md bg-olive/50 hover:bg-olive text-cream border border-olive transition-colors cursor-pointer"
            >
              <ImagePlus className="w-3.5 h-3.5 text-gold" />
              <span>{t.coverDropzoneChangeBtn}</span>
            </button>

            {/* Download cover button */}
            {currentCover && (
              <button
                type="button"
                onClick={handleDownloadImage}
                disabled={isDownloading}
                className="flex items-center gap-1.5 px-3 py-1.5 text-[11px] font-semibold uppercase tracking-wider rounded-md bg-charcoal/80 hover:bg-olive/40 text-gold border border-gold/40 hover:border-gold transition-colors cursor-pointer disabled:opacity-50"
                title="Descargar cover.jpg"
              >
                {isDownloading ? (
                  <RefreshCw className="w-3.5 h-3.5 animate-spin text-gold" />
                ) : downloadSuccess ? (
                  <Check className="w-3.5 h-3.5 text-cream" />
                ) : (
                  <Download className="w-3.5 h-3.5 text-gold" />
                )}
                <span>{t.coverDropzoneDownloadBtn}</span>
              </button>
            )}

            {/* Generate & embed audio spectrogram button (discreet) */}
            {onGenerateSpectrogram && (
              <button
                type="button"
                onClick={handleSpectrogramClick}
                disabled={disabled || isGeneratingSpectrogram}
                className="flex items-center gap-1.5 px-3 py-1.5 text-[11px] font-semibold uppercase tracking-wider rounded-md bg-charcoal/80 hover:bg-rust/20 text-cream/90 border border-olive/50 hover:border-rust transition-all cursor-pointer disabled:opacity-50"
                title={t.coverDropzoneSpectrogramTitle}
              >
                {isGeneratingSpectrogram ? (
                  <RefreshCw className="w-3.5 h-3.5 animate-spin text-rust" />
                ) : (
                  <Activity className="w-3.5 h-3.5 text-rust" />
                )}
                <span>
                  {isGeneratingSpectrogram
                    ? t.coverDropzoneGeneratingSpectrogram
                    : t.coverDropzoneSpectrogramBtn}
                </span>
              </button>
            )}

            {isCustom && (
              <button
                type="button"
                onClick={e => {
                  e.stopPropagation();
                  onResetCover();
                }}
                className="flex items-center gap-1.5 px-3 py-1.5 text-[11px] font-semibold uppercase tracking-wider rounded-md bg-charcoal hover:bg-rust/20 text-rust border border-rust/50 transition-colors cursor-pointer"
              >
                <RotateCcw className="w-3.5 h-3.5" />
                <span>{t.coverDropzoneResetBtn}</span>
              </button>
            )}
          </div>

          {/* On/Off Switch: Apply to entire series vs. only selected track */}
          {hasMultipleTracks && onToggleApplyToAll && (
            <div
              onClick={e => e.stopPropagation()}
              className="flex flex-wrap items-center gap-3 pt-2.5 border-t border-olive/30 mt-3"
            >
              <div
                onClick={() => onToggleApplyToAll(!applyToAll)}
                className="flex items-center gap-2 cursor-pointer select-none group"
              >
                <button
                  type="button"
                  role="switch"
                  aria-checked={applyToAll}
                  onClick={e => {
                    e.stopPropagation();
                    onToggleApplyToAll(!applyToAll);
                  }}
                  className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
                    applyToAll ? 'bg-gold' : 'bg-charcoal border-olive/60'
                  }`}
                >
                  <span
                    className={`pointer-events-none inline-block h-4 w-4 transform rounded-full shadow ring-0 transition duration-200 ease-in-out ${
                      applyToAll ? 'translate-x-4 bg-charcoal' : 'translate-x-0 bg-olive'
                    }`}
                  />
                </button>

                <span className="text-xs font-semibold uppercase tracking-wider text-cream flex items-center gap-1.5">
                  {t.coverDropzoneApplyToAllLabel}
                  <span
                    className={`text-[9px] font-bold px-1.5 py-0.5 rounded uppercase tracking-wider ${
                      applyToAll
                        ? 'bg-gold/30 text-gold border border-gold/40'
                        : 'bg-charcoal text-cream/50 border border-olive/40'
                    }`}
                  >
                    {applyToAll ? 'ON' : 'OFF'}
                  </span>
                </span>
              </div>

              <span className="text-[11px] text-cream/70 italic">
                {applyToAll ? t.coverDropzoneApplyToAllHintOn : t.coverDropzoneApplyToAllHintOff}
              </span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
