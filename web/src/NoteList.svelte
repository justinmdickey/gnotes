<script lang="ts">
  import { api, type TreeNote } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import Menu from "./lib/Menu.svelte";
  import { app, composeNote, goBack, navigate, notesFor, openNote, viewTitle } from "./lib/store.svelte";
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

  async function remove() {
    if (!notebook || !confirm(`Move “${notebook.name}” and everything in it to the trash?`)) return;
    await api.deleteNotebook(notebook.id);
    navigate({ kind: "all" }, null, true);
  }

  function when(ms: number) {
    const d = new Date(ms);
    const today = new Date().toDateString() === d.toDateString();
    return today ? d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) : d.toLocaleDateString();
  }
</script>

<section>
  <header>
    <button class="flat back narrow-only" onclick={goBack}>
      <Icon name="back" /><span>Notebooks</span>
    </button>
    <span class="title">{title}</span>
    {#if notebook?.role === "owner"}
      <button class="flat share" onclick={() => (sharing = true)}><Icon name="people" /><span>Share</span></button>
      <Menu
        label="Notebook menu"
        items={[
          { label: "Rename…", onselect: () => ((newName = notebook.name), (renaming = true)) },
          { label: "Move to Trash", destructive: true, onselect: remove },
        ]}
      />
    {/if}
    {#if canCreate}
      <button class="flat icon wide-only" title="New note" aria-label="New note" onclick={() => composeNote()}><Icon name="compose" /></button>
    {/if}
  </header>

  <div class="search">
    <Icon name="search" />
    <input type="search" placeholder="Search" aria-label="Search notes" bind:value={query} />
  </div>

  <div class="scroll">
    {#each groups as group (group.label)}
      <h3>{group.label}</h3>
      <ul>
        {#each group.notes as note (note.id)}
          <li>
            <button class="flat" class:selected={app.noteId === note.id} onclick={() => openNote(note.id)}>
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
      <div class="empty dim">
        {#if query}
          No results
        {:else}
          <p>No notes yet</p>
          {#if canCreate}<button class="suggested" onclick={() => composeNote()}>New Note</button>{/if}
        {/if}
      </div>
    {/each}
  </div>

  <footer class="phone-only">
    <span class="spacer"></span>
    <span class="dim count">{notes.length} {notes.length === 1 ? "Note" : "Notes"}</span>
    <span class="spacer compose-slot">
      {#if canCreate}
        <button class="flat icon compose" aria-label="New note" onclick={() => composeNote()}><Icon name="compose" /></button>
      {/if}
    </span>
  </footer>
</section>

{#if renaming}
  <Dialog title="Rename Notebook" onclose={() => (renaming = false)}>
    <form id="rename-notebook" onsubmit={rename}>
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={newName} autofocus />
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
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--view-bg);
  }

  header {
    display: flex;
    align-items: center;
    gap: 2px;
    min-height: 47px;
    padding: 0 6px;
    padding-top: env(safe-area-inset-top);
  }

  .back {
    padding: 0 10px 0 6px;
    gap: 4px;
    color: var(--accent);
    font-weight: 400;
  }

  .share {
    gap: 6px;
    padding: 0 12px;
    color: var(--accent);
  }

  .title {
    flex: 1;
    min-width: 0;
    padding-left: 10px;
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 12px 8px;
    padding-left: 10px;
    border-radius: var(--radius);
    background: var(--hover);
    color: var(--dim-fg);
  }

  .search input {
    flex: 1;
    min-width: 0;
    background: transparent;
    padding-left: 0;
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 0 12px 12px;
  }

  h3 {
    margin: 14px 4px 6px;
    font-size: 0.95rem;
    font-weight: 800;
  }

  /* Boxed list, like an AdwPreferencesGroup. */
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--card-bg);
    box-shadow: 0 0 0 1px var(--border);
    overflow: hidden;
  }

  li + li {
    border-top: 1px solid var(--border);
  }

  li button {
    width: 100%;
    flex-direction: column;
    align-items: stretch;
    gap: 2px;
    padding: 10px 14px;
    border-radius: 0;
    font-weight: 400;
    text-align: left;
  }

  li button.selected {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
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
    font-size: 0.9rem;
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
    padding: 48px 16px;
    text-align: center;
  }

  footer {
    display: flex;
    align-items: center;
    min-height: 48px;
    padding: 0 8px max(0px, env(safe-area-inset-bottom));
    border-top: 1px solid var(--border);
    background: var(--headerbar-bg);
  }

  .spacer {
    flex: 1;
  }

  .compose-slot {
    display: flex;
    justify-content: flex-end;
  }

  .compose {
    color: var(--accent);
    min-width: 44px;
    min-height: 44px;
  }

  .compose :global(svg) {
    width: 22px;
    height: 22px;
  }

  .count {
    font-size: 0.85rem;
  }

  form input {
    width: 100%;
  }
</style>
