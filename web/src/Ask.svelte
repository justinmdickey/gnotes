<script lang="ts">
  import { onMount, tick } from "svelte";
  import type { SnippetPart, TreeNote } from "./lib/api";
  import { answerParts, ask, cards, chat, endChat, startChat, titleParts } from "./lib/ask.svelte";
  import AccountButton from "./lib/AccountButton.svelte";
  import Icon from "./lib/Icon.svelte";
  import StatusPage from "./lib/StatusPage.svelte";
  import { liveSearch, matchingNotes } from "./lib/search.svelte";
  import { app, openNote, pathOf, viewTitle } from "./lib/store.svelte";
  import { media, scrollEdge } from "./lib/ui.svelte";

  /**
   * Search and Ask in one screen. Typing lists matching notes, by their words and (when it's on)
   * by meaning; sending starts a conversation about it, and the same field asks follow-ups. Without
   * AI services set up it's a plain Search. What's typed and the conversation live in lib/ask, so
   * they're still here after a visit to another tab.
   */
  const ai = $derived(!!app.features.ask);
  const chatting = $derived(ai && chat.turns.length > 0);
  const q = $derived(chatting ? "" : chat.draft.trim().toLowerCase());

  const search = liveSearch(() => q);
  const notes = $derived(q ? matchingNotes(q, search.hits) : []);
  /** Meaning matches the word search didn't already list, best first. */
  const meaningNotes = $derived.by(() => {
    const listed = new Set(notes.map((n) => n.id));
    return search.meaning.flatMap((r) => {
      const note = app.tree.notes.find((n) => n.id === r.note);
      return note && !listed.has(note.id) ? [{ note, snippet: r.snippet }] : [];
    });
  });

  let scroller = $state<HTMLElement>();
  let field = $state<HTMLInputElement>();

  // A new question goes to the top, with its answer appearing under it. Coming back to the tab shows the latest one.
  let shown = 0;
  $effect(() => {
    const n = chat.turns.length;
    if (!n || n === shown) return;
    const smooth = shown > 0;
    shown = n;
    void tick().then(() => scroller?.querySelector(".turn:last-child")?.scrollIntoView({ block: "start", behavior: smooth ? "smooth" : "instant" }));
  });

  onMount(() => {
    // A keyboard is ready to type into on wide screens; a phone's would cover the screen.
    if (!media.phone && !chatting) field?.focus();
    return () => (app.typing = false);
  });

  function send(e: SubmitEvent) {
    e.preventDefault();
    const text = chat.draft.trim();
    // A phone puts its keyboard away, so the answer has the screen.
    if (media.phone) field?.blur();
    if (!text || !ai || chat.busy) return;
    if (chatting) {
      chat.draft = "";
      void ask(text);
    } else startChat(text);
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

  /** A search result's passage: where its words matched, or else its preview. */
  function passage(note: TreeNote): SnippetPart[] {
    const hit = search.hits.get(note.id);
    if (hit?.snippet.some((p) => p.hit)) return hit.snippet;
    return note.preview ? [{ text: note.preview, hit: false }] : [];
  }
</script>

{#snippet marked(parts: SnippetPart[])}{#each parts as part, i (i)}{#if part.hit}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}{/snippet}

<!-- A found note, in the results and in a conversation alike: number (in a conversation), title, the passage that matched, and where it lives. -->
{#snippet card(id: string, title: SnippetPart[], snippet: SnippetPart[], n?: number)}
  {@const place = where(id)}
  <li>
    <button class="flat card" onclick={() => openNote(id)}>
      <span class="head">
        {#if n !== undefined}<span class="num" aria-hidden="true">{n}</span>{/if}
        <span class="t">{#if title.length}{@render marked(title)}{:else}New Note{/if}</span>
      </span>
      {#if snippet.length}<span class="dim snippet">{@render marked(snippet)}</span>{/if}
      {#if place}<span class="where dim"><Icon name={place.icon} size={12} /><span>{place.text}</span></span>{/if}
    </button>
  </li>
{/snippet}

<section>
  <header class="headerbar">
    <div class="start">
      <button class="flat icon tablet-only" title="Show notebooks" aria-label="Show notebooks" onclick={() => (app.drawer = true)}>
        <Icon name="sidebar" />
      </button>
    </div>
    <div class="title"><strong>{viewTitle(app.view, app.tree)}</strong></div>
    <div class="end">
      {#if chatting}
        <button class="flat icon new-chat" title="New chat" aria-label="New chat" onclick={() => (endChat(), field?.focus())}>
          <Icon name="compose" />
        </button>
      {/if}
      <AccountButton class="phone-only" />
    </div>
  </header>

  <div class="scroll" use:scrollEdge bind:this={scroller}>
    {#if chatting}
      <div class="turns">
        {#each chat.turns as turn, i (i)}
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
                <p class="answer">{#each answerParts(turn) as part, j (j)}{#if "n" in part}<button class="cite" title={part.title || "New Note"} onclick={() => openNote(part.note)}>{part.n}</button>{:else}{part.text}{/if}{/each}</p>
              {:else if !turn.cited}
                <p class="dim status"><span class="spinner"></span> Reading {turn.notes.length === 1 ? "1 note" : `${turn.notes.length} notes`}…</p>
              {:else if turn.none}
                <p class="dim status">Your notes don't seem to say. These came closest:</p>
              {/if}
              <ul class="boxed-list cards">
                {#each cards(turn) as note (note.n)}
                  {@render card(note.note, note.title ? titleParts(note.title, turn.terms) : [], note.snippet, note.n)}
                {/each}
              </ul>
            {/if}
          </section>
        {/each}
      </div>
      {#if chat.turns.some((t) => t.answer)}
        <p class="dim disclaimer">Written by AI from your notes. It can get things wrong.</p>
      {/if}
    {:else if q}
      {#if notes.length}
        <h3 class="group-title">Notes</h3>
        <ul class="boxed-list cards results">
          {#each notes as note (note.id)}
            {@render card(note.id, note.title ? [{ text: note.title, hit: false }] : [], passage(note))}
          {/each}
        </ul>
      {/if}
      {#if meaningNotes.length}
        <h3 class="group-title">By Meaning</h3>
        <ul class="boxed-list cards meaning">
          {#each meaningNotes as m (m.note.id)}
            {@render card(m.note.id, m.note.title ? [{ text: m.note.title, hit: false }] : [], m.snippet)}
          {/each}
        </ul>
      {/if}
      {#if !notes.length && !meaningNotes.length}
        <StatusPage
          icon="search"
          title="No Results"
          description={ai ? `No note has the words “${chat.draft.trim()}”. Send it to ask your notes instead.` : `Nothing matches “${chat.draft.trim()}”.`}
          tone="neutral"
        />
      {/if}
    {:else if ai}
      <StatusPage icon="sparkle" title="Ask Your Notes" description="Find notes, or ask a question and get an answer from them." fill />
    {:else}
      <StatusPage icon="search" title="Search Your Notes" description="Find notes by the words in them." fill />
    {/if}
  </div>

  <form class="composer" onsubmit={send}>
    <label class="field">
      <Icon name={chatting ? "sparkle" : "search"} />
      <input
        bind:this={field}
        bind:value={chat.draft}
        type={chatting ? "text" : "search"}
        placeholder={chatting ? "Ask a follow-up…" : ai ? "Search or ask…" : "Search notes…"}
        aria-label={chatting ? "Ask a follow-up" : ai ? "Search or ask" : "Search notes"}
        enterkeyhint={ai ? "send" : "search"}
        maxlength="1000"
        onfocus={() => (app.typing = media.phone)}
        onblur={() => (app.typing = false)}
      />
      {#if chat.draft && !chatting}
        <button type="button" class="flat icon circular clear" aria-label="Clear search" onclick={() => ((chat.draft = ""), field?.focus())}><Icon name="close" /></button>
      {/if}
    </label>
    {#if ai}
      <button class="suggested icon circular send" type="submit" title="Ask" aria-label="Ask" disabled={chat.busy || !chat.draft.trim()}>
        <Icon name="send" />
      </button>
    {/if}
  </form>
</section>

<style>
  section {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--view-bg);
  }

  /* Wide screens lay the headerbar out as one row; the groups only matter on phones. */
  .start,
  .end {
    display: contents;
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 4px 12px 16px;
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
    margin: 8px 0 12px auto;
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

  .disclaimer {
    margin: 14px 0 0;
    font-size: var(--text-sm);
  }

  /* The one field, pinned under the results or the conversation, with send beside it. */
  .composer {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px 10px;
  }

  .field {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 4px 0 12px;
    border-radius: var(--radius);
    background: var(--entry-bg);
    color: var(--dim-fg);
    transition: box-shadow var(--fast) ease;
  }

  .field:focus-within {
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-bg) 60%, transparent);
  }

  .field input {
    flex: 1;
    min-width: 0;
    background: transparent;
    padding-left: 0;
    box-shadow: none;
    color: var(--fg);
  }

  .field input::-webkit-search-cancel-button {
    display: none;
  }

  .clear {
    min-width: 28px;
    min-height: 28px;
  }

  /* As tall as the field, so the two line up. */
  .send {
    flex: none;
    width: 40px;
    min-height: 40px;
  }

  /* The page keeps a readable width in the middle of the pane, like a folder page. */
  @media (min-width: 1001px) {
    .composer {
      padding-inline: max(12px, calc((100% - 720px) / 2));
    }
  }

  @media (max-width: 700px) {
    /* Three columns, so the title sits in the middle of the screen whatever buttons are on each side. */
    .headerbar {
      display: grid;
      grid-template-columns: 1fr minmax(0, auto) 1fr;
    }

    .start,
    .end {
      display: flex;
      align-items: center;
      gap: 4px;
    }

    .end {
      justify-content: flex-end;
    }

    .headerbar .title {
      align-items: center;
      text-align: center;
    }

    .scroll {
      padding: 4px 16px 16px;
    }

    .composer {
      padding: 8px 16px 10px;
    }

    .field input {
      min-height: 42px;
    }

    .send {
      width: 42px;
      min-height: 42px;
    }

    .card {
      padding: 12px 16px;
    }

    .head .t {
      font-size: var(--text-lg);
    }
  }
</style>
