<script lang="ts">
  import { api, type TreeNotebook } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import { app, composeNote, navigate, notesFor, openSettings, type View } from "./lib/store.svelte";

  const count = (view: View) => notesFor(view, app.tree).length;

  let creating = $state<{ parent: string | null } | null>(null);
  let name = $state("");

  const byParent = $derived.by(() => {
    const ids = new Set(app.tree.notebooks.map((n) => n.id));
    const map = new Map<string | null, TreeNotebook[]>();
    for (const nb of app.tree.notebooks) {
      // A shared notebook whose parent I can't see is a root for me.
      const parent = nb.parent_id && ids.has(nb.parent_id) ? nb.parent_id : null;
      map.set(parent, [...(map.get(parent) ?? []), nb]);
    }
    return map;
  });
  const ownRoots = $derived((byParent.get(null) ?? []).filter((n) => n.role === "owner"));
  const hiddenShares = $derived(new Set(app.tree.shared.filter((s) => s.hidden).map((s) => s.resource_id)));
  const sharedRoots = $derived((byParent.get(null) ?? []).filter((n) => n.role !== "owner" && !hiddenShares.has(n.id)));
  const hasSharedNotes = $derived(app.tree.shared.some((s) => s.resource_type === "note" && !s.hidden));

  function select(view: View) {
    navigate(view);
  }

  function isSelected(view: View) {
    return app.view.kind === view.kind && (view.kind !== "notebook" || (app.view as { id: string }).id === view.id);
  }

  async function create(e: SubmitEvent) {
    e.preventDefault();
    if (!creating || !name.trim()) return;
    const { id } = await api.createNotebook(name.trim(), creating.parent);
    creating = null;
    name = "";
    select({ kind: "notebook", id });
  }
</script>

{#snippet notebookRow(nb: TreeNotebook, depth: number)}
  <button
    class="row flat"
    class:selected={isSelected({ kind: "notebook", id: nb.id })}
    style:padding-left="{12 + depth * 16}px"
    onclick={() => select({ kind: "notebook", id: nb.id })}
  >
    <Icon name="folder" />
    <span class="label">{nb.name}</span>
    {#if nb.role !== "owner"}<span class="dim owner">{nb.owner}</span>{/if}
    <span class="dim count">{count({ kind: "notebook", id: nb.id })}</span>
  </button>
  {#each byParent.get(nb.id) ?? [] as child (child.id)}
    {@render notebookRow(child, depth + 1)}
  {/each}
{/snippet}

<nav>
  <header>
    <span class="title">Gnotes</span>
    <button class="flat icon" title="Settings" aria-label="Settings" onclick={openSettings}><Icon name="settings" /></button>
  </header>
  <div class="scroll">
    <button class="row flat" class:selected={isSelected({ kind: "all" })} onclick={() => select({ kind: "all" })}>
      <Icon name="note" /><span class="label">All Notes</span>
      <span class="dim count">{count({ kind: "all" })}</span>
    </button>

    <div class="section">
      <span>Notebooks</span>
      <button class="flat icon" title="New notebook" aria-label="New notebook" onclick={() => (creating = { parent: null })}>
        <Icon name="plus" />
      </button>
    </div>
    {#each ownRoots as nb (nb.id)}
      {@render notebookRow(nb, 0)}
    {:else}
      <p class="dim empty">No notebooks yet</p>
    {/each}

    {#if sharedRoots.length || hasSharedNotes}
      <div class="section"><span>Shared with Me</span></div>
      {#if hasSharedNotes}
        <button class="row flat" class:selected={isSelected({ kind: "shared-notes" })} onclick={() => select({ kind: "shared-notes" })}>
          <Icon name="people" /><span class="label">Shared Notes</span>
          <span class="dim count">{count({ kind: "shared-notes" })}</span>
        </button>
      {/if}
      {#each sharedRoots as nb (nb.id)}
        {@render notebookRow(nb, 0)}
      {/each}
    {/if}

  </div>
  <footer class="dim">
    <span class="dot {app.status}"></span>
    {app.user?.display_name} · {app.status === "online" ? "Connected" : app.status === "connecting" ? "Connecting…" : "Offline"}
  </footer>
  <!-- Phone home screen toolbar, like Apple Notes' Folders screen. -->
  <div class="toolbar phone-only">
    <button class="flat icon tool" aria-label="New notebook" onclick={() => (creating = { parent: null })}><Icon name="newfolder" /></button>
    <button class="flat icon tool" aria-label="New note" onclick={() => composeNote({ kind: "all" })}><Icon name="compose" /></button>
  </div>
</nav>


{#if creating}
  <Dialog title="New Notebook" onclose={() => (creating = null)}>
    <form id="new-notebook" onsubmit={create}>
      <!-- svelte-ignore a11y_autofocus -->
      <input placeholder="Name" bind:value={name} autofocus />
    </form>
    {#snippet actions()}
      <button onclick={() => (creating = null)}>Cancel</button>
      <button class="suggested" type="submit" form="new-notebook" disabled={!name.trim()}>Create</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  nav {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--sidebar-bg);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 47px;
    padding: 0 6px 0 16px;
    padding-top: env(safe-area-inset-top);
  }

  .title {
    font-weight: 700;
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 0 6px 12px;
  }

  .row {
    width: 100%;
    justify-content: flex-start;
    gap: 10px;
    padding: 0 12px;
    min-height: 38px;
    font-weight: 400;
  }

  .row.selected {
    background: var(--active);
  }

  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
  }

  .owner,
  .count {
    font-size: 0.8rem;
  }

  .count {
    min-width: 1.5em;
    text-align: right;
  }

  .toolbar {
    display: flex;
    justify-content: space-between;
    padding: 2px 8px env(safe-area-inset-bottom);
    border-top: 1px solid var(--border);
    background: var(--headerbar-bg);
  }

  .tool {
    min-width: 44px;
    min-height: 44px;
    color: var(--accent);
  }

  .tool :global(svg) {
    width: 22px;
    height: 22px;
  }

  /* Phone: bigger rows, like the Folders list in Apple Notes. */
  @media (max-width: 700px) {
    .row {
      min-height: 48px;
      font-size: 1.05rem;
    }

    footer {
      padding-bottom: 10px;
    }
  }

  .section {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 0 4px 12px;
    font-size: 0.85rem;
    font-weight: 700;
    color: var(--dim-fg);
  }

  .empty {
    margin: 4px 12px;
    font-size: 0.9rem;
  }

  form input {
    width: 100%;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 16px;
    padding-bottom: max(10px, env(safe-area-inset-bottom));
    font-size: 0.85rem;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--dim-fg);
  }

  .dot.online {
    background: var(--success);
  }
</style>
