import { api, type SearchResult, type TreeNote } from "./api";
import { savedText } from "./offline";
import { app } from "./store.svelte";

/**
 * The notes a search lists, for `q` in lower case: the best full-text matches first, then anything
 * whose title or preview contains the text, newest first. `inScope` limits it to a folder.
 */
export function matchingNotes(q: string, hits: Map<string, SearchResult>, inScope: (note: TreeNote) => boolean = () => true): TreeNote[] {
  const order = [...hits.keys()];
  const rank = (n: TreeNote) => (hits.has(n.id) ? order.indexOf(n.id) : order.length);
  return app.tree.notes
    .filter((n) => inScope(n) && (hits.has(n.id) || `${n.title}\n${n.preview}`.toLowerCase().includes(q)))
    .toSorted((a, b) => rank(a) - rank(b) || b.updated_at - a.updated_at);
}

/**
 * Offline, the server's matches are the copies on this device: whole note text, with the same
 * every-word-as-prefix rule as /search. Notes with no copy here fall back to title and preview.
 */
function offlineHits(q: string): Map<string, SearchResult> {
  const words = q.toLowerCase().split(/[\s\p{P}\p{S}]+/u).filter(Boolean).slice(0, 16);
  if (!words.length) return new Map();
  const out = new Map<string, SearchResult>();
  for (const n of app.tree.notes) {
    const text = savedText(n.id);
    if (!text) continue;
    const hayWords = `${n.title}\n${text}`.toLowerCase().split(/[\s\p{P}\p{S}]+/u);
    if (words.every((w) => hayWords.some((hw) => hw.startsWith(w)))) {
      out.set(n.id, { note: n.id, title: n.title, snippet: [], source: "text" });
    }
  }
  return out;
}

/**
 * The server's full-text matches for what's typed, for a search field, a moment after each key.
 * Finding notes by meaning is Ask's job. Call it while a component starts; `query` is read
 * reactively. Offline it matches whole note text from the copies on this device.
 */
export function liveSearch(query: () => string) {
  let found = $state<{ q: string; results: Map<string, SearchResult> } | null>(null);
  $effect(() => {
    const text = query();
    void app.tree; // Edits that change the list can change what matches, too.
    void app.status;
    if (!text) return;
    const timer = setTimeout(async () => {
      if (app.status !== "online") {
        if (query() === text) found = { q: text, results: offlineHits(text) };
        return;
      }
      try {
        const { results } = await api.search(text);
        if (query() === text) found = { q: text, results: new Map(results.map((r) => [r.note, r])) };
      } catch {
        // Offline or the server is down.
      }
    }, 150);
    return () => clearTimeout(timer);
  });

  const hits = $derived(found && found.q === query() ? found.results : new Map<string, SearchResult>());
  return {
    /** Full-text matches for the current query, by note id, best first. */
    get hits() {
      return hits;
    },
  };
}
