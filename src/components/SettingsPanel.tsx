import { useEffect, useState } from 'react';
import { DownloadSettings, AudioFormat, Bitrate } from '../types';
import { Settings, FileAudio, Disc, FileVideo, FolderOpen, RefreshCw } from 'lucide-react';
import { getSettings, pickDownloadFolder } from '../lib/api';

interface SettingsPanelProps {
  settings: DownloadSettings;
  onChange: (settings: DownloadSettings) => void;
}

export default function SettingsPanel({ settings, onChange }: SettingsPanelProps) {
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
    { value: '128k', label: '128 Kbps', desc: 'Standard quality, smaller file' },
    { value: '256k', label: '256 Kbps', desc: 'High quality, balanced file size' },
    { value: '320k', label: '320 Kbps', desc: 'Extreme quality, best for MP3/M4A' },
    { value: 'lossless', label: 'Lossless', desc: 'Original studio master quality' },
  ];

  const videoQualities: {
    value: NonNullable<DownloadSettings['videoQuality']>;
    label: string;
    desc: string;
  }[] = [
    {
      value: 'best',
      label: 'Max Resolution',
      desc: 'Highest available video feed with premium audio',
    },
    {
      value: '1080p',
      label: 'Full HD (1080p)',
      desc: '1920x1080 resolution high-definition video stream',
    },
    { value: '720p', label: 'Standard HD (720p)', desc: '1280x720 standard HD video stream' },
    {
      value: '480p',
      label: 'Standard Quality (480p)',
      desc: '854x480 standard definition video stream',
    },
    {
      value: '360p',
      label: 'Compact Quality (360p)',
      desc: '640x360 compact video (optimized for bandwidth)',
    },
  ];

  const isLosslessOnly = settings.format === 'flac' || settings.format === 'wav';
  const isVideoFormat = settings.format === 'mp4';

  return (
    <div className="p-6 shadow-xl relative">
      <div className="flex items-center gap-3 mb-6 border-b border-olive pb-4">
        <Settings className="size-5 text-cream" />
        <h2 className="uppercase font-display">02 / Export Configuration</h2>
      </div>

      <div className="flex flex-col gap-6">
        {/* Format selection */}
        <div>
          <label className="block text-xs text-olive uppercase tracking-[0.2em] mb-3 font-semibold">
            Output Format
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
            {isVideoFormat ? 'Video Quality / Resolution' : 'Audio Quality (Bitrate)'}
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
              <p className="text-sm uppercase mb-1">Lossless Locked</p>
              <p className="text-xs max-w-60 leading-relaxed">
                {settings.format === 'flac' ? 'FLAC' : 'WAV'} is inherently lossless and ignores
                bitrate compressions.
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
                Lossless Frequency (Sample Rate)
              </label>
              <div className="grid grid-cols-2 gap-3">
                {[
                  { value: '44100', label: '44.1 kHz', desc: 'CD Quality' },
                  { value: '48000', label: '48.0 kHz', desc: 'Studio Quality' },
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
          Tagging & Organization
        </label>

        {/* Naming Pattern */}
        <div className="mb-5">
          <span className="block text-xs font-black uppercase tracking-wider text-cream mb-1">
            File Naming Pattern
          </span>
          <span className="block text-xs text-rust leading-relaxed font-mono mb-3">
            Choose how output filenames are structured when saved or zipped.
          </span>
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
            {(
              [
                {
                  id: 'artist_title',
                  label: 'Artist - Title',
                  example: 'Daft Punk - One More Time.mp3',
                },
                {
                  id: 'number_artist_title',
                  label: '01 - Artist - Title',
                  example: '01 - Daft Punk - One More Time.mp3',
                },
                { id: 'title', label: 'Title Only', example: 'One More Time.mp3' },
              ] satisfies {
                id: NonNullable<DownloadSettings['namingPattern']>;
                label: string;
                example: string;
              }[]
            ).map(pattern => (
              <button
                key={pattern.id}
                type="button"
                id={`btn-naming-${pattern.id}`}
                onClick={() =>
                  onChange({
                    ...settings,
                    namingPattern: pattern.id,
                  })
                }
                className={`p-3 text-left border-2 rounded-md transition-all flex flex-col justify-between ${
                  (settings.namingPattern || 'artist_title') === pattern.id
                    ? 'border-rust'
                    : 'scale-98 opacity-70 hover:opacity-100'
                }`}
              >
                <span
                  className={`text-xs font-bold tracking-tight ${settings.namingPattern === pattern.id ? 'text-rust' : ''} `}
                >
                  {pattern.label}
                </span>
                <span className="text-[9px] text-cream/60 mt-1 truncate">{pattern.example}</span>
              </button>
            ))}
          </div>
        </div>

        <div className="space-y-4">
          {/* Embed ID3 Tags Toggle */}
          <div className="flex items-start justify-between gap-4">
            <div className="flex flex-col gap-1">
              <span className="text-xs font-black uppercase tracking-wider text-cream">
                Embed ID3 Tags & Artwork
              </span>
              <span className="text-[10px] md:text-xs text-rust leading-relaxed">
                Embeds Title, Artist, Album, Year, Track Number, and High-Res Artwork directly into
                the audio file headers.
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
                Download Folder
              </span>
              <span className="text-[10px] md:text-sm text-rust leading-relaxed truncate">
                {folderLoading
                  ? 'Loading…'
                  : downloadFolder ||
                    'No folder set — downloads will be blocked until you choose one.'}
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
              {downloadFolder ? 'Change' : 'Choose'}
            </button>
          </div>

          {/* Skip Missing Tracks Toggle */}
          <div className="flex items-start justify-between gap-4">
            <div className="flex flex-col gap-1">
              <span className="text-xs font-black uppercase tracking-wider text-cream">
                Skip Missing Tracks
              </span>
              <span className="text-[10px] md:text-sm text-rust leading-relaxed">
                Automatically skip tracks that are unavailable or fail to download instead of
                aborting the batch.
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
          Bypass YouTube Bot Block
        </label>

        <div className="flex flex-col gap-4">
          {/* Browser Profile extraction option */}
          <div>
            <p className="text-[9px] md:text-xs text-rust mb-3 leading-relaxed">
              Extract session cookies automatically from your active browser profile.
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
              <option value="">-- None (Use Manual Paste) --</option>
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
                Or Paste Netscape Cookies
              </span>
              <p className="text-xs mb-3 leading-relaxed">
                Manually paste your Netscape cookies file text if browser extraction is not
                available on your current device setup.
              </p>
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
                Browser Extract Mode: {settings.cookiesFromBrowser.toUpperCase()}
              </span>
              <p className="text-xs text-cream/50 leading-relaxed">
                The download engine will dynamically parse cookies from your active{' '}
                {settings.cookiesFromBrowser} profile on request.
              </p>
              <p className="text-xs text-gold/80 leading-relaxed mt-2">
                <strong>Tip:</strong> if downloads intermittently fail to read cookies, close{' '}
                {settings.cookiesFromBrowser} first — it locks its cookie database while running,
                which can block extraction.
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
