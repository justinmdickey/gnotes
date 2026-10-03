import { api, type SearchResult, type TreeNote } from "./api";
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
 * The server's full-text matches for what's typed, for a search field, a moment after each key.
 * Finding notes by meaning is Ask's job. Call it while a component starts; `query` is read
 * reactively. Offline it stays empty, and the caller's own title and preview matches are all there is.
 */
export function liveSearch(query: () => string) {
  let found = $state<{ q: string; results: Map<string, SearchResult> } | null>(null);
  $effect(() => {
    const text = query();
    void app.tree; // Edits that change the list can change what matches, too.
    if (!text) return;
    const timer = setTimeout(async () => {
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
