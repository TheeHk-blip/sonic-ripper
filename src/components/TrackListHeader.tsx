import { ListMusic } from 'lucide-react';
import { useI18n } from '../lib/i18n';

interface TrackListHeaderProps {
  trackCount: number;
  playlistName?: string;
}

export default function TrackListHeader({ trackCount, playlistName }: TrackListHeaderProps) {
  const { t } = useI18n();

  return (
    <div className="flex flex-col my-5">
      <div className="flex flex-row gap-2.5 mb-4 items-center">
        <ListMusic className="size-5 text-gold" />
        <div className="flex flex-col">
          <h3 className="text-lg">
            {playlistName ? (
              playlistName
            ) : (
              <span>{trackCount > 1 ? t.discoveredTracks : t.discoveredTrack}</span>
            )}
          </h3>
          <p className="text-xs">
            {trackCount > 1 ? t.songsParsed(trackCount) : t.songParsed(trackCount)}
          </p>
        </div>
      </div>
    </div>
  );
}
