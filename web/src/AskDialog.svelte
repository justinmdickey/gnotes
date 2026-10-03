<script lang="ts">
  import { api, ApiError, type AskAnswer } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import StatusPage from "./lib/StatusPage.svelte";
  import { openNote } from "./lib/store.svelte";

  /** Ask: a short answer written from your own notes, with links to the notes it came from. */
  let { question, onclose }: { question: string; onclose: () => void } = $props();

  let reply = $state<AskAnswer | null>(null);
  let error = $state("");

  $effect(() => {
    api.ask(question).then(
      (r) => (reply = r),
      (err) => (error = err instanceof ApiError ? err.message : "Couldn't reach the server"),
    );
  });

  /** The answer as text and [n] citations, which become links to their notes. Numbers that aren't a note are dropped. */
  const parts = $derived.by(() => {
    const out: ({ text: string } | { n: number; title: string; note: string })[] = [];
    if (!reply?.answer) return out;
    let last = 0;
    for (const m of reply.answer.matchAll(/\s?\[(\d+(?:\s*,\s*\d+)*)\]/g)) {
      out.push({ text: reply.answer.slice(last, m.index) });
      for (const n of m[1].split(",").map(Number)) {
        const source = reply.sources.find((s) => s.n === n);
        if (source) out.push(source);
      }
      last = m.index + m[0].length;
    }
    out.push({ text: reply.answer.slice(last) });
    return out;
  });

  function open(note: string) {
    openNote(note);
    onclose();
  }
</script>

<Dialog title="Ask Your Notes" {onclose}>
  <p class="question">{question}</p>
  {#if error}
    <p class="error">{error}</p>
  {:else if !reply}
    <p class="dim reading"><span class="spinner"></span> Reading your notes…</p>
  {:else if !reply.answer}
    <StatusPage icon="search" title="No Answer Found" description="Your notes don't seem to say. Try asking another way." tone="neutral" />
  {:else}
    <p class="answer">{#each parts as part, i (i)}{#if "n" in part}<button class="cite" title={part.title || "New Note"} onclick={() => open(part.note)}>{part.n}</button>{:else}{part.text}{/if}{/each}</p>
    <h3 class="group-title">From Your Notes</h3>
    <ul class="boxed-list sources">
      {#each reply.sources as s (s.n)}
        <li>
          <button class="flat source" onclick={() => open(s.note)}>
            <span class="cite" aria-hidden="true">{s.n}</span>
            <span class="t">{s.title || "New Note"}</span>
            <span class="dim chev"><Icon name="next" /></span>
          </button>
        </li>
      {/each}
    </ul>
    <p class="dim note">Written by AI from these notes. It can get things wrong.</p>
  {/if}
  {#snippet actions()}
    <button onclick={onclose}>Close</button>
  {/snippet}
</Dialog>

<style>
  .question {
    margin: -4px 0 12px;
    color: var(--dim-fg);
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .reading {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 8px 0 16px;
  }

  .error {
    color: var(--destructive);
  }

  /* Plain text from the model, so its line breaks and "- " lists show as written. */
  .answer {
    margin: 0 0 4px;
    line-height: 1.5;
    white-space: pre-line;
    overflow-wrap: anywhere;
  }

  /* A citation: the note's number, small and round, in the accent color since it links somewhere. */
  .cite {
    display: inline-grid;
    place-items: center;
    min-width: 20px;
    min-height: 20px;
    margin: 0 1px 0 3px;
    padding: 0 5px;
    border-radius: var(--radius-pill);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: var(--text-xs);
    font-weight: 700;
    line-height: 1;
    vertical-align: 0.1em;
  }

  .sources {
    margin-bottom: 10px;
  }

  .source {
    width: 100%;
    justify-content: flex-start;
    gap: 12px;
    min-height: 50px;
    padding: 0 12px 0 14px;
    border-radius: 0;
    font-weight: 600;
    text-align: left;
  }

  .source:active:not(:disabled) {
    transform: none;
  }

  .source .cite {
    flex: none;
    margin: 0;
  }

  .source .t {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .source .chev {
    display: flex;
  }

  .note {
    margin: 0;
    font-size: var(--text-sm);
  }
</style>
