import { useCallback, useEffect, useRef, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { openUrl } from '@tauri-apps/plugin-opener';
import { AnimatePresence, motion } from 'motion/react';
import { ExternalLink, Sparkles, X } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { consumeReleaseNotesForVersion, type PendingReleaseNotes } from '../lib/releaseNotes';

const ALL_RELEASES_URL = 'https://github.com/TheeHk-blip/sonic-ripper/releases';

export function ReleaseNotesDialog() {
  const [release, setRelease] = useState<PendingReleaseNotes | null>(null);
  const checked = useRef(false);

  useEffect(() => {
    if (checked.current) return;
    checked.current = true;

    getVersion()
      .then(version => {
        const pending = consumeReleaseNotesForVersion(version);
        if (pending) setRelease(pending);
      })
      .catch(err => console.error('[ReleaseNotesDialog] failed to read app version:', err));
  }, []);

  const dismiss = useCallback(() => setRelease(null), []);

  useEffect(() => {
    if (!release) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') dismiss();
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [release, dismiss]);

  const openExternal = (url: string) => {
    openUrl(url).catch(err => console.error('[ReleaseNotesDialog] failed to open link:', err));
  };

  return (
    <AnimatePresence>
      {release && (
        <motion.div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4 backdrop-blur-sm"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          onClick={dismiss}
          role="presentation"
        >
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-labelledby="release-notes-title"
            className="flex max-h-[80vh] w-full max-w-xl flex-col overflow-hidden rounded-2xl border border-gold/40 bg-charcoal shadow-2xl"
            initial={{ opacity: 0, y: 16, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 12, scale: 0.98 }}
            transition={{ duration: 0.2, ease: 'easeOut' }}
            onClick={event => event.stopPropagation()}
          >
            <div className="flex items-start justify-between gap-4 border-b border-olive/40 px-5 py-4">
              <div className="flex items-center gap-3">
                <span className="flex size-10 shrink-0 items-center justify-center rounded-full bg-rust/25 text-gold">
                  <Sparkles className="size-5" />
                </span>
                <div>
                  <h2 id="release-notes-title" className="font-display text-xl text-cream">
                    What&apos;s new
                  </h2>
                  <p className="mt-0.5 text-xs tracking-widest text-gold uppercase">
                    Updated to v{release.version}
                  </p>
                </div>
              </div>
              <button
                type="button"
                aria-label="Close release notes"
                onClick={dismiss}
                className="rounded-sm p-1.5 text-cream/60 transition-colors hover:bg-white/10 hover:text-cream"
              >
                <X className="size-5" />
              </button>
            </div>

            <div className="overflow-y-auto px-5 py-4">
              <ReactMarkdown
                remarkPlugins={[remarkGfm]}
                components={{
                  h1: ({ children }) => (
                    <h3 className="mb-2 mt-5 font-display text-lg text-gold first:mt-0">
                      {children}
                    </h3>
                  ),
                  h2: ({ children }) => (
                    <h4 className="mb-2 mt-5 font-display text-base text-gold first:mt-0">
                      {children}
                    </h4>
                  ),
                  h3: ({ children }) => (
                    <h5 className="mb-2 mt-4 text-sm font-semibold text-gold first:mt-0">
                      {children}
                    </h5>
                  ),
                  p: ({ children }) => (
                    <p className="mb-3 text-sm leading-relaxed text-cream/85">{children}</p>
                  ),
                  ul: ({ children }) => (
                    <ul className="mb-3 list-disc space-y-1 pl-5 text-sm text-cream/85">
                      {children}
                    </ul>
                  ),
                  ol: ({ children }) => (
                    <ol className="mb-3 list-decimal space-y-1 pl-5 text-sm text-cream/85">
                      {children}
                    </ol>
                  ),
                  li: ({ children }) => <li className="leading-relaxed">{children}</li>,
                  a: ({ children, href }) => (
                    <a
                      href={href}
                      target="_blank"
                      rel="noreferrer"
                      className="text-gold underline underline-offset-2 transition-colors hover:text-cream"
                      onClick={event => {
                        event.preventDefault();
                        if (href) openExternal(href);
                      }}
                    >
                      {children}
                    </a>
                  ),
                  strong: ({ children }) => (
                    <strong className="font-semibold text-cream">{children}</strong>
                  ),
                  blockquote: ({ children }) => (
                    <blockquote className="mb-3 border-l-2 border-rust pl-3 text-sm italic text-cream/70">
                      {children}
                    </blockquote>
                  ),
                  pre: ({ children }) => (
                    <pre className="mb-3 overflow-x-auto rounded-sm bg-brown/70 p-3 text-xs text-cream/85">
                      {children}
                    </pre>
                  ),
                  code: ({ children }) => (
                    <code className="rounded-sm bg-brown/70 px-1 py-0.5 text-[0.9em] text-gold">
                      {children}
                    </code>
                  ),
                  table: ({ children }) => (
                    <div className="mb-3 overflow-x-auto">
                      <table className="w-full text-left text-xs text-cream/85">{children}</table>
                    </div>
                  ),
                  th: ({ children }) => (
                    <th className="border-b border-olive/40 px-2 py-1.5 font-semibold text-gold">
                      {children}
                    </th>
                  ),
                  td: ({ children }) => (
                    <td className="border-b border-olive/20 px-2 py-1.5">{children}</td>
                  ),
                  hr: () => <hr className="my-4 border-olive/40" />,
                }}
              >
                {release.notes}
              </ReactMarkdown>
            </div>

            <div className="flex items-center justify-between gap-3 border-t border-olive/40 px-5 py-4">
              <button
                type="button"
                onClick={() => openExternal(ALL_RELEASES_URL)}
                className="flex items-center gap-1.5 text-xs text-cream/60 transition-colors hover:text-gold"
              >
                <ExternalLink className="size-3.5" />
                All releases
              </button>
              <button
                type="button"
                onClick={dismiss}
                className="rounded-sm bg-gold px-5 py-2 text-xs font-semibold tracking-widest text-charcoal uppercase transition-colors hover:bg-rust hover:text-cream"
              >
                Got it
              </button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
