import { ListMusic } from 'lucide-react';

interface TrackListHeaderProps {
  trackCount: number;
  playlistName?: string;
}

export default function TrackListHeader({ trackCount, playlistName }: TrackListHeaderProps) {
  return (
    <div className="flex flex-col my-5">
      <div className="flex flex-row gap-2.5 mb-4 items-center">
        <ListMusic className="size-5 text-gold" />
        <div className="flex flex-col">
          <h3 className="text-lg">
            {playlistName ? (
              playlistName
            ) : (
              <span>{trackCount > 1 ? 'Discovered Tracks' : 'Discovered Track'}</span>
            )}
          </h3>
          <p className="text-xs">
            {trackCount} {trackCount > 1 ? 'Songs' : 'Song'} parsed successfully
          </p>
        </div>
      </div>
    </div>
  );
}
