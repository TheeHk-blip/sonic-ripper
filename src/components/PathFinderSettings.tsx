import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

function useSavedField(initial: string) {
  const [value, setValue] = useState(initial);
  const [saved, setSaved] = useState(initial);
  const dirty = value !== saved;
  return { value, setValue, saved, setSaved, dirty };
}

export function SpotifyPathfinderSettings() {
  const clientToken = useSavedField('');
  const playlistHash = useSavedField('');
  const albumHash = useSavedField('');
  const [status, setStatus] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    (async () => {
      const [currentToken, currentPlaylistHash, currentAlbumHash] = await Promise.all([
        invoke<string | null>('get_spotify_client_token_cmd'),
        invoke<string>('get_pathfinder_hash_cmd'),
        invoke<string>('get_album_hash_cmd'),
      ]);
      clientToken.setValue(currentToken ?? '');
      clientToken.setSaved(currentToken ?? '');
      playlistHash.setValue(currentPlaylistHash);
      playlistHash.setSaved(currentPlaylistHash);
      albumHash.setValue(currentAlbumHash);
      albumHash.setSaved(currentAlbumHash);
      setLoaded(true);
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function applyClientToken() {
    await invoke('set_spotify_client_token_cmd', { token: clientToken.value });
    clientToken.setSaved(clientToken.value);
    setStatus('Client token updated.');
  }

  async function applyPlaylistHash() {
    await invoke('set_pathfinder_hash_cmd', { hash: playlistHash.value });
    const effective = await invoke<string>('get_pathfinder_hash_cmd');
    playlistHash.setValue(effective);
    playlistHash.setSaved(effective);
    setStatus(
      playlistHash.value.trim() ? 'Playlist hash override set.' : 'Playlist hash reset to default.'
    );
  }

  async function applyAlbumHash() {
    await invoke('set_album_hash_cmd', { hash: albumHash.value });
    const effective = await invoke<string>('get_album_hash_cmd');
    albumHash.setValue(effective);
    albumHash.setSaved(effective);
    setStatus(albumHash.value.trim() ? 'Album hash override set.' : 'Album hash reset to default.');
  }

  if (!loaded) return <p>Loading current settings…</p>;

  return (
    <div className="flex flex-col w-full my-3 gap-2.5">
      <div className="my-5">
        <h1>NOTE</h1>
        <p>
          By default this app pulls metadata from Spotify&apos;s public <em>embed</em> page.
          It&apos;s convenient, but it&apos;s also thin. For a playlist larger than 100 tracks,
          resolving every track this way means one HTTP fetch per track beyond the first 100, a
          500-track playlist is ~400 extra requests.
        </p>
        <br />
        <p>
          <em>Pathfinder</em> is the private GraphQL API the Spotify web player itself uses to load
          these pages. Querying it directly gets you the same structured data the web player
          renders; full album name and artwork, an exact release date, and for playlists/albums the
          entire tracklist in a couple of paginated calls instead of one request per track. A single
          benefits the same way an album does, since Spotify treats a single as a one-track album
          under the hood, which is also why capturing its hash below works a little differently (see
          Step 1).
        </p>
        <br />
        <p>
          It&apos;s a private, undocumented API with no official access grant, so using it means
          capturing your own session token and query hashes from the web player&apos;s network
          traffic instead of an API key, and it&apos;s entirely optional. If you choose to skip it,
          the app just uses the embed fetch logic.
        </p>
        <br />
        <div className="flex flex-col mx-2.5 gap-3">
          <div>
            <h3>STEP 1</h3>
            <p>Open Spotify web and Dev Tools, then click a playlist or album.</p>
            <p>
              Navigate to the Network tab and filter with the keyword{' '}
              <span className="text-gold font-semibold">pathfinder</span>
            </p>
            <p>
              Click through the queries and check the payload tab for one that lists operationName
              as <span className="text-gold font-semibold">fetchPlaylist</span> or{' '}
              <span className="text-gold font-semibold">getAlbum</span> as shown below.
            </p>
            <p>
              These are two separate queries, each with its own hash:{' '}
              <span className="text-gold font-semibold">fetchPlaylist</span>&apos;s hash only helps
              with playlists, and <span className="text-gold font-semibold">getAlbum</span>&apos;s
              helps with albums <em>and</em> singles. Fill in whichever field(s) match what you want
              covered — playlists, albums/singles, or both. To fill in both, you&apos;ll need to
              repeat steps 1–3 twice: once while a fetchPlaylist request is selected, once while a
              getAlbum request is selected.
            </p>
            <p>
              For a single, no request will fire unless you open the single&apos;s own page —
              clicking play on it from a playlist, artist discography, or search results won&apos;t
              trigger a getAlbum request. Since Spotify treats a single as a one-track album,
              navigate to that release&apos;s own page first (the same way you&apos;d open an
              album), and the getAlbum request will show up there.
            </p>
            <img src="/operationName.png" alt="Operation Name" width={650} height={400} />
          </div>
          <div>
            <h3>STEP 2</h3>
            <p>
              In the same payload tab, click the extensions dropdown to get the sha256Hash for the
              request you selected, as shown below
            </p>
            <img src="/sha256Hash.png" alt="sha256Hash" width={650} height={400} />
          </div>
          <div>
            <h3>STEP 3</h3>
            <p>
              Navigate to the Headers tab of the same request, and look for the client token as
              shown below
            </p>
            <img src="/clientToken.png" alt="Client Token" width={650} height={400} />
          </div>
          <p>
            Enter the client token once — it&apos;s shared by both queries. Enter each sha256Hash
            into the field labeled for the operation it came from (fetchPlaylist → Playlist Query
            Hash, getAlbum → Album Query Hash). That&apos;s it. Hashes rotate after a while but the
            client token is only available when you have an active session. If at one point
            resolution takes longer or metadata is not complete, re-capture whichever hash matches
            what stopped working.
          </p>
        </div>
      </div>
      <div className="flex flex-row items-center justify-between gap-5 w-full">
        <div className="flex flex-col flex-1 border-2 gap-2.5 border-olive rounded-sm px-4 py-3">
          <label htmlFor="client-token">Spotify Client Token</label>
          <input
            id="client-token"
            type="password"
            value={clientToken.value}
            onChange={e => clientToken.setValue(e.target.value)}
            placeholder="AAE5n90/psAco..."
            className="py-2 px-3 bg-charcoal rounded-sm"
          />
        </div>
        {clientToken.dirty ? (
          <button
            onClick={applyClientToken}
            className="px-3 py-2.5 w-28.75 font-medium bg-olive/60 hover:bg-olive rounded-sm active:scale-98 transition-all duration-300 flex items-center justify-center gap-2 cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed"
          >
            Save token
          </button>
        ) : (
          <div className="px-3 py-2.5 w-28.75 font-medium bg-olive/40 rounded-sm flex items-center justify-center gap-2 cursor-not-allowed">
            Saved
          </div>
        )}
      </div>

      <div className="flex flex-row items-center justify-between gap-5 w-full">
        <div className="flex flex-col flex-1 border-2 gap-2.5 border-olive rounded-sm px-4 py-3">
          <label htmlFor="playlist-query-hash">Playlist Query Hash (fetchPlaylist)</label>
          <input
            id="playlist-query-hash"
            type="text"
            value={playlistHash.value}
            onChange={e => playlistHash.setValue(e.target.value)}
            className="py-2 px-3 bg-charcoal rounded-sm"
          />
        </div>
        {playlistHash.dirty ? (
          <button
            onClick={applyPlaylistHash}
            className="px-3 py-2.5 w-28.75 font-medium bg-olive/60 hover:bg-olive rounded-sm active:scale-98 transition-all duration-300 flex items-center justify-center gap-2 cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed"
          >
            Save Hash
          </button>
        ) : (
          <div className="px-3 py-2.5 w-28.75 font-medium bg-olive/40 rounded-sm flex items-center justify-center gap-2 cursor-not-allowed">
            Hashed
          </div>
        )}
      </div>

      <div className="flex flex-row items-center justify-between gap-5 w-full">
        <div className="flex flex-col flex-1 border-2 gap-2.5 border-olive rounded-sm px-4 py-3">
          <label htmlFor="album-query-hash">Album Query Hash (getAlbum)</label>
          <input
            id="album-query-hash"
            type="text"
            value={albumHash.value}
            onChange={e => albumHash.setValue(e.target.value)}
            className="py-2 px-3 bg-charcoal rounded-sm"
          />
        </div>
        {albumHash.dirty ? (
          <button
            onClick={applyAlbumHash}
            className="px-3 py-2.5 w-28.75 font-medium bg-olive/60 hover:bg-olive rounded-sm active:scale-98 transition-all duration-300 flex items-center justify-center gap-2 cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed"
          >
            Save Hash
          </button>
        ) : (
          <div className="px-3 py-2.5 w-28.75 font-medium bg-olive/40 rounded-sm flex items-center justify-center gap-2 cursor-not-allowed">
            Hashed
          </div>
        )}
      </div>
      {status && <p>{status}</p>}
    </div>
  );
}
