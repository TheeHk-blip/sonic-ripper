const LAST_SEEN_VERSION_KEY = 'sonic-ripper.lastSeenVersion';
const PENDING_RELEASE_NOTES_KEY = 'sonic-ripper.pendingReleaseNotes';

export type PendingReleaseNotes = {
  version: string;
  notes: string;
};

function safeGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch (err) {
    console.error(`[releaseNotes] failed to read ${key}:`, err);
    return null;
  }
}

function safeSet(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch (err) {
    console.error(`[releaseNotes] failed to write ${key}:`, err);
  }
}

function safeRemove(key: string): void {
  try {
    localStorage.removeItem(key);
  } catch (err) {
    console.error(`[releaseNotes] failed to remove ${key}:`, err);
  }
}

/** Save the notes from the updater so they can be shown after relaunch. */
export function queueReleaseNotes(version: string, notes: string | null | undefined): void {
  const trimmed = notes?.trim();
  if (!trimmed) {
    safeRemove(PENDING_RELEASE_NOTES_KEY);
    return;
  }

  safeSet(PENDING_RELEASE_NOTES_KEY, JSON.stringify({ version, notes: trimmed }));
}

/**
 * Returns the saved notes once, on the first launch of a new version.
 * Fresh installs and launches without a matching queued update are skipped.
 */
export function consumeReleaseNotesForVersion(currentVersion: string): PendingReleaseNotes | null {
  const lastSeenVersion = safeGet(LAST_SEEN_VERSION_KEY);
  safeSet(LAST_SEEN_VERSION_KEY, currentVersion);

  if (!lastSeenVersion || lastSeenVersion === currentVersion) {
    safeRemove(PENDING_RELEASE_NOTES_KEY);
    return null;
  }

  const raw = safeGet(PENDING_RELEASE_NOTES_KEY);
  safeRemove(PENDING_RELEASE_NOTES_KEY);

  if (!raw) return null;

  try {
    const pending = JSON.parse(raw) as Partial<PendingReleaseNotes>;
    if (
      pending.version !== currentVersion ||
      typeof pending.notes !== 'string' ||
      !pending.notes.trim()
    ) {
      return null;
    }

    return { version: currentVersion, notes: pending.notes.trim() };
  } catch (err) {
    console.error('[releaseNotes] failed to parse saved notes:', err);
    return null;
  }
}
