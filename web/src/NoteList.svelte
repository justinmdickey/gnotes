<script lang="ts">
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { api, type TreeNote } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import Menu from "./lib/Menu.svelte";
  import { app, composeNote, goBack, notesFor, openNote, trashNotebook, viewTitle } from "./lib/store.svelte";
  import { media, scrollEdge } from "./lib/ui.svelte";
  import ShareDialog from "./ShareDialog.svelte";

  const notebook = $derived(
    app.view.kind === "notebook" ? app.tree.notebooks.find((n) => n.id === (app.view as { id: string }).id) : undefined,
  );
  const title = $derived(viewTitle(app.view, app.tree));
  const canCreate = $derived(app.view.kind === "all" || (notebook !== undefined && notebook.role !== "viewer"));

  let query = $state("");
  let renaming = $state(false);
  let newName = $state("");
  let sharing = $state(false);

  const notes = $derived.by(() => {
    const all = notesFor(app.view, app.tree);
    const q = query.trim().toLowerCase();
    return q ? all.filter((n) => `${n.title}\n${n.preview}`.toLowerCase().includes(q)) : all;
  });

  /** Apple Notes-style buckets: Today, Yesterday, Previous 7 Days, Previous 30 Days, then by month. */
  const groups = $derived.by(() => {
    const now = new Date();
    const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
    const day = 24 * 60 * 60 * 1000;
    const out: { label: string; notes: TreeNote[] }[] = [];
    for (const note of notes) {
      const t = note.updated_at;
      const label =
        t >= startOfToday
          ? "Today"
          : t >= startOfToday - day
            ? "Yesterday"
            : t >= startOfToday - 7 * day
              ? "Previous 7 Days"
              : t >= startOfToday - 30 * day
                ? "Previous 30 Days"
                : new Date(t).toLocaleDateString([], { month: "long", year: "numeric" });
      if (out.at(-1)?.label !== label) out.push({ label, notes: [] });
      out.at(-1)!.notes.push(note);
    }
    return out;
  });

  async function rename(e: SubmitEvent) {
    e.preventDefault();
    if (notebook && newName.trim()) await api.renameNotebook(notebook.id, newName.trim());
    renaming = false;
  }

  function when(ms: number) {
    const d = new Date(ms);
    const today = new Date().toDateString() === d.toDateString();
    return today ? d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) : d.toLocaleDateString();
  }

  const reveal = (node: Element) => slide(node, { duration: media.reduced ? 0 : 200, easing: cubicOut });
</script>

<section>
  <header class="headerbar">
    <button class="flat icon tablet-only" title="Show notebooks" aria-label="Show notebooks" onclick={() => (app.drawer = true)}>
      <Icon name="sidebar" />
    </button>
    <button class="flat back phone-only" onclick={goBack}>
      <Icon name="back" /><span>Notebooks</span>
    </button>
    <div class="title">
      <strong>{title}</strong>
      <span>{notes.length} {notes.length === 1 ? "note" : "notes"}</span>
    </div>
    {#if notebook?.role === "owner"}
      <button class="flat accent share" title="Share this notebook" onclick={() => (sharing = true)}>
        <Icon name="people" /><span>Share</span>
      </button>
      <Menu
        label="Notebook menu"
        items={[
          { label: "Rename…", icon: "rename", onselect: () => ((newName = notebook.name), (renaming = true)) },
          { label: "Move to Trash", icon: "trash", destructive: true, onselect: () => trashNotebook(notebook.id, notebook.name) },
        ]}
      />
    {/if}
    {#if canCreate}
      <button class="suggested icon wide-only new" title="New note" aria-label="New note" onclick={() => composeNote()}>
        <Icon name="compose" />
      </button>
    {/if}
  </header>

  <div class="scroll" use:scrollEdge>
    <label class="search">
      <Icon name="search" />
      <input type="search" placeholder="Search notes" aria-label="Search notes" bind:value={query} />
      {#if query}
        <button class="flat icon circular clear" aria-label="Clear search" onclick={() => (query = "")}><Icon name="close" /></button>
      {/if}
    </label>

    <!-- A new list for each view; only adds and removes within one view animate. -->
    {#key app.view.kind === "notebook" ? app.view.id : app.view.kind}
    {#each groups as group (group.label)}
      <h3>{group.label}</h3>
      <ul class="boxed-list">
        {#each group.notes as note (note.id)}
          <li transition:reveal>
            <button class="flat note" class:selected={app.noteId === note.id} onclick={() => openNote(note.id)}>
              <span class="note-title">{note.title || "New Note"}</span>
              <span class="meta">
                <span class="time">{when(note.updated_at)}</span>
                <span class="dim preview">{note.preview || (note.role !== "owner" ? note.owner : "No additional text")}</span>
              </span>
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <div class="empty">
        {#if query}
          <div class="empty-icon"><Icon name="search" size={36} /></div>
          <strong>No Results</strong>
          <p class="dim">Nothing matches “{query}”.</p>
        {:else}
          <div class="empty-icon"><Icon name="note" size={36} /></div>
          <strong>No Notes Yet</strong>
          {#if canCreate}
            <p class="dim">Notes you write here show up in this list.</p>
            <button class="suggested pill" onclick={() => composeNote()}><Icon name="compose" /> New Note</button>
          {:else}
            <p class="dim">Nothing has been shared here yet.</p>
          {/if}
        {/if}
      </div>
    {/each}
    {/key}
  </div>

  {#if canCreate}
    <button class="fab phone-only" onclick={() => composeNote()}><Icon name="compose" /> New Note</button>
  {/if}
</section>

{#if renaming}
  <Dialog title="Rename Notebook" onclose={() => (renaming = false)}>
    <form id="rename-notebook" onsubmit={rename}>
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={newName} aria-label="Notebook name" autofocus />
    </form>
    {#snippet actions()}
      <button onclick={() => (renaming = false)}>Cancel</button>
      <button class="suggested" type="submit" form="rename-notebook" disabled={!newName.trim()}>Rename</button>
    {/snippet}
  </Dialog>
{/if}

{#if sharing && notebook}
  <ShareDialog kind="notebook" id={notebook.id} name={notebook.name} onclose={() => (sharing = false)} />
{/if}

<style>
  section {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--view-bg);
  }

  .share {
    gap: 6px;
    padding: 0 12px;
  }

  .new {
    margin-left: 2px;
    min-width: 36px;
    border-radius: 50%;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 2px 0 6px;
    padding: 0 4px 0 12px;
    border-radius: var(--radius);
    background: var(--entry-bg);
    color: var(--dim-fg);
    transition: box-shadow var(--fast) ease;
  }

  .search:focus-within {
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-bg) 60%, transparent);
  }

  .search input {
    flex: 1;
    min-width: 0;
    background: transparent;
    padding-left: 0;
    box-shadow: none;
    color: var(--fg);
  }

  .search input::-webkit-search-cancel-button {
    display: none;
  }

  .clear {
    min-width: 28px;
    min-height: 28px;
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 0 12px 16px;
  }

  h3 {
    margin: 16px 4px 6px;
    font-size: 0.82rem;
    font-weight: 800;
    color: var(--dim-fg);
  }

  .note {
    width: 100%;
    flex-direction: column;
    align-items: stretch;
    gap: 3px;
    padding: 11px 14px;
    border-radius: 0;
    font-weight: 400;
    text-align: left;
  }

  .note:active:not(:disabled) {
    transform: none;
  }

  .note.selected {
    background: var(--accent-soft);
    box-shadow: inset 3px 0 0 var(--accent-bg);
  }

  .note-title {
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    display: flex;
    gap: 8px;
    font-size: 0.88rem;
    min-width: 0;
  }

  .time {
    flex: none;
  }

  .preview {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 56px 16px;
    text-align: center;
    animation: rise 260ms var(--ease-out) both;
  }

  .empty-icon {
    display: grid;
    place-items: center;
    width: 72px;
    height: 72px;
    margin-bottom: 10px;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent);
  }

  .empty strong {
    font-size: 1.1rem;
  }

  .empty p {
    margin: 0 0 16px;
  }

  form input {
    width: 100%;
  }

  @media (max-width: 700px) {
    .headerbar .title {
      align-items: center;
      text-align: center;
    }

    /* Keep the title centered between the back button and the actions. */
    .back {
      max-width: 38%;
    }

    .scroll {
      padding: 0 16px 96px;
    }

    .search input {
      min-height: 42px;
    }

    .share span {
      display: none;
    }

    .note {
      padding: 13px 16px;
    }

    .note-title {
      font-size: 1.05rem;
    }

    .note.selected {
      background: transparent;
      box-shadow: none;
    }
  }
</style>
