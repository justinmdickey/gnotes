<script lang="ts">
  import { api } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import { app, notesFor, openNote } from "./lib/store.svelte";
  import ShareDialog from "./ShareDialog.svelte";

  const notebook = $derived(
    app.view.kind === "notebook" ? app.tree.notebooks.find((n) => n.id === (app.view as { id: string }).id) : undefined,
  );
  const title = $derived(
    app.view.kind === "all" ? "All Notes" : app.view.kind === "shared-notes" ? "Shared Notes" : (notebook?.name ?? ""),
  );
  const notes = $derived(notesFor(app.view, app.tree));
  const canCreate = $derived(app.view.kind === "all" || (notebook !== undefined && notebook.role !== "viewer"));

  let renaming = $state(false);
  let newName = $state("");
  let sharing = $state(false);

  async function newNote() {
    const { id } = await api.createNote(notebook?.id ?? null);
    openNote(id);
  }

  async function rename(e: SubmitEvent) {
    e.preventDefault();
    if (notebook && newName.trim()) await api.renameNotebook(notebook.id, newName.trim());
    renaming = false;
  }

  async function remove() {
    if (!notebook || !confirm(`Move “${notebook.name}” and everything in it to the trash?`)) return;
    await api.deleteNotebook(notebook.id);
    app.view = { kind: "all" };
  }

  function when(ms: number) {
    const d = new Date(ms);
    const today = new Date().toDateString() === d.toDateString();
    return today ? d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) : d.toLocaleDateString();
  }
</script>

<section>
  <header>
    <button class="flat icon narrow-only" aria-label="Show sidebar" onclick={() => (app.pane = "sidebar")}>
      <Icon name="menu" />
    </button>
    <span class="title">{title}</span>
    {#if notebook?.role === "owner"}
      <button class="flat icon" title="Share notebook" aria-label="Share notebook" onclick={() => (sharing = true)}><Icon name="share" /></button>
      <button class="flat icon" title="Rename notebook" aria-label="Rename notebook" onclick={() => ((newName = notebook.name), (renaming = true))}>
        <Icon name="pencil" />
      </button>
      <button class="flat icon" title="Delete notebook" aria-label="Delete notebook" onclick={remove}><Icon name="trash" /></button>
    {/if}
    {#if canCreate}
      <button class="flat icon" title="New note" aria-label="New note" onclick={newNote}><Icon name="plus" /></button>
    {/if}
  </header>

  <ul>
    {#each notes as note (note.id)}
      <li>
        <button class="flat" class:selected={app.noteId === note.id} onclick={() => openNote(note.id)}>
          <span class="note-title" class:dim={!note.title}>{note.title || "Untitled"}</span>
          <span class="meta dim">
            {when(note.updated_at)}{#if note.role !== "owner"} · {note.owner}{/if}{#if note.role === "viewer"} · view only{/if}
          </span>
        </button>
      </li>
    {:else}
      <li class="empty dim">No notes</li>
    {/each}
  </ul>
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
    border-bottom: 1px solid var(--border);
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

  ul {
    flex: 1;
    margin: 0;
    padding: 6px;
    overflow-y: auto;
    list-style: none;
  }

  li button {
    width: 100%;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    padding: 10px 12px;
    font-weight: 400;
    text-align: left;
  }

  li button.selected {
    background: var(--active);
  }

  .note-title {
    width: 100%;
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    font-size: 0.85rem;
  }

  .empty {
    padding: 24px;
    text-align: center;
  }

  form input {
    width: 100%;
  }
</style>
