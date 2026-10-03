<script lang="ts">
  import { onMount, tick } from "svelte";
  import { api, ApiError, type AskMessage, type AskNote, type SnippetPart } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import { app, openNote, pathOf } from "./lib/store.svelte";

  /**
   * Ask: a conversation with your notes. Each question shows the notes found for it as cards, then
   * an answer written from them with numbered links. Follow-ups build on the turns before; the
   * conversation lasts until the dialog closes, and only the app keeps it.
   */
  let { question, onclose }: { question: string; onclose: () => void } = $props();

  interface Turn {
    question: string;
    /** The notes found; null while searching. */
    notes: AskNote[] | null;
    terms: string[];
    /** The answer so far, as it streams in. */
    answer: string;
    /** Set once the answer is finished: the notes it cites. */
    cited: number[] | null;
    /** The notes didn't say. */
    none: boolean;
    error: string;
  }

  let turns = $state<Turn[]>([]);
  let draft = $state("");
  let busy = $state(false);
  let list: HTMLElement;
  /** Stops the answer being written when the dialog closes. */
  const stop = new AbortController();

  onMount(() => {
    void ask(question);
    return () => stop.abort();
  });

  /** The earlier turns as the server wants them: question, then the answer, for each one answered. */
  function history(): AskMessage[] {
    return turns.flatMap((t): AskMessage[] =>
      t.answer && t.cited ? [{ role: "user", text: t.question }, { role: "assistant", text: t.answer }] : [{ role: "user", text: t.question }],
    );
  }

  async function ask(text: string) {
    const messages: AskMessage[] = [...history(), { role: "user", text }];
    turns.push({ question: text, notes: null, terms: [], answer: "", cited: null, none: false, error: "" });
    const turn = turns[turns.length - 1];
    busy = true;
    // The new question goes to the top, with its answer appearing under it.
    await tick();
    list?.lastElementChild?.scrollIntoView({ block: "start", behavior: "smooth" });
    try {
      const done = await api.ask(
        messages,
        {
          onSources: (s) => {
            turn.notes = s.notes;
            turn.terms = s.terms;
          },
          onText: (piece) => (turn.answer += piece),
        },
        stop.signal,
      );
      turn.answer = done.answer ?? "";
      turn.none = !done.answer;
      turn.cited = done.cited;
    } catch (err) {
      if (stop.signal.aborted) return;
      turn.error = err instanceof ApiError && err.status ? err.message : "Couldn't reach the server";
    } finally {
      busy = false;
    }
  }

  function send(e: SubmitEvent) {
    e.preventDefault();
    const text = draft.trim();
    if (!text || busy) return;
    draft = "";
    void ask(text);
  }

  /** The cards to show: every note found, then only the ones the finished answer cites. */
  function cards(turn: Turn): AskNote[] {
    const notes = turn.notes ?? [];
    if (!turn.cited?.length) return notes;
    return turn.cited.flatMap((n) => notes.filter((note) => note.n === n));
  }

  /** The answer as text and [n] citations, which become links to their notes. Numbers that aren't a found note are dropped. */
  function parts(turn: Turn) {
    const out: ({ text: string } | AskNote)[] = [];
    let last = 0;
    for (const m of turn.answer.matchAll(/\s?\[(\d+(?:\s*,\s*\d+)*)\]/g)) {
      out.push({ text: turn.answer.slice(last, m.index) });
      for (const n of m[1].split(",").map(Number)) {
        const note = turn.notes?.find((s) => s.n === n);
        if (note) out.push(note);
      }
      last = m.index + m[0].length;
    }
    out.push({ text: turn.answer.slice(last) });
    return out;
  }

  /** A title split into words, the ones searched for as hits, like the snippet under it. */
  function titleParts(title: string, terms: string[]): SnippetPart[] {
    return title.split(/(\s+)/).map((word) => {
      const w = word.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "").toLowerCase();
      return { text: word, hit: !!w && terms.some((t) => w.startsWith(t)) };
    });
  }

  /** Where a note lives, as in search results. */
  function where(id: string) {
    const note = app.tree.notes.find((n) => n.id === id);
    if (!note) return null;
    const path = pathOf(note.notebook_id);
    return {
      icon: note.notebook_id ? ("folder" as const) : note.role === "owner" ? ("home" as const) : ("people" as const),
      text: path.length ? path.join(" › ") : note.role === "owner" ? "Notes" : `Shared by ${note.owner}`,
    };
  }

  function open(note: string) {
    openNote(note);
    onclose();
  }
</script>

{#snippet marked(parts: SnippetPart[])}{#each parts as part, i (i)}{#if part.hit}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}{/snippet}

<Dialog title="Ask Your Notes" {onclose} wide>
  <!-- Focus starts here, not in the follow-up field, so a phone's keyboard doesn't cover the answer. -->
  <!-- svelte-ignore a11y_autofocus -->
  <div class="turns" bind:this={list} tabindex="-1" autofocus>
    {#each turns as turn, i (i)}
      <section class="turn" aria-busy={!turn.cited && !turn.error}>
        <p class="question">{turn.question}</p>
        {#if turn.error}
          <p class="error">{turn.error}</p>
        {:else if !turn.notes}
          <p class="dim status"><span class="spinner"></span> Searching your notes…</p>
        {:else if !turn.notes.length}
          <p class="dim status">Nothing in your notes matches that. Try asking another way.</p>
        {:else}
          {#if turn.answer}
            <p class="answer">{#each parts(turn) as part, j (j)}{#if "n" in part}<button class="cite" title={part.title || "New Note"} onclick={() => open(part.note)}>{part.n}</button>{:else}{part.text}{/if}{/each}</p>
          {:else if !turn.cited}
            <p class="dim status"><span class="spinner"></span> Reading {turn.notes.length === 1 ? "1 note" : `${turn.notes.length} notes`}…</p>
          {:else if turn.none}
            <p class="dim status">Your notes don't seem to say. These came closest:</p>
          {/if}
          <ul class="boxed-list cards">
            {#each cards(turn) as note (note.n)}
              {@const place = where(note.note)}
              <li>
                <button class="flat card" onclick={() => open(note.note)}>
                  <span class="head">
                    <span class="num" aria-hidden="true">{note.n}</span>
                    <span class="t">{#if note.title}{@render marked(titleParts(note.title, turn.terms))}{:else}New Note{/if}</span>
                  </span>
                  {#if note.snippet.length}<span class="dim snippet">{@render marked(note.snippet)}</span>{/if}
                  {#if place}<span class="where dim"><Icon name={place.icon} size={12} /><span>{place.text}</span></span>{/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/each}
  </div>
  {#if turns.some((t) => t.answer)}
    <p class="dim note">Written by AI from your notes. It can get things wrong.</p>
  {/if}
  {#snippet actions()}
    <form class="composer" onsubmit={send}>
      <input bind:value={draft} placeholder="Ask a follow-up…" aria-label="Ask a follow-up" enterkeyhint="send" maxlength="1000" />
      <button class="suggested" type="submit" disabled={busy || !draft.trim()}>Ask</button>
    </form>
  {/snippet}
</Dialog>

<style>
  .turns:focus {
    outline: none;
  }

  .turn {
    /* Room above a new question when it scrolls to the top. */
    scroll-margin-top: 8px;
  }

  .turn + .turn {
    margin-top: 22px;
    padding-top: 18px;
    border-top: 1px solid var(--border);
  }

  /* What was asked, like a sent message: on the right, in a soft accent bubble. */
  .question {
    width: fit-content;
    max-width: 85%;
    margin: 0 0 12px auto;
    padding: 8px 12px;
    border-radius: var(--radius-lg) var(--radius-lg) var(--radius-sm) var(--radius-lg);
    background: var(--accent-soft);
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .status {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 4px 0 12px;
  }

  .error {
    color: var(--destructive);
  }

  /* Plain text from the model, so its line breaks and "- " lists show as written. */
  .answer {
    margin: 0 0 12px;
    line-height: 1.5;
    white-space: pre-line;
    overflow-wrap: anywhere;
  }

  /* A citation: the note's number, small and round, in the accent color since it links somewhere. */
  .cite,
  .num {
    display: inline-grid;
    place-items: center;
    min-width: 20px;
    min-height: 20px;
    padding: 0 5px;
    border-radius: var(--radius-pill);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: var(--text-xs);
    font-weight: 700;
    line-height: 1;
  }

  .cite {
    margin: 0 1px 0 3px;
    vertical-align: 0.1em;
  }

  /* A found note: number and title, the passage that matched, and where it lives. */
  .card {
    width: 100%;
    flex-direction: column;
    align-items: stretch;
    gap: 3px;
    padding: 10px 14px;
    border-radius: 0;
    font-weight: 400;
    text-align: left;
  }

  .card:active:not(:disabled) {
    transform: none;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    font-weight: 700;
  }

  .num {
    flex: none;
  }

  .head .t {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .snippet {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
    overflow-wrap: anywhere;
    font-size: var(--text-sm);
  }

  /* Matched words, marked like search results. */
  .card mark {
    padding: 0 2px;
    border-radius: 3px;
    background: var(--highlight-bg);
    color: var(--highlight-fg);
    box-decoration-break: clone;
  }

  .where {
    display: flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .where span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .note {
    margin: 14px 0 0;
    font-size: var(--text-sm);
  }

  .composer {
    display: flex;
    flex: 1;
    gap: 8px;
  }

  .composer input {
    flex: 1;
    min-width: 0;
  }

  .composer button {
    flex: none;
    padding: 0 18px;
  }

  @media (max-width: 700px) {
    .head .t {
      font-size: var(--text-lg);
    }
  }
</style>
