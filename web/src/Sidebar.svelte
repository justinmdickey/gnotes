<script lang="ts">
  import type { TreeNote, TreeNotebook } from "./lib/api";
  import { untrack } from "svelte";
  import Icon, { type IconName } from "./lib/Icon.svelte";
  import Menu from "./lib/Menu.svelte";
  import NewNotebookDialog from "./NewNotebookDialog.svelte";
  import { app, canMoveTo, colorFor, composeNote, drag, moveTo, navigate, notesFor, openSettings, type View } from "./lib/store.svelte";
  import { media, scrollEdge } from "./lib/ui.svelte";

  /** Items directly inside, notebooks and notes alike, the same count the folder rows show. */
  function count(view: View) {
    const notes = notesFor(view, app.tree).length;
    if (view.kind === "root") return notes + ownRoots.length;
    if (view.kind === "shared-notes") return notes + sharedRoots.length;
    if (view.kind === "notebook") return notes + (byParent.get(view.id)?.length ?? 0);
    return notes;
  }

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

  const ROOT = "@root";
  const SHARED = "@shared";

  /**
   * Desktop: which folders show what's inside. Notebooks start folded; Notes and Shared with Me start
   * open. `flipped` holds the folders that differ from that right now; only the ones you flipped by
   * hand are remembered, per browser, so folders opened on the way to a note fold again next time.
   */
  let saved = loadFlipped();
  let flipped = $state<Set<string>>(new Set(saved));
  function loadFlipped() {
    try {
      return new Set<string>(JSON.parse(localStorage.getItem("gnotes.folds") ?? "[]"));
    } catch {
      return new Set<string>();
    }
  }
  const startsOpen = (key: string) => key === ROOT || key === SHARED;
  function folded(key: string) {
    return startsOpen(key) === flipped.has(key);
  }
  function setFolded(keys: string[], fold: boolean) {
    const change = keys.filter((k) => folded(k) !== fold);
    if (!change.length) return;
    const next = new Set(flipped);
    for (const k of change) if (!next.delete(k)) next.add(k);
    flipped = next;
  }
  function toggle(key: string) {
    setFolded([key], !folded(key));
    if (flipped.has(key)) saved.add(key);
    else saved.delete(key);
    try {
      localStorage.setItem("gnotes.folds", JSON.stringify([...saved]));
    } catch {}
  }

  /** Notebooks showing what's inside, so the headings know whether Collapse All has anything to do. */
  const openNotebooks = $derived(app.tree.notebooks.filter((nb) => !folded(nb.id)).map((nb) => nb.id));
  /** Folds every notebook under a heading, leaving the headings themselves open. */
  function collapseAll(owned: boolean) {
    const ids = app.tree.notebooks.filter((nb) => (nb.role === "owner") === owned).map((nb) => nb.id);
    setFolded(ids, true);
    for (const id of ids) saved.delete(id);
    try {
      localStorage.setItem("gnotes.folds", JSON.stringify([...saved]));
    } catch {}
  }

  // Going to a folder, or opening a note from anywhere, unfolds the way to it so its row is in view.
  // It runs when you move, and once the tree first arrives; later tree changes leave your folds alone.
  const loaded = $derived(app.tree.notebooks.length + app.tree.notes.length > 0);
  $effect(() => {
    const view = app.view;
    const noteId = app.noteId;
    if (!loaded) return;
    untrack(() => {
      const note = noteId ? app.tree.notes.find((n) => n.id === noteId) : undefined;
      let id = view.kind === "notebook" ? view.id : (note?.notebook_id ?? null);
      const keys: string[] = [];
      let top: TreeNotebook | undefined;
      while (id) {
        const nb = app.tree.notebooks.find((n) => n.id === id);
        if (!nb) break;
        keys.push(nb.id);
        top = nb;
        id = nb.parent_id;
      }
      const owner = top?.role ?? note?.role;
      if (owner) keys.push(owner === "owner" ? ROOT : SHARED);
      setFolded(keys, false);
    });
  });

  /** A folder's own notes, A–Z so rows don't jump while you type. Desktop and tablet only. */
  function notesIn(view: View): TreeNote[] {
    if (media.phone) return [];
    return notesFor(view, app.tree).toSorted((a, b) => (a.title || "New Note").localeCompare(b.title || "New Note"));
  }

  const rootNotes = $derived(notesIn({ kind: "root" }));
  const sharedNotes = $derived(notesIn({ kind: "shared-notes" }));

  /** Notes whose rows are on screen, i.e. no folder above them is folded. */
  const shownNotes = $derived.by(() => {
    const out = new Set<string>();
    const walk = (view: View, fold: string, kids: TreeNotebook[]) => {
      if (folded(fold)) return;
      for (const n of notesIn(view)) out.add(n.id);
      for (const nb of kids) walk({ kind: "notebook", id: nb.id }, nb.id, byParent.get(nb.id) ?? []);
    };
    walk({ kind: "root" }, ROOT, ownRoots);
    walk({ kind: "shared-notes" }, SHARED, sharedRoots);
    return out;
  });

  /** The open note's own row takes the highlight; its folder keeps it only when the note's row is folded away. */
  function isSelected(view: View) {
    if (app.noteId && shownNotes.has(app.noteId)) return false;
    return app.view.kind === view.kind && (view.kind !== "notebook" || (app.view as { id: string }).id === view.id);
  }

  /** Rows you can add to: your top folder and notebooks you can edit. */
  function addable(view: View) {
    if (view.kind === "root") return true;
    if (view.kind !== "notebook") return false;
    const id = view.id;
    return app.tree.notebooks.find((n) => n.id === id)?.role !== "viewer";
  }

  // Desktop drag and drop: notebooks drag from here, notes from the list, and both drop on a folder row.
  /** Where a row drops things: a notebook id, null for the top folder, undefined for neither. */
  const dropInto = (view: View) => (view.kind === "root" ? null : view.kind === "notebook" ? view.id : undefined);
  /** The row a drag is over, by its fold key, so it can light up. */
  let dropKey = $state<string | null>(null);
  let openTimer: ReturnType<typeof setTimeout> | undefined;

  function dragOver(e: DragEvent, view: View, fold: string | null) {
    const target = dropInto(view);
    if (target === undefined || !drag.item || !canMoveTo(drag.item, target)) return;
    e.preventDefault();
    e.dataTransfer!.dropEffect = "move";
    const key = target ?? ROOT;
    if (dropKey === key) return;
    dropKey = key;
    clearTimeout(openTimer);
    // Hovering a folded notebook opens it, so a drag can reach the notebooks inside.
    if (fold && folded(fold)) openTimer = setTimeout(() => setFolded([fold], false), 600);
  }

  function dragLeave(e: DragEvent) {
    const li = e.currentTarget as HTMLElement;
    if (e.relatedTarget instanceof Node && li.contains(e.relatedTarget)) return;
    dropKey = null;
    clearTimeout(openTimer);
  }

  function dropOn(e: DragEvent, view: View) {
    const target = dropInto(view);
    const item = drag.item;
    endDrag();
    if (target === undefined || !item || !canMoveTo(item, target)) return;
    e.preventDefault();
    void moveTo(item, target);
  }

  function endDrag() {
    drag.item = null;
    dropKey = null;
    clearTimeout(openTimer);
  }

  /** The notebook a New Notebook dialog is open for; `null` is the top folder. */
  let newNotebookIn = $state<string | null | undefined>(undefined);
</script>

{#snippet row(view: View, icon: IconName, label: string, owner = "", depth = 0, fold: string | null = null, shared = false, heading = false)}
  {@const canAdd = addable(view)}
  {@const movable = view.kind === "notebook" && !media.phone && app.tree.notebooks.find((n) => n.id === (view as { id: string }).id)?.role !== "viewer"}
  <li
    class:foldable={fold}
    class:addable={canAdd}
    class:heading
    class:drop={dropKey !== null && dropInto(view) !== undefined && dropKey === (dropInto(view) ?? ROOT)}
    ondragover={(e) => dragOver(e, view, fold)}
    ondragleave={dragLeave}
    ondrop={(e) => dropOn(e, view)}
  >
    <button
      class="row flat"
      class:selected={isSelected(view)}
      style:--depth={depth}
      onclick={() => navigate(view)}
      draggable={movable}
      ondragstart={(e) => {
        if (view.kind !== "notebook") return;
        drag.item = { kind: "notebook", id: view.id };
        // A private type, so the editor doesn't take the drop as text.
        e.dataTransfer?.setData("application/x-gnotes", view.id);
        if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
      }}
      ondragend={endDrag}
    >
      {@render guides(depth)}
      {#if !heading}<Icon name={icon} />{/if}
      <span class="label">{label}</span>
      {#if shared}<span class="shared-badge" title="Shared"><Icon name="person" size={14} /></span>{/if}
      {#if owner}<span class="dim owner">{owner}</span>{/if}
      {#if !heading}<span class="count">{count(view)}</span>{/if}
      <span class="chevron phone-only"><Icon name="next" /></span>
    </button>
    {#if canAdd}
      <!-- Shows in place of the count while you point at the row. -->
      <span class="adder">
        <Menu
          label="New in {label}"
          class="flat icon circular"
          items={[
            { label: "New Note", icon: "compose", onselect: () => composeNote(view) },
            { label: "New Notebook", icon: "newfolder", onselect: () => (newNotebookIn = view.kind === "notebook" ? view.id : null) },
          ]}
        >
          {#snippet trigger()}<Icon name="plus" size={14} />{/snippet}
        </Menu>
      </span>
    {/if}
    {#if heading && openNotebooks.some((id) => app.tree.notebooks.find((n) => n.id === id && (n.role === "owner") === (view.kind === "root")))}
      <button class="flat icon circular collapse" title="Collapse All" aria-label="Collapse all in {label}" onclick={() => collapseAll(view.kind === "root")}>
        <Icon name="collapse" size={14} />
      </button>
    {/if}
    {#if fold}
      <button
        class="flat icon circular fold"
        class:closed={folded(fold)}
        aria-label="{folded(fold) ? 'Show' : 'Hide'} what's in {label}"
        aria-expanded={!folded(fold)}
        onclick={() => toggle(fold)}
      >
        <Icon name="expand" size={14} />
      </button>
    {/if}
  </li>
{/snippet}

<!-- One colored line per level above a row, under each parent's icon, so deep folders read at a glance. -->
{#snippet guides(depth: number)}
  {#each { length: depth } as _, i (i)}
    <span class="guide" style:--i={i} style:--guide="var(--guide-{(i % 5) + 1})"></span>
  {/each}
{/snippet}

{#snippet noteRow(note: TreeNote, folder: View, depth: number)}
  <li>
    <button
      class="row flat note-row"
      class:selected={app.noteId === note.id}
      style:--depth={depth}
      onclick={() => navigate(folder, note.id)}
      draggable={note.role !== "viewer"}
      ondragstart={(e) => {
        drag.item = { kind: "note", id: note.id };
        e.dataTransfer?.setData("application/x-gnotes", note.id);
        if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
      }}
      ondragend={endDrag}
    >
      {@render guides(depth)}
      <Icon name="note" />
      <span class="label">{note.title || "New Note"}</span>
      {#if note.shared}<span class="shared-badge" title="Shared"><Icon name="person" size={14} /></span>{/if}
    </button>
  </li>
{/snippet}

<!-- Desktop shows the whole tree, notebooks first and then notes; the phone opens one level at a time. -->
{#snippet notebookRows(nb: TreeNotebook, depth: number)}
  {@const view = { kind: "notebook", id: nb.id } as View}
  {@const kids = byParent.get(nb.id) ?? []}
  {@const notes = notesIn(view)}
  {@render row(view, "folder", nb.name, nb.role !== "owner" ? nb.owner : "", depth, kids.length || notes.length ? nb.id : null, nb.shared)}
  {#if !folded(nb.id)}
    {#each kids as child (child.id)}
      {@render notebookRows(child, depth + 1)}
    {/each}
    {#each notes as note (note.id)}
      {@render noteRow(note, view, depth + 1)}
    {/each}
  {/if}
{/snippet}

<nav>
  <header class="headerbar">
    <div class="title"><strong class="brand"><img src="/icon.svg" alt="" width="22" height="22" />Gnotes</strong></div>
    <!-- Your avatar opens account settings, as in most phone apps. -->
    <button class="flat icon circular account" title="Account and settings" aria-label="Settings" onclick={openSettings}>
      <span class="avatar small {colorFor(app.user?.id ?? '')}">{app.user?.display_name.slice(0, 1).toUpperCase()}</span>
    </button>
  </header>

  <div class="scroll" use:scrollEdge>
    <!-- The same places as the phone's tab bar: Ask (Search without AI) and Recent, then your folders
         and what's shared with you under headings, so the tree starts at the left edge. Trash comes last. -->
    <ul class="group">
      <li>
        <button class="row flat" class:selected={isSelected({ kind: "ask" })} onclick={() => navigate({ kind: "ask" })}>
          <Icon name={app.features.ask ? "sparkle" : "search"} />
          <span class="label">{app.features.ask ? "Ask" : "Search"}</span>
        </button>
      </li>
      {@render row({ kind: "all" }, "clock", "Recent")}
    </ul>
    <ul class="group section">
      {@render row({ kind: "root" }, "home", "Notes", "", 0, ownRoots.length || rootNotes.length ? ROOT : null, false, true)}
      {#if !folded(ROOT)}
        {#each ownRoots as nb (nb.id)}
          {@render notebookRows(nb, 0)}
        {/each}
        {#each rootNotes as note (note.id)}
          {@render noteRow(note, { kind: "root" }, 0)}
        {/each}
      {/if}
    </ul>
    {#if sharedRoots.length || hasSharedNotes}
      <ul class="group section">
        {@render row({ kind: "shared-notes" }, "people", "Shared with Me", "", 0, sharedRoots.length || sharedNotes.length ? SHARED : null, false, true)}
        {#if !folded(SHARED)}
          {#each sharedRoots as nb (nb.id)}
            {@render notebookRows(nb, 0)}
          {/each}
          {#each sharedNotes as note (note.id)}
            {@render noteRow(note, { kind: "shared-notes" }, 0)}
          {/each}
        {/if}
      </ul>
    {/if}
    <ul class="group section">
      <li>
        <button class="row flat" class:selected={app.view.kind === "trash"} onclick={() => navigate({ kind: "trash" })}>
          <Icon name="trash" />
          <span class="label">Trash</span>
        </button>
      </li>
    </ul>
  </div>

  <footer class="dim" title={app.status === "online" ? "Changes sync live" : "Changes will sync when the server is back"}>
    <span class="dot {app.status}"></span>
    {app.status === "online" ? "Connected" : app.status === "connecting" ? "Connecting…" : "Offline"}
  </footer>
</nav>

{#if newNotebookIn !== undefined}
  <NewNotebookDialog parent={newNotebookIn} onclose={() => (newNotebookIn = undefined)} />
{/if}


<style>
  nav {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--sidebar-bg);
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 2px 8px 16px;
  }

  .group {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .row {
    position: relative;
    width: 100%;
    justify-content: flex-start;
    gap: 12px;
    padding: 0 10px 0 calc(12px + var(--depth, 0) * 18px);
    min-height: 40px;
    font-weight: 500;
  }

  .foldable,
  .addable {
    position: relative;
  }

  .foldable .row {
    padding-right: 38px;
  }

  .fold {
    position: absolute;
    top: 50%;
    right: 6px;
    min-width: 26px;
    min-height: 26px;
    translate: 0 -50%;
    color: var(--dim-fg);
  }

  .adder {
    position: absolute;
    top: 50%;
    right: 6px;
    translate: 0 -50%;
    opacity: 0;
    transition: opacity var(--fast) ease;
  }

  .foldable .adder {
    right: 34px;
  }

  .adder :global(button) {
    min-width: 26px;
    min-height: 26px;
    color: var(--dim-fg);
  }

  .addable:hover .adder,
  .addable:focus-within .adder,
  .adder:has(:global(.open)) {
    opacity: 1;
  }

  .addable:hover .count,
  .addable:focus-within .count,
  .addable:has(.adder :global(.open)) .count {
    visibility: hidden;
  }

  /* Touch has no hover, so the folder you're in keeps its + showing. */
  @media (hover: none) {
    .addable:has(.row.selected) .adder {
      opacity: 1;
    }

    .addable:has(.row.selected) .count {
      visibility: hidden;
    }
  }

  .fold :global(svg) {
    transition: rotate var(--fast) var(--ease-out);
  }

  .fold.closed :global(svg) {
    rotate: -90deg;
  }

  /* Notes and Shared with Me: small headings over their folders, still a place you can open and drop on. */
  .section {
    margin-top: 14px;
  }

  .heading .row {
    min-height: 30px;
    color: var(--dim-fg);
    font-size: var(--text-sm);
    font-weight: 700;
  }

  .heading .row.selected {
    background: transparent;
    color: var(--accent);
  }

  .collapse {
    position: absolute;
    top: 50%;
    right: 34px;
    min-width: 24px;
    min-height: 24px;
    translate: 0 -50%;
    color: var(--dim-fg);
  }

  .heading:has(.collapse) .adder {
    right: 62px;
  }

  .guide {
    position: absolute;
    top: 0;
    bottom: 0;
    left: calc(19px + var(--i) * 18px);
    width: 1.5px;
    border-radius: 1px;
    background: var(--guide);
    opacity: 0.6;
    pointer-events: none;
  }

  .heading .fold,
  .heading .adder :global(button) {
    min-width: 24px;
    min-height: 24px;
  }

  .row:active:not(:disabled) {
    transform: none;
  }

  .row.selected {
    background: var(--active);
    font-weight: 700;
  }

  /* Notes sit under the folders in plain weight, so the folders still read as the structure. */
  .note-row {
    font-weight: 400;
  }

  /* Icons let drags through to their row: a drop that lands on an icon never fires otherwise. */
  .row :global(svg) {
    color: var(--dim-fg);
    pointer-events: none;
  }

  .row .shared-badge :global(svg) {
    color: var(--shared) !important;
  }

  .row.selected :global(svg) {
    color: var(--accent);
  }

  .add,
  .add :global(svg) {
    color: var(--accent) !important;
  }

  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
  }

  .owner {
    font-size: var(--text-xs);
    font-weight: 400;
  }

  .count {
    min-width: 22px;
    padding: 1px 7px;
    border-radius: var(--radius-pill);
    font-size: var(--text-xs);
    font-weight: 700;
    color: var(--dim-fg);
    text-align: center;
  }

  /* A drag is over this folder: it lights up like the place you're about to be. */
  .drop .row,
  .drop .row.selected {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .drop .row :global(svg) {
    color: var(--accent);
  }

  .drop .count {
    color: var(--accent);
  }

  .row.selected .count {
    background: var(--accent-soft);
    color: var(--accent);
  }

  /* Desktop sidebar rows are inset 12px; line the headings up with their text. */
  .group-title {
    margin-left: 12px;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 18px;
    padding-bottom: max(10px, env(safe-area-inset-bottom));
    font-size: var(--text-xs);
  }

  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--dim-fg);
    transition: background var(--fast) ease;
  }

  .dot.online {
    background: var(--success-bg);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--success-bg) 20%, transparent);
  }

  .dot.connecting {
    animation: pulse 1.2s ease-in-out infinite;
  }

  .dot.offline {
    background: var(--warning-bg);
  }

  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }

  /* Phone: the home screen. Boxed groups with big rows, like a settings page. */
  @media (max-width: 700px) {
    nav {
      background: var(--window-bg);
    }

    .brand {
      font-size: var(--text-lg);
    }

    .scroll {
      padding: 4px 16px 96px;
    }

    .group {
      border-radius: var(--radius-lg);
      background: var(--card-bg);
      box-shadow: var(--shadow-sm), 0 0 0 1px var(--border);
      overflow: hidden;
    }

    .group > li + li {
      border-top: 1px solid var(--border);
    }


    .group:first-child {
      margin-top: 8px;
    }

    .row {
      min-height: 54px;
      padding-right: 8px;
      border-radius: 0;
      font-size: var(--text-lg);
    }

    .row :global(svg) {
      width: var(--icon-touch);
      height: var(--icon-touch);
      color: var(--accent);
    }

    .row.selected {
      background: transparent;
      font-weight: 500;
    }

    .row.selected .count {
      background: none;
      color: var(--dim-fg);
    }

    .count {
      font-size: var(--text-sm);
      font-weight: 400;
    }

    /* Phones add from the tab bar and the list's headerbar instead. */
    .adder {
      display: none;
    }

    .chevron {
      display: flex;
      color: var(--dim-fg);
    }

    .chevron :global(svg) {
      width: 16px !important;
      height: 16px !important;
      color: var(--dim-fg) !important;
    }

    .group-title {
      margin-left: 6px;
    }

    footer {
      position: absolute;
      left: 0;
      bottom: 0;
      padding: 0 18px calc(34px + env(safe-area-inset-bottom));
      pointer-events: none;
    }
  }
</style>
