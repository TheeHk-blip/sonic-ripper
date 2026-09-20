import { useState, useRef, useEffect, DragEvent, ChangeEvent } from 'react';
import {
  Image,
  ImagePlus,
  RotateCcw,
  Download,
  Check,
  RefreshCw,
  ChevronLeft,
  ChevronRight,
  ChevronDown,
  ChevronUp,
  Activity,
  Search,
  X,
  ExternalLink,
  Globe,
} from 'lucide-react';
import { useI18n } from '../lib/i18n';
import { saveCoverImage, searchOnlineCovers, openExternalUrl, OnlineCoverResult } from '../lib/api';

export interface SpectrogramPalette {
  id: string;
  name: string;
  gradient: string;
  accent: string;
}

export const SPECTROGRAM_PALETTES: SpectrogramPalette[] = [
  {
    id: 'magma',
    name: 'Magma',
    gradient: 'linear-gradient(90deg, #000004, #51127c, #b73779, #fb8861, #fcfdbf)',
    accent: '#fb8861',
  },
  {
    id: 'fire',
    name: 'Fire',
    gradient: 'linear-gradient(90deg, #000000, #800000, #ff0000, #ff8000, #ffff00)',
    accent: '#ff8000',
  },
  {
    id: 'plasma',
    name: 'Plasma',
    gradient: 'linear-gradient(90deg, #0d0887, #6a00a8, #b12a90, #e16462, #fca636, #f0f921)',
    accent: '#e16462',
  },
  {
    id: 'viridis',
    name: 'Viridis',
    gradient: 'linear-gradient(90deg, #440154, #3b528b, #21918c, #5ec962, #fde725)',
    accent: '#21918c',
  },
  {
    id: 'rainbow',
    name: 'Rainbow',
    gradient:
      'linear-gradient(90deg, #ff0000, #ff8000, #ffff00, #00ff00, #00ffff, #0000ff, #8000ff)',
    accent: '#00ffaa',
  },
  {
    id: 'nebulae',
    name: 'Nebulae',
    gradient: 'linear-gradient(90deg, #0a081e, #3d1c7a, #852d91, #c8457d, #ff719a)',
    accent: '#c8457d',
  },
  {
    id: 'cool',
    name: 'Cool',
    gradient: 'linear-gradient(90deg, #00ffff, #0080ff, #8000ff, #ff00ff)',
    accent: '#00ffff',
  },
  {
    id: 'green',
    name: 'Green Matrix',
    gradient: 'linear-gradient(90deg, #001100, #004400, #00aa00, #00ff33, #aaffbb)',
    accent: '#00ff33',
  },
  {
    id: 'cividis',
    name: 'Cividis',
    gradient: 'linear-gradient(90deg, #00204d, #414d6b, #7c7b78, #bdaf5e, #ffea46)',
    accent: '#ffea46',
  },
  {
    id: 'fruit',
    name: 'Fruit',
    gradient: 'linear-gradient(90deg, #1a2a00, #4d8000, #ffbb00, #ff5500, #ff0077)',
    accent: '#ff5500',
  },
  {
    id: 'fiery',
    name: 'Fiery',
    gradient: 'linear-gradient(90deg, #000000, #400000, #d02000, #ff8000, #ffff40)',
    accent: '#ff8000',
  },
  {
    id: 'moreland',
    name: 'Moreland',
    gradient: 'linear-gradient(90deg, #3b4cc0, #8cb2e9, #dddddd, #f49a7b, #b40426)',
    accent: '#8cb2e9',
  },
  {
    id: 'terrain',
    name: 'Terrain',
    gradient: 'linear-gradient(90deg, #336699, #339966, #ffcc66, #996633, #ffffff)',
    accent: '#339966',
  },
  {
    id: 'intensity',
    name: 'Intensity',
    gradient: 'linear-gradient(90deg, #000000, #444444, #888888, #cccccc, #ffffff)',
    accent: '#ffffff',
  },
  {
    id: 'channel',
    name: 'Stereo Channels',
    gradient: 'linear-gradient(90deg, #ff3333, #ffff33, #33ff33, #33ffff, #3333ff)',
    accent: '#33ffff',
  },
];

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
  onGenerateSpectrogram?: (palette: string) => Promise<void>;
  spectrogramProgress?: { current: number; total: number } | null;
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
  applyToAll = false,
  onToggleApplyToAll,
  onPrevTrack,
  onNextTrack,
  onGenerateSpectrogram,
  spectrogramProgress,
}: AlbumCoverDropzoneProps) {
  const { t } = useI18n();
  const [isCollapsed, setIsCollapsed] = useState(true);
  const [spectrogramPalette, setSpectrogramPalette] = useState('magma');
  const [isPaletteOpen, setIsPaletteOpen] = useState(false);
  const paletteDropdownRef = useRef<HTMLDivElement>(null);

  const currentPaletteObj =
    SPECTROGRAM_PALETTES.find(p => p.id === spectrogramPalette) || SPECTROGRAM_PALETTES[0];

  useEffect(() => {
    if (!isPaletteOpen) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (paletteDropdownRef.current && !paletteDropdownRef.current.contains(e.target as Node)) {
        setIsPaletteOpen(false);
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [isPaletteOpen]);

  const [isDragging, setIsDragging] = useState(false);
  const [isDownloading, setIsDownloading] = useState(false);
  const [downloadSuccess, setDownloadSuccess] = useState(false);
  const [isGeneratingSpectrogram, setIsGeneratingSpectrogram] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Online Cover Search state
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [searchResults, setSearchResults] = useState<OnlineCoverResult[]>([]);
  const [isSearching, setIsSearching] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [hasSearched, setHasSearched] = useState(false);
  const [appliedCoverUrl, setAppliedCoverUrl] = useState<string | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);

  const executeSearch = async (queryText?: string) => {
    const q = (queryText !== undefined ? queryText : searchQuery).trim();
    if (!q) return;

    setIsSearching(true);
    setSearchError(null);
    setHasSearched(true);

    try {
      const results = await searchOnlineCovers(q);
      setSearchResults(results.slice(0, 12));
    } catch (err) {
      console.error('Failed to search online covers:', err);
      setSearchError(String(err));
    } finally {
      setIsSearching(false);
    }
  };

  const [isSearchingWeb, setIsSearchingWeb] = useState(false);

  const handleSearchWeb = async (e: React.MouseEvent) => {
    e.stopPropagation();
    const q = searchQuery.trim() || [trackArtist, trackTitle].filter(Boolean).join(' ');
    if (!q) return;

    setIsSearchingWeb(true);
    setSearchError(null);

    try {
      const webResults = await searchOnlineCovers(q, 'web');
      setSearchResults(webResults.slice(0, 12));
    } catch (err) {
      console.error('Failed to search web covers:', err);
      setSearchError(String(err));
    } finally {
      setIsSearchingWeb(false);
    }
  };

  const handleToggleSearch = (e: React.MouseEvent) => {
    e.stopPropagation();
    const nextState = !isSearchOpen;
    setIsSearchOpen(nextState);

    if (nextState) {
      // Reverted to artist + title as before
      const defaultQuery = [trackArtist, trackTitle].filter(Boolean).join(' ');
      if (!searchQuery.trim() && defaultQuery) {
        setSearchQuery(defaultQuery);
        executeSearch(defaultQuery);
      } else if (searchQuery.trim() && searchResults.length === 0) {
        executeSearch(searchQuery);
      }
      setTimeout(() => {
        searchInputRef.current?.focus();
      }, 100);
    }
  };

  const handleSearchGoogle = (e: React.MouseEvent) => {
    e.stopPropagation();
    const term = searchQuery.trim() || [trackArtist, trackTitle].filter(Boolean).join(' ');
    const googleUrl = `https://www.google.com/search?tbm=isch&q=${encodeURIComponent(
      term ? `${term} album cover` : 'album cover'
    )}`;
    openExternalUrl(googleUrl);
  };

  const handleSelectCover = (e: React.MouseEvent, cover: OnlineCoverResult) => {
    e.stopPropagation();
    onCoverChange(cover.coverUrl);
    setAppliedCoverUrl(cover.coverUrl);
    setTimeout(() => {
      setAppliedCoverUrl(null);
    }, 2500);
  };

  const handleSpectrogramClick = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!onGenerateSpectrogram || isGeneratingSpectrogram || disabled) return;
    setIsGeneratingSpectrogram(true);
    try {
      await onGenerateSpectrogram(spectrogramPalette);
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

  if (isCollapsed) {
    return (
      <div
        onClick={() => setIsCollapsed(false)}
        className="w-full mb-3 bg-charcoal/80 hover:bg-charcoal/95 border border-olive/50 hover:border-gold/60 rounded-xl px-3.5 py-2.5 flex items-center justify-between gap-3 cursor-pointer transition-all duration-200 shadow-sm group select-none"
      >
        <div className="flex items-center gap-3 min-w-0 flex-1">
          {currentCover ? (
            <img
              src={currentCover}
              alt=""
              className="w-9 h-9 rounded object-cover border border-olive/50 shrink-0 shadow-sm group-hover:scale-105 transition-transform"
            />
          ) : (
            <div className="w-9 h-9 rounded bg-charcoal flex items-center justify-center border border-olive/40 shrink-0">
              <Image className="w-4 h-4 text-cream/40" />
            </div>
          )}

          <div className="min-w-0 flex flex-col">
            <div className="flex items-center gap-2">
              <span className="text-xs font-bold text-cream truncate uppercase tracking-wider">
                {trackTitle || t.coverDropzoneTitle}
              </span>
              {isCustom && (
                <span className="text-[9px] px-1.5 py-0.2 rounded bg-gold/20 text-gold border border-gold/40 uppercase tracking-widest font-bold shrink-0">
                  {t.coverDropzoneCustomBadge}
                </span>
              )}
            </div>
            <div className="flex items-center gap-2 text-[11px] text-cream/60 truncate">
              {trackArtist && <span>{trackArtist}</span>}
              {hasMultipleTracks && (
                <span className="text-olive/80 font-mono">
                  • {t.coverDropzoneTrackIndicator(selectedTrackIndex + 1, totalTracks)}
                </span>
              )}
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2 shrink-0">
          <div className="flex items-center gap-1.5 text-xs text-gold/90 group-hover:text-gold font-semibold uppercase tracking-wider bg-olive/20 group-hover:bg-olive/40 px-2.5 py-1.5 rounded-md border border-olive/30 transition-all">
            <span>{t.coverDropzoneExpand}</span>
            <ChevronDown className="w-4 h-4 text-gold group-hover:translate-y-0.5 transition-transform" />
          </div>
        </div>
      </div>
    );
  }

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
          {/* Header Row: Title, Track Navigator, Badges & Minimize button */}
          <div className="flex items-center justify-between gap-2 mb-1.5 flex-wrap">
            <div className="flex items-center gap-2 flex-wrap">
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

            <button
              type="button"
              onClick={e => {
                e.stopPropagation();
                setIsCollapsed(true);
              }}
              className="flex items-center gap-1 px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wider text-cream/70 hover:text-gold bg-charcoal/80 hover:bg-charcoal border border-olive/40 rounded transition-colors cursor-pointer"
              title={t.coverDropzoneMinimize}
            >
              <span>{t.coverDropzoneMinimize}</span>
              <ChevronUp className="w-3.5 h-3.5" />
            </button>
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

            {/* Round Online Search Button with Magnifying Glass */}
            <button
              type="button"
              onClick={handleToggleSearch}
              className={`w-7 h-7 sm:w-8 sm:h-8 rounded-full flex items-center justify-center border transition-all cursor-pointer shrink-0 shadow-sm ${
                isSearchOpen
                  ? 'bg-gold text-charcoal border-gold shadow-gold/30 scale-105'
                  : 'bg-charcoal/90 hover:bg-olive/40 text-gold border-gold/50 hover:border-gold hover:scale-105'
              }`}
              title={t.coverSearchOnlineTitle}
              aria-label={t.coverSearchOnlineTitle}
            >
              <Search className="w-3.5 h-3.5 sm:w-4 sm:h-4 stroke-[2.5]" />
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

            {/* Generate & embed audio spectrogram button with custom palette selector */}
            {onGenerateSpectrogram && (
              <div
                onClick={e => e.stopPropagation()}
                className="flex items-center rounded-md border border-olive/50 bg-charcoal/80 focus-within:border-gold hover:border-rust transition-colors"
              >
                <button
                  type="button"
                  onClick={handleSpectrogramClick}
                  disabled={disabled || isGeneratingSpectrogram}
                  className="flex items-center gap-1.5 px-3 py-1.5 text-[11px] font-semibold uppercase tracking-wider text-cream/90 hover:bg-rust/20 border-r border-olive/40 transition-all cursor-pointer disabled:opacity-50"
                  title={t.coverDropzoneSpectrogramTitle}
                >
                  {isGeneratingSpectrogram ? (
                    <RefreshCw className="w-3.5 h-3.5 animate-spin text-rust" />
                  ) : (
                    <Activity className="w-3.5 h-3.5 text-rust" />
                  )}
                  <span>
                    {isGeneratingSpectrogram
                      ? spectrogramProgress
                        ? `${t.coverDropzoneGeneratingSpectrogram} (${spectrogramProgress.current}/${spectrogramProgress.total})`
                        : t.coverDropzoneGeneratingSpectrogram
                      : t.coverDropzoneSpectrogramBtn}
                  </span>
                </button>

                {/* Custom Color Palette Dropdown */}
                <div ref={paletteDropdownRef} className="relative">
                  <button
                    type="button"
                    onClick={e => {
                      e.stopPropagation();
                      if (!disabled && !isGeneratingSpectrogram) {
                        setIsPaletteOpen(!isPaletteOpen);
                      }
                    }}
                    disabled={disabled || isGeneratingSpectrogram}
                    className="flex items-center gap-2 px-2.5 py-1.5 text-[11px] font-mono uppercase tracking-wider text-cream hover:text-gold bg-charcoal/90 transition-colors cursor-pointer disabled:opacity-50"
                    title={t.coverDropzonePaletteLabel}
                  >
                    {/* Colored preview swatch of current selection */}
                    <span
                      className="w-4 h-3 rounded-xs border border-white/30 shadow-xs shrink-0"
                      style={{ background: currentPaletteObj.gradient }}
                    />
                    <span className="font-semibold truncate max-w-[85px]">
                      {currentPaletteObj.name}
                    </span>
                    <ChevronDown
                      className={`w-3 h-3 text-cream/70 transition-transform ${isPaletteOpen ? 'rotate-180 text-gold' : ''}`}
                    />
                  </button>

                  {/* Custom dropdown popover */}
                  {isPaletteOpen && (
                    <div
                      onClick={e => e.stopPropagation()}
                      className="absolute left-0 sm:left-auto sm:right-0 bottom-full mb-1 sm:bottom-auto sm:top-full sm:mt-1 w-52 bg-charcoal border border-olive/60 rounded-lg shadow-2xl p-1.5 z-50 max-h-60 overflow-y-auto"
                      style={{
                        scrollbarWidth: 'thin',
                        scrollbarColor: 'var(--color-gold) transparent',
                      }}
                    >
                      {SPECTROGRAM_PALETTES.map(p => {
                        const isSelected = p.id === spectrogramPalette;
                        return (
                          <button
                            key={p.id}
                            type="button"
                            onClick={() => {
                              setSpectrogramPalette(p.id);
                              setIsPaletteOpen(false);
                            }}
                            className={`w-full flex items-center justify-between gap-2 px-2 py-1.5 rounded text-left text-xs transition-colors cursor-pointer ${
                              isSelected
                                ? 'bg-olive/40 text-gold font-semibold'
                                : 'text-cream/80 hover:bg-olive/20 hover:text-cream'
                            }`}
                          >
                            <div className="flex items-center gap-2.5 min-w-0">
                              <span
                                className="w-5 h-3 rounded-xs border border-white/20 shadow-xs shrink-0"
                                style={{ background: p.gradient }}
                              />
                              <span className="truncate">{p.name}</span>
                            </div>
                            {isSelected && <Check className="w-3.5 h-3.5 text-gold shrink-0" />}
                          </button>
                        );
                      })}
                    </div>
                  )}
                </div>
              </div>
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

          {/* Online Cover Search Drawer */}
          {isSearchOpen && (
            <div
              onClick={e => e.stopPropagation()}
              className="mt-3 p-3 rounded-lg bg-black/40 border border-gold/40 shadow-inner flex flex-col gap-2.5 transition-all duration-200 text-left"
            >
              {/* Search Bar */}
              <div className="flex items-center gap-2">
                <div className="relative flex-1 flex items-center min-w-0">
                  <Search className="w-3.5 h-3.5 text-cream/40 absolute left-2.5 pointer-events-none" />
                  <input
                    ref={searchInputRef}
                    type="text"
                    value={searchQuery}
                    onChange={e => setSearchQuery(e.target.value)}
                    onKeyDown={e => {
                      if (e.key === 'Enter') {
                        e.preventDefault();
                        executeSearch();
                      }
                    }}
                    placeholder={t.coverSearchPlaceholder}
                    className="w-full pl-8 pr-7 py-1.5 text-xs rounded bg-charcoal border border-olive/50 text-cream placeholder-cream/40 focus:outline-none focus:border-gold transition-colors"
                  />
                  {searchQuery && (
                    <button
                      type="button"
                      onClick={() => {
                        setSearchQuery('');
                        searchInputRef.current?.focus();
                      }}
                      className="absolute right-2 text-cream/40 hover:text-cream transition-colors p-0.5 cursor-pointer"
                    >
                      <X className="w-3.5 h-3.5" />
                    </button>
                  )}
                </div>

                <button
                  type="button"
                  onClick={() => executeSearch()}
                  disabled={isSearching || !searchQuery.trim()}
                  className="flex items-center gap-1 px-2.5 py-1.5 text-[11px] font-semibold uppercase tracking-wider rounded bg-olive/70 hover:bg-olive text-cream border border-olive transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed shrink-0"
                >
                  {isSearching ? (
                    <RefreshCw className="w-3.5 h-3.5 animate-spin text-gold" />
                  ) : (
                    <Search className="w-3.5 h-3.5 text-gold" />
                  )}
                  <span>{isSearching ? t.coverSearching : t.coverSearchBtn}</span>
                </button>

                <button
                  type="button"
                  onClick={() => setIsSearchOpen(false)}
                  className="p-1 rounded hover:bg-charcoal text-cream/50 hover:text-rust transition-colors cursor-pointer shrink-0"
                  title={t.coverSearchClose}
                >
                  <X className="w-4 h-4" />
                </button>
              </div>

              {/* Status / Results */}
              {isSearching ? (
                <div className="py-6 flex flex-col items-center justify-center gap-2 text-gold">
                  <RefreshCw className="w-5 h-5 animate-spin text-gold" />
                  <span className="text-[11px] font-mono tracking-wider">{t.coverSearching}</span>
                </div>
              ) : searchError ? (
                <div className="py-2.5 px-3 rounded bg-rust/15 border border-rust/30 text-rust text-xs">
                  {searchError}
                </div>
              ) : searchResults.length > 0 ? (
                <div className="flex flex-col gap-1.5">
                  <div className="flex items-center justify-between text-[10px] uppercase font-mono tracking-wider text-cream/60 px-0.5">
                    <span>{t.coverSearchSelectPrompt}</span>
                    <span className="text-gold/80">
                      {Math.min(searchResults.length, 12)} resultados
                    </span>
                  </div>

                  {/* Grid of Results (Compact Cards) */}
                  <div
                    className="grid grid-cols-3 sm:grid-cols-4 md:grid-cols-6 gap-2 max-h-56 overflow-y-auto p-1"
                    style={{
                      scrollbarWidth: 'thin',
                      scrollbarColor: 'var(--color-gold) transparent',
                    }}
                  >
                    {searchResults.slice(0, 12).map(result => {
                      const isSelected =
                        currentCover === result.coverUrl || appliedCoverUrl === result.coverUrl;
                      return (
                        <div
                          key={result.id}
                          onClick={e => handleSelectCover(e, result)}
                          className={`group relative rounded-md overflow-hidden border cursor-pointer transition-all duration-200 hover:scale-[1.03] bg-black/50 flex flex-col ${
                            isSelected
                              ? 'border-gold ring-2 ring-gold/60 shadow-md shadow-gold/20'
                              : 'border-olive/40 hover:border-gold/80'
                          }`}
                        >
                          <div className="aspect-square w-full relative overflow-hidden bg-black/20">
                            <img
                              src={result.thumbnailUrl}
                              alt={result.album || result.title}
                              referrerPolicy="no-referrer"
                              loading="lazy"
                              className="w-full h-full object-cover transition-transform duration-200 group-hover:scale-105"
                            />

                            {/* Resolution badge */}
                            <span className="absolute bottom-1 right-1 px-1 py-0.2 rounded text-[8px] font-mono font-bold bg-black/75 text-cream/90 border border-white/20">
                              HD
                            </span>

                            {/* Source badge */}
                            <span className="absolute top-1 left-1 px-1 py-0.2 rounded text-[8px] font-semibold uppercase bg-charcoal/85 text-gold/90 border border-gold/30">
                              {result.source}
                            </span>

                            {/* Selected overlay */}
                            {isSelected && (
                              <div className="absolute inset-0 bg-gold/30 backdrop-blur-[1px] flex items-center justify-center">
                                <div className="w-5 h-5 rounded-full bg-gold text-charcoal flex items-center justify-center shadow-lg">
                                  <Check className="w-3.5 h-3.5 stroke-[3]" />
                                </div>
                              </div>
                            )}
                          </div>

                          <div className="p-1 text-[10px] leading-tight flex flex-col min-w-0 bg-charcoal/90">
                            <span
                              className="font-semibold text-cream truncate"
                              title={result.album || result.title}
                            >
                              {result.album || result.title}
                            </span>
                            <span className="text-cream/60 truncate" title={result.artist}>
                              {result.artist}
                            </span>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </div>
              ) : hasSearched ? (
                <div className="py-4 text-center text-xs text-cream/50 italic">
                  {t.coverSearchNoResults}
                </div>
              ) : null}

              {/* Buscar más en la Web (integrado directamente dentro de la app) */}
              {hasSearched && !isSearching && (
                <div className="flex flex-col items-center gap-2 pt-2.5 pb-0.5 border-t border-olive/30 mt-1">
                  <div className="flex items-center gap-2 flex-wrap justify-center">
                    <button
                      type="button"
                      onClick={handleSearchWeb}
                      disabled={isSearchingWeb}
                      className="flex items-center gap-1.5 px-3.5 py-1.5 text-[11px] font-semibold uppercase tracking-wider rounded-md bg-charcoal/95 hover:bg-olive/40 text-gold border border-gold/50 hover:border-gold transition-all cursor-pointer shadow-sm disabled:opacity-50"
                      title={t.coverSearchWebBtn}
                    >
                      {isSearchingWeb ? (
                        <RefreshCw className="w-3.5 h-3.5 animate-spin text-gold" />
                      ) : (
                        <Globe className="w-3.5 h-3.5 text-gold" />
                      )}
                      <span>{isSearchingWeb ? t.coverSearchingWeb : t.coverSearchWebBtn}</span>
                    </button>

                    <button
                      type="button"
                      onClick={handleSearchGoogle}
                      className="flex items-center gap-1 text-[10px] text-cream/50 hover:text-gold transition-colors py-1 px-2 rounded hover:bg-charcoal cursor-pointer"
                      title={t.coverSearchOpenBrowser}
                    >
                      <span>{t.coverSearchOpenBrowser}</span>
                      <ExternalLink className="w-3 h-3" />
                    </button>
                  </div>

                  <span className="text-[10px] text-cream/50 italic text-center">
                    {t.coverSearchGoogleTip}
                  </span>
                </div>
              )}
            </div>
          )}

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

                <span className="text-xs font-semibold uppercase tracking-wider text-cream">
                  {t.coverDropzoneApplyToAllLabel}
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
