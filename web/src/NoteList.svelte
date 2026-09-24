<script lang="ts">
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { api, type TreeNote } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import Menu from "./lib/Menu.svelte";
  import StatusPage from "./lib/StatusPage.svelte";
  import { app, composeNote, goBack, navigate, notesFor, openNote, parentView, trashNotebook, viewTitle } from "./lib/store.svelte";
  import { media, scrollEdge } from "./lib/ui.svelte";
  import FolderList from "./FolderList.svelte";
  import MoveDialog from "./MoveDialog.svelte";
  import ShareDialog from "./ShareDialog.svelte";

  const notebook = $derived(
    app.view.kind === "notebook" ? app.tree.notebooks.find((n) => n.id === (app.view as { id: string }).id) : undefined,
  );
  const title = $derived(viewTitle(app.view, app.tree));
  const canCreate = $derived(app.view.kind === "all" || (notebook !== undefined && notebook.role !== "viewer"));

  let query = $state("");
  /** The big title has scrolled away, so the headerbar shows the name instead. */
  let compact = $state(false);

  /** Parent notebooks, outermost first, for the path above the title. */
  const path = $derived.by(() => {
    const out = [];
    let parent = notebook?.parent_id;
    while (parent) {
      const nb = app.tree.notebooks.find((n) => n.id === parent);
      if (!nb) break;
      out.unshift(nb);
      parent = nb.parent_id;
    }
    return out;
  });
  const heroIcon = $derived(app.view.kind === "notebook" ? "folder" : app.view.kind === "shared-notes" ? "people" : "note");
  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  const subCount = $derived(notebook ? app.tree.notebooks.filter((n) => n.parent_id === notebook.id).length : 0);
  const countLabel = $derived(
    [subCount ? plural(subCount, "notebook", "notebooks") : "", plural(notesFor(app.view, app.tree).length, "note", "notes")]
      .filter(Boolean)
      .join(" · "),
  );
  const subtitle = $derived(
    notebook && notebook.role !== "owner"
      ? `${notebook.owner}'s notebook · ${notebook.role === "viewer" ? "view only" : "can edit"} · ${countLabel}`
      : notebook
        ? `Notebook · ${countLabel}`
        : countLabel,
  );
  /** Sub-notebooks, shown as folders above the notes. */
  const folders = $derived(
    notebook
      ? app.tree.notebooks
          .filter((n) => n.parent_id === notebook.id)
          .filter((n) => !query.trim() || n.name.toLowerCase().includes(query.trim().toLowerCase()))
          .sort((a, b) => a.name.localeCompare(b.name))
      : [],
  );
  const up = $derived(parentView(app.view));
  const backLabel = $derived(up ? viewTitle(up, app.tree) : "Notebooks");
  const canEdit = $derived(notebook !== undefined && notebook.role !== "viewer");
  const menuItems = $derived.by(() => {
    if (!notebook) return [];
    const nb = notebook;
    const items = [];
    if (canEdit) {
      items.push({ label: "Move to…", icon: "move" as const, onselect: () => (moving = true) });
    }
    if (nb.role === "owner") {
      items.push({ label: "Rename…", icon: "rename" as const, onselect: () => ((newName = nb.name), (renaming = true)) });
      items.push({ label: "Move to Trash", icon: "trash" as const, destructive: true, onselect: () => trashNotebook(nb.id, nb.name) });
    }
    return items;
  });
  /** Crumbs for the levels above: all of them, or first … last two when it's deep. */
  const crumbs = $derived.by(() => {
    if (path.length <= 3) return { head: path, hidden: [], tail: [] };
    return { head: path.slice(0, 1), hidden: path.slice(1, -2), tail: path.slice(-2) };
  });
  let moving = $state(false);
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
      <Icon name="back" /><span>{backLabel}</span>
    </button>
    <div class="title" class:shown={compact} aria-hidden={!compact}>
      <strong>{title}</strong>
    </div>
    {#if notebook?.role === "owner"}
      <button class="flat accent share" title="Share this notebook" onclick={() => (sharing = true)}>
        <Icon name="people" /><span>Share</span>
      </button>
    {/if}
    {#if menuItems.length}
      <Menu label="Notebook menu" items={menuItems} />
    {/if}
    {#if canCreate}
      <button class="suggested icon wide-only new" title="New note" aria-label="New note" onclick={() => composeNote()}>
        <Icon name="compose" />
      </button>
    {/if}
  </header>

  <div class="scroll" use:scrollEdge onscroll={(e) => (compact = e.currentTarget.scrollTop > 56)}>
    <!-- Where you are: the notebook's path, icon and name, big enough to notice. -->
    {#key app.view.kind === "notebook" ? app.view.id : app.view.kind}
      <div class="hero">
        {#if notebook}
          <!-- Where this notebook sits: every level above it, squeezed or folded when deep. -->
          <nav class="crumbs" class:bare={!path.length} aria-label="Notebook path">
            <button class="crumb root phone-only" class:icon-only={path.length > 1} title="All notebooks" aria-label="All notebooks" onclick={() => navigate(null)}>
              <Icon name="home" size={14} />{#if path.length <= 1}<span>Notebooks</span>{/if}
            </button>
            <span class="sep phone-only"><Icon name="next" size={12} /></span>
            {#snippet crumb(nb: { id: string; name: string })}
              <button class="crumb" title={nb.name} onclick={() => navigate({ kind: "notebook", id: nb.id })}><span>{nb.name}</span></button>
              <span class="sep"><Icon name="next" size={12} /></span>
            {/snippet}
            {#each crumbs.head as nb (nb.id)}{@render crumb(nb)}{/each}
            {#if crumbs.hidden.length}
              <Menu
                label="{crumbs.hidden.length} more levels"
                class="crumb more"
                items={crumbs.hidden.map((nb) => ({ label: nb.name, icon: "folder" as const, onselect: () => navigate({ kind: "notebook", id: nb.id }) }))}
              >
                {#snippet trigger()}<span>…</span>{/snippet}
              </Menu>
              <span class="sep"><Icon name="next" size={12} /></span>
            {/if}
            {#each crumbs.tail as nb (nb.id)}{@render crumb(nb)}{/each}
          </nav>
        {/if}
        <div class="hero-row">
          <span class="tile" class:folder={app.view.kind === "notebook"}><Icon name={heroIcon} size={22} /></span>
          <div class="hero-text">
            <h1>{title}</h1>
            <span class="dim">{subtitle}</span>
          </div>
        </div>
      </div>
    {/key}

    <label class="search">
      <Icon name="search" />
      <input type="search" placeholder="Search notes" aria-label="Search notes" bind:value={query} />
      {#if query}
        <button class="flat icon circular clear" aria-label="Clear search" onclick={() => (query = "")}><Icon name="close" /></button>
      {/if}
    </label>

    <!-- A new list for each view; only adds and removes within one view animate. -->
    {#key app.view.kind === "notebook" ? app.view.id : app.view.kind}
    {#if notebook}
      <FolderList {folders} parent={notebook.id} canCreate={canEdit && !query} />
    {/if}
    {#each groups as group (group.label)}
      <h3 class="group-title">{group.label}</h3>
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
      {#if folders.length}
        <!-- The folders above are enough; no empty state under them. -->
      {:else if query}
        <StatusPage icon="search" title="No Results" description="Nothing matches “{query}”." tone="neutral" />
      {:else if canCreate}
        <StatusPage icon="note" title="No Notes Yet" description="Notes you write here show up in this list.">
          <button class="suggested pill" onclick={() => composeNote()}><Icon name="compose" /> New Note</button>
        </StatusPage>
      {:else}
        <StatusPage icon="note" title="No Notes Yet" description="Nothing has been shared here yet." tone="neutral" />
      {/if}
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

{#if moving && notebook}
  <MoveDialog kind="notebook" id={notebook.id} name={notebook.name} onclose={() => (moving = false)} />
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

  /* The small headerbar title only appears once the big one scrolls away. */
  .headerbar .title {
    opacity: 0;
    transform: translateY(4px);
    transition:
      opacity var(--fast) ease,
      transform var(--fast) ease;
  }

  .headerbar .title.shown {
    opacity: 1;
    transform: none;
  }

  .hero {
    padding: 4px 4px 14px;
    animation: rise 260ms var(--ease-out) both;
  }

  .crumbs {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    margin: 0 0 10px;
  }

  /* A top-level notebook on a wide screen has nothing above it. */
  .crumbs.bare {
    display: none;
  }

  .crumbs :global(.crumb) {
    flex: 0 1 auto;
    gap: 5px;
    min-width: 3.2em;
    min-height: 30px;
    padding: 0 9px;
    border-radius: var(--radius-pill);
    background: var(--button-bg);
    color: var(--fg);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .crumbs :global(.crumb span) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .crumbs :global(.crumb svg) {
    flex: none;
    color: var(--accent);
  }

  .crumbs :global(.crumb:not(.root)) {
    max-width: 10em;
  }

  /* Once there's a path, home is just its icon. */
  .crumbs :global(.crumb.root.icon-only) {
    flex: none;
    min-width: 30px;
    padding: 0 8px;
  }

  /* The nearest level, last before the title, keeps its name longest. */
  .crumbs :global(.crumb:nth-last-child(2)) {
    flex-shrink: 0.3;
  }

  .crumbs :global(.crumb.more) {
    flex: none;
    min-width: 34px;
    padding: 0 8px;
  }

  .sep {
    flex: none;
    display: flex;
    color: var(--dim-fg);
  }

  .hero-row {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .tile {
    flex: none;
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    border-radius: var(--radius-md);
    background: var(--hover);
    color: var(--dim-fg);
  }

  .tile.folder {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .hero-text {
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .hero h1 {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 800;
    line-height: 1.2;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hero-text span {
    font-size: var(--text-sm);
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
    font-size: var(--text-sm);
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

    .crumbs.bare {
      display: flex;
    }

    .share span {
      display: none;
    }

    .note {
      padding: 13px 16px;
    }
    .note-title {
      font-size: var(--text-lg);
    }

    .note.selected {
      background: transparent;
      box-shadow: none;
    }
  }
</style>
