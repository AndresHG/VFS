// Content search on the screen (VFS-27): when the offer shows, what the status area and the
// content section say, the fragment's highlights, and the content query with its debounce and
// sequence numbers. No DOM here, so it is tested outside the browser
// (`apps/desktop/ui-dev/content.test.mjs`).

import { TimeUnit } from './choices.js';
import { ContentPhase, ContentStop, ContentWait, FileKind, RootStatus, Source, UnreadReason } from './contract.js';
import { showsCloudBadge } from './copies.js';

export { TimeUnit };

/** R12: the content query starts this long after typing stops… */
export const CONTENT_DEBOUNCE_MS = 300;
/** …and only for queries of at least this many characters. */
export const CONTENT_MIN_CHARS = 3;

/** What the content part of the status area offers (R8, R15). */
export const ContentAction = Object.freeze({ PAUSE: 'pause', RESUME: 'resume', RETRY: 'retry' });

const MINUTE = 60;
const HOUR = 3600;
const DAY = 86_400;

/** Content search is switched on in status `st` (a `content_status` answer, or null). */
export function contentOn(st) {
  return !!st?.settings?.enabled;
}

/**
 * R4: the offer shows once content search was never offered nor switched on, and every
 * folder has finished its first name scan (installs upgraded from 0.2.x included).
 */
export function offerDue(st, roots) {
  if (!st || st.settings.offered || st.settings.enabled) return false;
  const live = roots.filter((r) => r.status !== RootStatus.REMOVING);
  return live.length > 0 && live.every((r) => r.lastFullScan != null);
}

/** Seconds as whole numbers of one rough unit, for a range (`lo`–`hi`) or one value. */
export function roughTime(lo, hi = lo) {
  const [unit, size] = hi < 90 * MINUTE ? [TimeUnit.MINUTES, MINUTE]
    : hi < 48 * HOUR ? [TimeUnit.HOURS, HOUR] : [TimeUnit.DAYS, DAY];
  const whole = (s) => Math.max(1, Math.round(s / size));
  return { unit, lo: whole(lo), hi: whole(hi) };
}

/** The offer's sentence and, below 2 GB free, its warning (R4, R6). `o` is `content_offer`'s
 *  answer: {documents, seconds: [lo, hi], bytes: [lo, hi], freeBytes, lowSpace}. */
export function offerText({ S, N }, o) {
  const t = roughTime(o.seconds[0], o.seconds[1]);
  const time = S.timeRange[t.unit](N.count(t.lo), N.count(t.hi));
  const disk = S.sizeRange(N.size(o.bytes[0]), N.size(o.bytes[1]));
  return {
    text: S.contentOffer(N.count(o.documents), time, disk),
    warn: o.lowSpace && o.freeBytes != null ? S.contentOfferLowSpace(N.size(o.freeBytes)) : '',
  };
}

/**
 * The content part of the status area for status `st` (R8, R15): {text, action} or null when
 * there is nothing to say. Damage has its own notice (R11). The time left counts reading
 * time only, so it says "with the computer idle".
 */
export function contentStatusLine({ S, N }, st) {
  if (!st) return null;
  const done = N.count(st.done);
  const total = N.count(st.total);
  const { kind, reason } = st.phase;
  switch (kind) {
    case ContentPhase.OFF:
    case ContentPhase.DAMAGED:
      return null;
    case ContentPhase.CHECKING:
      return { text: S.contentChecking, action: null };
    case ContentPhase.PAUSED:
      return { text: S.contentPaused(done, total), action: ContentAction.RESUME };
    case ContentPhase.STOPPED:
      return {
        text: S.contentStopped[reason] ?? S.contentStopped[ContentStop.WORKER_CANNOT_START],
        action: ContentAction.RETRY,
      };
    case ContentPhase.FINISHED:
      // Done short of the total: the rest is in offline or paused folders (R9).
      return st.done < st.total ? { text: S.contentFinishedPart(done, total), action: null } : null;
    default: {
      const parts = [S.contentReading(done, total)];
      const wait = kind === ContentPhase.WAITING ? S.contentWaiting[reason] : '';
      if (wait) {
        parts.push(wait);
      } else {
        if (st.perHour != null) parts.push(S.contentRate(N.count(Math.round(st.perHour))));
        if (st.secondsLeft != null) {
          const t = roughTime(st.secondsLeft);
          parts.push(S.timeLeft[t.unit](N.count(t.hi)));
        }
      }
      // A low-space pause ends with «Reanudar» too (R6).
      const action = reason === ContentWait.LOW_SPACE ? ContentAction.RESUME : ContentAction.PAUSE;
      return { text: parts.join(' · '), action };
    }
  }
}

/** The lines above the content hits (R13): reading still under way, and a partial result. */
export function sectionNotes({ S, N }, st, partial) {
  const notes = [];
  if (st && st.done < st.total) {
    const done = N.count(st.done);
    const total = N.count(st.total);
    notes.push(st.phase.kind === ContentPhase.FINISHED ? S.contentReadPart(done, total) : S.contentStillReading(done, total));
  }
  if (partial) notes.push(S.contentPartial);
  return notes;
}

/** R12: a content query needs this many characters, spaces at the ends not counted. */
export function contentQueryDue(text) {
  return [...String(text).trim()].length >= CONTENT_MIN_CHARS;
}

/** The fragment as runs of {text, hit}. `highlights` are `[start, end)` UTF-8 byte offsets
 *  (VFS-2 search); a range out of order or out of bounds is ignored. */
export function snippetRuns(snippet, highlights) {
  const bytes = new TextEncoder().encode(snippet);
  const dec = new TextDecoder();
  const runs = [];
  let at = 0;
  for (const [a, b] of [...highlights].sort((x, y) => x[0] - y[0])) {
    if (a < at || a >= b || b > bytes.length) continue;
    if (a > at) runs.push({ text: dec.decode(bytes.subarray(at, a)), hit: false });
    runs.push({ text: dec.decode(bytes.subarray(a, b)), hit: true });
    at = b;
  }
  if (at < bytes.length) runs.push({ text: dec.decode(bytes.subarray(at)), hit: false });
  return runs;
}

/** The page of a content hit's fragment, when the format has pages. */
export function hitPage(hit) {
  return hit.passages?.[0]?.page ?? null;
}

/** R14: the name hits that may carry an unread reason: files of the document kind. */
export function unreadCandidates(hits) {
  return hits
    .filter((h) => h && h.source !== Source.BOOKMARKS && !h.isDir && h.kind === FileKind.DOCUMENT)
    .map((h) => h.id);
}

/** R14: the label of a name hit with unread reason `reason` (or null): none for a cloud-only
 *  file that already has its cloud badge. */
export function unreadLabel(S, hit, reason) {
  if (!reason || (reason === UnreadReason.CLOUD && showsCloudBadge(hit))) return '';
  return S.contentUnread[reason] || '';
}

/**
 * The content section's query (R12). `query(text, filters, on)` starts it
 * CONTENT_DEBOUNCE_MS after the last call, only when `on` and the text is long enough; each
 * query has a sequence number and an answer to an older one is dropped (`retrieve` itself is
 * not interrupted, D5). `more()` loads the next page. `render(view)` gets null (no section) or
 * {hits, partial, more, loading, failed}. `schedule` and `cancel` are setTimeout and
 * clearTimeout, injected for tests.
 */
export function createContentSearch({ api, render, schedule = setTimeout, cancel = clearTimeout }) {
  let seq = 0;
  let timer = 0;
  let current = null;

  const view = () => current && {
    hits: current.hits, partial: current.partial, more: current.more, loading: current.loading, failed: current.failed,
  };

  async function fetch(mine, offset) {
    if (mine !== seq) return;
    const q = current;
    q.loading = true;
    q.failed = false;
    render(view());
    try {
      const res = await api.contentSearch(q.text, q.filters, offset);
      if (mine !== seq) return; // a newer query is on its way
      q.hits = offset ? q.hits.concat(res.hits) : res.hits;
      q.partial = res.partial;
      q.more = q.hits.length < res.total && res.hits.length > 0;
    } catch {
      if (mine !== seq) return;
      q.failed = true;
    }
    q.loading = false;
    render(view());
  }

  function query(text, filters, on) {
    cancel(timer);
    const mine = ++seq;
    if (!on || !contentQueryDue(text)) {
      current = null;
      render(null);
      return;
    }
    // The older hits go at once, so they are never read as this query's (R12).
    current = { text: String(text).trim(), filters, hits: [], partial: false, more: false, loading: true, failed: false };
    render(view());
    timer = schedule(() => fetch(mine, 0), CONTENT_DEBOUNCE_MS);
  }

  function more() {
    if (current?.more && !current.loading) fetch(seq, current.hits.length);
  }

  return { query, more, view };
}
