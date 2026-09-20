import { useEffect, useState } from 'react';
import { DownloadSettings, AudioFormat, Bitrate } from '../types';
import {
  Settings,
  FileAudio,
  Disc,
  FileVideo,
  FolderOpen,
  RefreshCw,
  FolderTree,
  Folder,
  FileText,
} from 'lucide-react';
import { getSettings, pickDownloadFolder, setNamingPattern } from '../lib/api';
import { useI18n } from '../lib/i18n';

interface SettingsPanelProps {
  settings: DownloadSettings;
  onChange: (settings: DownloadSettings) => void;
}

export default function SettingsPanel({ settings, onChange }: SettingsPanelProps) {
  const { t } = useI18n();
  const [downloadFolder, setDownloadFolder] = useState<string | null>(null);
  const [folderLoading, setFolderLoading] = useState(true);
  const [folderPicking, setFolderPicking] = useState(false);

  useEffect(() => {
    getSettings()
      .then(s => setDownloadFolder(s.downloadFolder))
      .catch(() => setDownloadFolder(null))
      .finally(() => setFolderLoading(false));
  }, []);

  const handleChangeFolder = async () => {
    setFolderPicking(true);
    try {
      const result = await pickDownloadFolder();
      if (result) setDownloadFolder(result.downloadFolder);
    } finally {
      setFolderPicking(false);
    }
  };

  const handleFormatChange = (format: AudioFormat) => {
    const defaultBitrate: Bitrate = format === 'flac' || format === 'wav' ? 'lossless' : '320k';
    onChange({ format, bitrate: defaultBitrate });
  };

  const handleBitrateChange = (bitrate: Bitrate) => {
    onChange({ ...settings, bitrate });
  };

  const formats: { value: AudioFormat; label: string; ext: string }[] = [
    { value: 'flac', label: 'FLAC (Lossless)', ext: '.flac' },
    { value: 'mp3', label: 'MP3 (Standard)', ext: '.mp3' },
    { value: 'm4a', label: 'M4A (AAC Audio)', ext: '.m4a' },
    { value: 'opus', label: 'Opus (Efficient)', ext: '.opus' },
    { value: 'wav', label: 'WAV (Waveform)', ext: '.wav' },
    { value: 'mp4', label: 'MP4 Video', ext: '.mp4' },
  ];

  const bitrates: { value: Bitrate; label: string; desc: string }[] = [
    { value: '128k', label: '128 Kbps', desc: t.bitrate128Desc },
    { value: '256k', label: '256 Kbps', desc: t.bitrate256Desc },
    { value: '320k', label: '320 Kbps', desc: t.bitrate320Desc },
    { value: 'lossless', label: 'Lossless', desc: t.bitrateLosslessDesc },
  ];

  const videoQualities: {
    value: NonNullable<DownloadSettings['videoQuality']>;
    label: string;
    desc: string;
  }[] = [
    {
      value: 'best',
      label: 'Max Resolution',
      desc: t.videoBestDesc,
    },
    {
      value: '1080p',
      label: 'Full HD (1080p)',
      desc: t.video1080pDesc,
    },
    { value: '720p', label: 'Standard HD (720p)', desc: t.video720pDesc },
    {
      value: '480p',
      label: 'Standard Quality (480p)',
      desc: t.video480pDesc,
    },
    {
      value: '360p',
      label: 'Compact Quality (360p)',
      desc: t.video360pDesc,
    },
  ];

  const isLosslessOnly = settings.format === 'flac' || settings.format === 'wav';
  const isVideoFormat = settings.format === 'mp4';

  return (
    <div className="p-6 shadow-xl relative">
      <div className="flex items-center gap-3 mb-6 border-b border-olive pb-4">
        <Settings className="size-5 text-cream" />
        <h2 className="uppercase font-display">{t.exportConfigTitle}</h2>
      </div>

      <div className="flex flex-col gap-6">
        {/* Format selection */}
        <div>
          <label className="block text-xs text-olive uppercase tracking-[0.2em] mb-3 font-semibold">
            {t.outputFormatTitle}
          </label>
          <div className="grid grid-cols-2 gap-3">
            {formats.map(f => {
              const active = settings.format === f.value;
              return (
                <button
                  key={f.value}
                  id={`btn-format-${f.value}`}
                  type="button"
                  onClick={() => handleFormatChange(f.value)}
                  className={`flex flex-col items-start p-4 rounded-md text-left transition-all duration-300 cursor-pointer ${
                    active ? 'bg-olive' : 'bg-charcoal scale-98'
                  }`}
                >
                  {f.value === 'mp4' ? (
                    <FileVideo className={`w-5 h-5 mb-2 ${active ? 'text-gold' : 'text-cream'}`} />
                  ) : (
                    <FileAudio className={`w-5 h-5 mb-2 ${active ? 'text-gold' : 'text-cream'}`} />
                  )}
                  <span
                    className={`text-xs font-bold tracking-tight ${active ? 'text-gold' : 'text-cream'}`}
                  >
                    {f.label}
                  </span>
                  <span className="text-[9px] uppercase mt-1">{f.ext}</span>
                </button>
              );
            })}
          </div>
        </div>

        {/* Bitrate or Video Quality selection */}
        <div>
          <label className="block text-xs uppercase tracking-[0.2em] text-olive mb-3 font-semibold">
            {isVideoFormat ? t.videoQualityTitle : t.audioQualityTitle}
          </label>
          {isVideoFormat ? (
            <div className="space-y-3">
              {videoQualities.map(vq => {
                const active = (settings.videoQuality || 'best') === vq.value;
                return (
                  <button
                    key={vq.value}
                    id={`btn-video-quality-${vq.value}`}
                    type="button"
                    onClick={() => onChange({ ...settings, videoQuality: vq.value })}
                    className={`w-full flex items-center justify-between p-3.5 rounded-md border-2 border-gold text-left transition-all duration-300 cursor-pointer ${
                      active ? '' : 'scale-98 opacity-70 hover:opacity-100'
                    }`}
                  >
                    <div>
                      <span
                        className={`font-semibold uppercase tracking-wider block ${active ? 'text-gold' : ''} `}
                      >
                        {vq.label}
                      </span>
                      <span className="text-xs text-cream mt-0.5 block">{vq.desc}</span>
                    </div>
                    <div
                      className={`w-4 h-4 rounded-full flex items-center justify-center ${
                        active ? 'bg-gold' : ''
                      }`}
                    >
                      {active && <div className="w-1.5 h-1.5 rounded-full" />}
                    </div>
                  </button>
                );
              })}
            </div>
          ) : isLosslessOnly ? (
            <div className="rounded p-5 h-40 flex flex-col justify-center items-center text-center text-olive">
              <Disc className="w-8 h-8 mb-3" />
              <p className="text-sm uppercase mb-1">{t.losslessLocked}</p>
              <p className="text-xs max-w-60 leading-relaxed">
                {settings.format === 'flac' ? 'FLAC' : 'WAV'} {t.losslessLockedDesc}
              </p>
            </div>
          ) : (
            <div className="space-y-3">
              {bitrates
                .filter(b => b.value !== 'lossless')
                .map(b => {
                  const active = settings.bitrate === b.value;
                  return (
                    <button
                      key={b.value}
                      id={`btn-bitrate-${b.value}`}
                      type="button"
                      onClick={() => handleBitrateChange(b.value)}
                      className={`w-full flex items-center justify-between p-3.5 rounded-md border-2 text-left transition-all duration-300 cursor-pointer ${
                        active
                          ? 'border-gold scale-100'
                          : 'border-rust opacity-70 hover:opacity-100 scale-98'
                      }`}
                    >
                      <div>
                        <span
                          className={`uppercase tracking-wider block ${active ? 'text-gold' : ''} `}
                        >
                          {b.label}
                        </span>
                        <span className="text-xs mt-0.5 block">{b.desc}</span>
                      </div>
                      <div
                        className={`w-4 h-4 rounded-full flex items-center justify-center ${
                          active ? 'bg-gold' : 'border'
                        }`}
                      >
                        {active && <div className="w-1.5 h-1.5 bg-black rounded-full" />}
                      </div>
                    </button>
                  );
                })}
            </div>
          )}

          {isLosslessOnly && (
            <div className="mt-4">
              <label className="block text-xs uppercase tracking-[0.3em] text-cream mb-2 font-semibold">
                {t.sampleRateTitle}
              </label>
              <div className="grid grid-cols-2 gap-3">
                {[
                  { value: '44100', label: '44.1 kHz', desc: t.cdQuality },
                  { value: '48000', label: '48.0 kHz', desc: t.studioQuality },
                ].map(sr => {
                  const active = (settings.sampleRate || '44100') === sr.value;
                  return (
                    <button
                      key={sr.value}
                      type="button"
                      id={`btn-samplerate-${sr.value}`}
                      onClick={() =>
                        onChange({ ...settings, sampleRate: sr.value as '44100' | '48000' })
                      }
                      className={`flex flex-col items-start p-3 rounded-md border-2 text-left transition-all duration-300 cursor-pointer ${
                        active ? 'border-gold' : 'scale-98'
                      }`}
                    >
                      <span className="text-xs font-bold tracking-tight">{sr.label}</span>
                      <span className="text-xs uppercase text-cream mt-1">{sr.desc}</span>
                    </button>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Batch download & Folder Organization settings */}
      <div className="border-t border-olive pt-6 mt-6">
        <label className="block text-sm uppercase tracking-[0.2em] mb-3 font-semibold">
          {t.taggingOrgTitle}
        </label>

        {/* Naming Pattern / Download Path Hierarchy */}
        <div className="mb-6">
          <div className="flex items-center justify-between mb-1">
            <span className="text-xs font-black uppercase tracking-wider text-cream flex items-center gap-1.5">
              <FolderTree className="w-3.5 h-3.5 text-gold" />
              {t.pathPatternTitle}
            </span>
          </div>
          <span className="block text-xs text-rust leading-relaxed font-mono mb-3">
            {t.pathPatternDesc}
          </span>

          {/* Quick Presets (Exactly 4 presets) */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 mb-3">
            {[
              {
                id: '{artist}/{year} - {album}/{trackNumber} - {title}',
                aliasIds: ['artist_year_album_track_title'],
                label: t.presetArtistYearAlbumTrack,
                badge: t.recommendedBadge,
                example: 'Daft Punk/2001 - Discovery/01 - One More Time',
              },
              {
                id: '{artist}/{album}/{trackNumber} - {title}',
                aliasIds: ['artist_album_track_title'],
                label: t.presetArtistAlbumTrack,
                badge: undefined,
                example: 'Daft Punk/Discovery/01 - One More Time',
              },
              {
                id: '{trackNumber} - {artist} - {title}',
                aliasIds: ['number_artist_title'],
                label: t.presetNumberArtistTitle,
                badge: t.flatBadge,
                example: '01 - Daft Punk - One More Time',
              },
              {
                id: '{artist} - {title}',
                aliasIds: ['artist_title'],
                label: t.presetArtistTitle,
                badge: t.flatBadge,
                example: 'Daft Punk - One More Time',
              },
            ].map(preset => {
              const current = settings.namingPattern || 'number_artist_title';
              const isSelected = current === preset.id || preset.aliasIds.includes(current);

              return (
                <button
                  key={preset.id}
                  type="button"
                  id={`btn-naming-preset-${preset.aliasIds[0] || 'custom'}`}
                  onClick={() => {
                    onChange({
                      ...settings,
                      namingPattern: preset.id,
                    });
                    setNamingPattern(preset.id).catch(() => {});
                  }}
                  className={`p-2.5 text-left border-2 rounded-md transition-all flex flex-col justify-between cursor-pointer ${
                    isSelected
                      ? 'border-gold bg-gold/10'
                      : 'border-rust/40 scale-98 opacity-70 hover:opacity-100 hover:border-rust'
                  }`}
                >
                  <div className="flex items-center justify-between gap-1">
                    <span
                      className={`text-xs font-bold tracking-tight truncate ${
                        isSelected ? 'text-gold' : 'text-cream'
                      }`}
                    >
                      {preset.label}
                    </span>
                    {preset.badge && (
                      <span className="text-[9px] px-1.5 py-0.2 bg-olive/40 text-cream rounded border border-olive/50 shrink-0">
                        {preset.badge}
                      </span>
                    )}
                  </div>
                  <span className="text-[9px] text-cream/60 mt-1 truncate font-mono">
                    {preset.example}
                  </span>
                </button>
              );
            })}
          </div>

          {/* Custom Pattern Input */}
          <div className="bg-charcoal/60 border border-rust/30 rounded-md p-3 mb-3">
            <label className="block text-[11px] font-bold uppercase tracking-wider text-cream mb-1">
              {t.editCustomPattern}
            </label>
            <input
              type="text"
              id="input-naming-pattern"
              value={settings.namingPattern || ''}
              placeholder="{artist}/{year} - {album}/{trackNumber} - {title}"
              onChange={e => {
                const val = e.target.value;
                onChange({
                  ...settings,
                  namingPattern: val,
                });
                setNamingPattern(val).catch(() => {});
              }}
              className="w-full px-3 py-2 bg-charcoal border border-rust/40 focus:border-gold rounded text-xs font-mono text-cream focus:outline-none transition-all placeholder:text-cream/30"
            />

            {/* Quick Variable Insertion Chips */}
            <div className="flex flex-wrap items-center gap-1.5 mt-2">
              <span className="text-[10px] text-rust uppercase font-semibold mr-1">
                {t.insertLabel}
              </span>
              {[
                { tag: '{artist}', label: t.chipArtist },
                { tag: '{year}', label: t.chipYear },
                { tag: '{album}', label: t.chipAlbum },
                { tag: '{trackNumber}', label: t.chipTrackNumber },
                { tag: '{title}', label: t.chipTitle },
                { tag: '{totalTracks}', label: t.chipTotalTracks },
                { tag: '{playlist}', label: t.chipPlaylist },
                { tag: '/', label: t.chipSlash },
              ].map(item => (
                <button
                  key={item.tag}
                  type="button"
                  onClick={() => {
                    const current = settings.namingPattern || '';
                    const needsSep =
                      item.tag !== '/' &&
                      current.length > 0 &&
                      !current.endsWith('/') &&
                      !current.endsWith(' ') &&
                      !current.endsWith('-');
                    const nextVal = current + (needsSep ? ' ' : '') + item.tag;
                    onChange({ ...settings, namingPattern: nextVal });
                    setNamingPattern(nextVal).catch(() => {});
                  }}
                  className="px-2 py-0.8 bg-charcoal hover:bg-olive/40 border border-rust/40 hover:border-gold rounded text-[10px] font-mono text-cream/90 transition-all cursor-pointer"
                >
                  {item.label}
                </button>
              ))}
            </div>
          </div>

          {/* Live Path Preview */}
          <div className="p-3 bg-charcoal/80 border border-olive/50 rounded-md">
            <div className="flex items-center gap-1.5 mb-1.5">
              <span className="text-[10px] font-bold uppercase tracking-wider text-gold">
                {t.livePreviewTitle}
              </span>
            </div>
            {(() => {
              let template =
                settings.namingPattern || '{artist}/{year} - {album}/{trackNumber} - {title}';
              if (template === 'number_artist_title') {
                template = '{trackNumber} - {artist} - {title}';
              } else if (template === 'artist_title') {
                template = '{artist} - {title}';
              } else if (template === 'artist_year_album_track_title') {
                template = '{artist}/{year} - {album}/{trackNumber} - {title}';
              } else if (template === 'artist_album_track_title') {
                template = '{artist}/{album}/{trackNumber} - {title}';
              }

              const ext = settings.format === 'mp4' ? 'mp4' : settings.format;
              const replaced = template
                .replace(
                  /{artist}|{artista}|{nombre Artista}|{nombre_artista}|{nombreArtista}/g,
                  'Daft Punk'
                )
                .replace(/{album}|{nombre album}|{nombre_album}|{nombreAlbum}/g, 'Discovery')
                .replace(
                  /{year}|{año}|{ano}|{albumYear}|{album_year}|{año del album}|{ano del album}/g,
                  '2001'
                )
                .replace(
                  /{trackNumber}|{track_number}|{track}|{pista}|{numero de pista}|{numero}/g,
                  '01'
                )
                .replace(/{totalTracks}|{total_tracks}|{total pistas}/g, '14')
                .replace(/{title}|{titulo}|{titulo de la pista}|{nombre pista}/g, 'One More Time')
                .replace(
                  /{playlist}|{playlistName}|{playlist_name}|{lista}/g,
                  'Best of Electronic'
                );

              const segments = replaced
                .replace(/^[/\\]+/, '')
                .split(/[/\\]+/)
                .map(s => s.trim().replace(/^[-_\s]+|[-_\s]+$/g, ''))
                .filter(Boolean);

              const fileName =
                segments.length > 0
                  ? `${segments[segments.length - 1]}.${ext}`
                  : `Daft Punk - One More Time.${ext}`;
              const folders = segments.length > 1 ? segments.slice(0, -1) : [];
              const baseFolder = downloadFolder
                ? downloadFolder.replace(/[/\\]+$/, '')
                : `[${t.downloadFolderTitle}]`;
              const fullPath = [baseFolder, ...folders, fileName].join('/');

              return (
                <div className="space-y-1.5">
                  <div className="flex flex-wrap items-center gap-1 font-mono text-[11px]">
                    <span className="px-1.5 py-0.5 bg-olive/30 text-cream/70 rounded flex items-center gap-1">
                      <FolderOpen className="w-3 h-3 text-gold/70" />
                      {baseFolder.split('/').pop() || t.downloadFolderTitle}
                    </span>
                    {folders.map((f, i) => (
                      <span key={i} className="flex items-center gap-1">
                        <span className="text-rust/60">/</span>
                        <span className="px-1.5 py-0.5 bg-olive/20 text-cream rounded flex items-center gap-1 border border-olive/30">
                          <Folder className="w-3 h-3 text-gold" />
                          {f}
                        </span>
                      </span>
                    ))}
                    <span className="text-rust/60">/</span>
                    <span className="px-1.5 py-0.5 bg-charcoal text-gold font-bold rounded border border-gold/40 flex items-center gap-1">
                      <FileAudio className="w-3 h-3 text-gold" />
                      {fileName}
                    </span>
                  </div>
                  <div className="text-[10px] text-cream/50 font-mono break-all select-all">
                    {fullPath}
                  </div>
                </div>
              );
            })()}
          </div>
        </div>

        <div className="space-y-4">
          {/* Embed ID3 Tags Toggle */}
          <div className="flex items-start justify-between gap-4">
            <div className="flex flex-col gap-1">
              <span className="text-xs font-black uppercase tracking-wider text-cream">
                {t.embedId3Title}
              </span>
              <span className="text-[10px] md:text-xs text-rust leading-relaxed">
                {t.embedId3Desc}
              </span>
            </div>
            <button
              type="button"
              id="btn-toggle-id3-tags"
              onClick={() =>
                onChange({
                  ...settings,
                  embedId3Tags: settings.embedId3Tags !== false ? false : true,
                })
              }
              className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
                settings.embedId3Tags !== false ? 'bg-olive' : 'bg-cream'
              }`}
            >
              <span
                className={`pointer-events-none inline-block h-4 w-4 bg-black transform rounded-full shadow ring-0 transition duration-200 ease-in-out ${
                  settings.embedId3Tags !== false ? 'translate-x-4' : 'translate-x-0'
                }`}
              />
            </button>
          </div>

          {/*Download Folder*/}
          <div className="flex items-start justify-between gap-4" id="download-folder-wrapper">
            <div className="flex flex-col gap-1 min-w-0">
              <span className="text-xs font-black uppercase tracking-wider text-cream">
                {t.downloadFolderTitle}
              </span>
              <span className="text-[10px] md:text-sm text-rust leading-relaxed truncate">
                {folderLoading ? t.loadingFolder : downloadFolder || t.noFolderSet}
              </span>
            </div>
            <button
              type="button"
              id="btn-change-download-folder"
              onClick={handleChangeFolder}
              disabled={folderPicking}
              className="shrink-0 px-3 py-2 bg-gold/40 rounded-sm text-[10px] font-black uppercase tracking-wider transition-all duration-200 flex items-center gap-2 cursor-pointer disabled:opacity-40"
            >
              {folderPicking ? (
                <RefreshCw className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <FolderOpen className="w-3.5 h-3.5" />
              )}
              {downloadFolder ? t.btnChange : t.btnChoose}
            </button>
          </div>

          {/* Download Lyrics (.txt) Toggle */}
          <div className="flex items-start justify-between gap-4" id="download-lyrics-wrapper">
            <div className="flex flex-col gap-1">
              <span className="text-xs font-black uppercase tracking-wider text-cream flex items-center gap-1.5">
                <FileText className="w-3.5 h-3.5 text-gold" />
                {t.downloadLyricsTitle}
                <span className="text-[9px] px-1.5 py-0.2 bg-olive/40 text-cream rounded border border-olive/50 shrink-0 font-mono">
                  .TXT
                </span>
              </span>
              <span className="text-[10px] md:text-xs text-rust leading-relaxed">
                {t.downloadLyricsDesc}
              </span>
            </div>
            <button
              type="button"
              id="btn-toggle-lyrics"
              onClick={() =>
                onChange({
                  ...settings,
                  downloadLyrics: settings.downloadLyrics !== false ? false : true,
                })
              }
              className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
                settings.downloadLyrics !== false ? 'bg-olive' : 'bg-cream'
              }`}
            >
              <span
                className={`pointer-events-none inline-block h-4 w-4 bg-black transform rounded-full shadow ring-0 transition duration-200 ease-in-out ${
                  settings.downloadLyrics !== false ? 'translate-x-4' : 'translate-x-0'
                }`}
              />
            </button>
          </div>

          {/* Skip Missing Tracks Toggle */}
          <div className="flex items-start justify-between gap-4">
            <div className="flex flex-col gap-1">
              <span className="text-xs font-black uppercase tracking-wider text-cream">
                {t.skipMissingTitle}
              </span>
              <span className="text-[10px] md:text-sm text-rust leading-relaxed">
                {t.skipMissingDesc}
              </span>
            </div>
            <button
              type="button"
              id="btn-toggle-skip-missing"
              onClick={() =>
                onChange({ ...settings, skipMissingTracks: !settings.skipMissingTracks })
              }
              className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer border-2 border-transparent rounded-full transition-colors duration-300 ease-in-out focus:outline-none ${
                settings.skipMissingTracks ? 'bg-olive' : 'bg-cream'
              }`}
            >
              <span
                className={`pointer-events-none inline-block h-4 w-4 transform bg-black rounded-full shadow ring-0 transition duration-200 ease-in-out ${
                  settings.skipMissingTracks ? 'translate-x-4' : 'translate-x-0'
                }`}
              />
            </button>
          </div>
        </div>
      </div>

      {/* Bypass Bot Detection Section */}
      <div className="mt-6 pt-6">
        <label className="block text-sm uppercase tracking-[0.2em] text-gold mb-2 font-semibold">
          {t.bypassBotTitle}
        </label>

        <div className="flex flex-col gap-4">
          {/* Browser Profile extraction option */}
          <div>
            <p className="text-[9px] md:text-xs text-rust mb-3 leading-relaxed">
              {t.bypassBotDesc}
            </p>
            <select
              id="browser-cookies-select"
              value={settings.cookiesFromBrowser || ''}
              onChange={e => {
                const val = e.target.value;
                onChange({ ...settings, cookiesFromBrowser: val || undefined });
              }}
              className="w-full p-3 text-[10px] text-charcoal transition-all cursor-pointer"
            >
              <option value="">{t.noneBrowserOption}</option>
              <option value="chrome">Google Chrome</option>
              <option value="firefox">Mozilla Firefox</option>
              <option value="safari">Apple Safari</option>
              <option value="edge">Microsoft Edge</option>
              <option value="brave">Brave Browser</option>
              <option value="opera">Opera</option>
              <option value="vivaldi">Vivaldi</option>
            </select>
          </div>

          {/* Manual paste fallback */}
          {!settings.cookiesFromBrowser && (
            <div>
              <span className="block text-xs uppercase tracking-[0.2em] mb-2 font-semibold">
                {t.orPasteCookiesTitle}
              </span>
              <p className="text-xs mb-3 leading-relaxed">{t.orPasteCookiesDesc}</p>
              <textarea
                id="cookies-text-input"
                value={settings.youtubeCookies || ''}
                onChange={e => onChange({ ...settings, youtubeCookies: e.target.value })}
                placeholder={`# Netscape HTTP Cookie File .youtube.com	TRUE	/	TRUE	1791234567	__Secure-3PSID	AIzaSy...`}
                rows={5}
                className="w-full p-3 bg-charcoal border-2 border-rust/30 rounded-md text-[10px] placeholder-cream/30 focus:outline-none focus:border-olive/40 transition-all"
              />
            </div>
          )}

          {settings.cookiesFromBrowser && (
            <div className="p-4 rounded-none flex flex-col justify-center">
              <span className="block text-xs uppercase tracking-wide text-gold mb-1 font-bold">
                {t.browserExtractMode} {settings.cookiesFromBrowser.toUpperCase()}
              </span>
              <p className="text-xs text-cream/50 leading-relaxed">{t.browserExtractDesc}</p>
              <p className="text-xs text-gold/80 leading-relaxed mt-2">
                <strong>Tip:</strong> {t.browserExtractTip}
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
