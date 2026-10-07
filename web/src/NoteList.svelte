<script lang="ts">
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { api, type TreeNote, type TreeNotebook } from "./lib/api";
  import AccountButton from "./lib/AccountButton.svelte";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import Menu from "./lib/Menu.svelte";
  import StatusPage from "./lib/StatusPage.svelte";
  import { liveSearch, matchingNotes } from "./lib/search.svelte";
  import { app, composeNote, drag, goBack, navigate, notesFor, openAsk, openNote, parentView, pathOf, sharedWithLabel, subtree, trashNotebook, viewTitle } from "./lib/store.svelte";
  import { media, scrollEdge } from "./lib/ui.svelte";
  import FolderList from "./FolderList.svelte";
  import MoveDialog from "./MoveDialog.svelte";
  import NewNotebookDialog from "./NewNotebookDialog.svelte";
  import RowMenu, { hasActions, type RowPress } from "./RowMenu.svelte";
  import ShareDialog from "./ShareDialog.svelte";
  import { longPress } from "./lib/longPress";

  const notebook = $derived(
    app.view.kind === "notebook" ? app.tree.notebooks.find((n) => n.id === (app.view as { id: string }).id) : undefined,
  );
  const title = $derived(viewTitle(app.view, app.tree));
  const canEdit = $derived(notebook !== undefined && notebook.role !== "viewer");
  /** New notes land here: the notebook you're in, or the top folder from Notes and Recent. */
  const canCreate = $derived(app.view.kind === "root" || app.view.kind === "all" || canEdit);
  const canMakeFolder = $derived(app.view.kind === "root" || canEdit);

  let query = $state("");
  $effect(() => {
    if (app.listReset) query = "";
  });
  let searchField = $state<HTMLInputElement>();
  $effect(() => {
    if (app.findNotes && searchField) {
      app.findNotes = false;
      searchField.focus();
    }
  });
  /** The big title has scrolled away, so the headerbar shows the name instead. */
  let compact = $state(false);
  /**
   * The top of each phone tab shows the app's name in the headerbar, as the sidebar does on desktop:
   * always on Notes, and on Recent and Shared until their big title scrolls away.
   */
  const brand = $derived(media.phone && !parentView(app.view) && (app.view.kind === "root" || !compact));
  /** Wider screens keep the place's name in the middle of the headerbar, as the note screen does. */
  const titleShown = $derived(compact || brand || !media.phone);

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
  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  const visible = $derived(new Set(app.tree.notebooks.map((n) => n.id)));
  const hiddenShares = $derived(new Set(app.tree.shared.filter((s) => s.hidden).map((s) => s.resource_id)));
  /** Notebooks at the top of a place: your own under Notes, other people's under Shared. */
  const tops = (owned: boolean) =>
    app.tree.notebooks.filter(
      (n) => (n.role === "owner") === owned && !(n.parent_id && visible.has(n.parent_id)) && !hiddenShares.has(n.id),
    );
  /** What you've shared with other people yourself, for the second half of Shared. */
  const byMeFolders = $derived(app.tree.notebooks.filter((n) => n.role === "owner" && n.shared));
  const byMeNotes = $derived(app.tree.notes.filter((n) => n.role === "owner" && n.shared));
  /** The folders directly in this place, before any search. */
  const children = $derived.by((): TreeNotebook[] => {
    const v = app.view;
    if (v.kind === "root") return tops(true);
    if (v.kind === "shared-notes") return [...tops(false), ...byMeFolders];
    if (v.kind === "notebook") return app.tree.notebooks.filter((n) => n.parent_id === v.id);
    return [];
  });
  const subCount = $derived(children.length);
  const noteCount = $derived(notesFor(app.view, app.tree).length + (app.view.kind === "shared-notes" ? byMeNotes.length : 0));
  /** Nothing when the place is empty: the empty state under it says so already. */
  const countLabel = $derived(
    subCount || noteCount
      ? [subCount ? plural(subCount, "notebook", "notebooks") : "", plural(noteCount, "note", "notes")].filter(Boolean).join(" · ")
      : "",
  );
  /** Who else sees your notebook: the people it, or a notebook it's inside, is shared with. */
  const sharedWith = $derived.by(() => {
    const names = new Set<string>();
    for (let nb = notebook; nb; nb = app.tree.notebooks.find((n) => n.id === nb!.parent_id)) {
      for (const name of nb.shared_with ?? []) names.add(name);
    }
    return [...names];
  });
  const shareLabel = $derived(
    !notebook
      ? ""
      : notebook.role !== "owner"
        ? `Shared by ${notebook.owner} · ${notebook.role === "viewer" ? "view only" : "can edit"}`
        : sharedWithLabel(sharedWith),
  );
  const subtitle = $derived([shareLabel, countLabel].filter(Boolean).join(" · "));
  const q = $derived(query.trim().toLowerCase());
  /**
   * What a search looks through: this folder and everything inside it. From Notes or Recent
   * that's everything you can see; from Shared, everything shared with you or by you.
   */
  const scope = $derived.by((): ((notebookId: string | null, role: string) => boolean) => {
    const v = app.view;
    if (v.kind === "notebook") {
      const ids = subtree(v.id);
      return (id) => ids.has(id);
    }
    if (v.kind === "shared-notes") {
      const ids = new Set(byMeFolders.flatMap((nb) => [...subtree(nb.id)]));
      return (id, role) => role !== "owner" || (id !== null && ids.has(id));
    }
    return () => true;
  });
  /** Shared, before a search: what's shared with you, then what you've shared, each with its notebooks and notes. */
  const sections = $derived(
    app.view.kind === "shared-notes" && !q
      ? [
          { label: "Shared with Me", folders: tops(false), notes: notesFor(app.view, app.tree), byMe: false },
          { label: "Shared by Me", folders: byMeFolders, notes: byMeNotes, byMe: true },
        ]
          .map((s) => ({ ...s, folders: s.folders.toSorted((a, b) => a.name.localeCompare(b.name)) }))
          .filter((s) => s.folders.length || s.notes.length)
      : [],
  );
  const folders = $derived(
    app.view.kind === "shared-notes" && !q
      ? []
      : (q
          ? app.tree.notebooks.filter(
              // A notebook you shared from the top of Notes has no shared parent, but still belongs in Shared.
              (n) =>
                n.id !== (notebook?.id ?? "") &&
                (app.view.kind === "shared-notes" || scope(n.parent_id, n.role)) &&
                scope(n.id, n.role) &&
                n.name.toLowerCase().includes(q),
            )
          : children
        ).toSorted((a, b) => a.name.localeCompare(b.name)),
  );
  const up = $derived(parentView(app.view));
  const searchHint = $derived(
    app.view.kind === "notebook" ? `Search in ${title}` : app.view.kind === "shared-notes" ? "Search shared" : "Search all notes",
  );
  const backLabel = $derived(up ? viewTitle(up, app.tree) : "Notes");
  /** Search results and Recent come from all over, so each row says where it lives. */
  const showWhere = $derived(!!q || app.view.kind === "all");
  function where(note: TreeNote) {
    const path = pathOf(note.notebook_id);
    if (path.length) return path.join(" › ");
    return note.role === "owner" ? "Notes" : `Shared by ${note.owner}`;
  }
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
  let creatingFolder = $state(false);
  let renaming = $state(false);
  let newName = $state("");
  let sharing = $state(false);
  /** A note whose actions are open, from a long-press or right-click on its row. */
  let pressed = $state<RowPress | null>(null);
  function openRow(id: string, x: number, y: number) {
    if (!hasActions("note", id)) return false;
    pressed = { kind: "note", id, x, y };
    return true;
  }

  /** The server's matches for `q`, once they arrive. Offline, search has titles and previews only. */
  const search = liveSearch(() => q);
  const hits = $derived(search.hits);
  /** Ask's row: first when the search reads like a question, otherwise after the results. It opens the Ask screen. */
  const asking = $derived(!!q && !!app.features.ask);
  const questionLike = $derived(/\?$|^(who|what|when|where|why|how|which|whose|is|are|was|were|do|does|did|can|could|should|would|will|have|has)\b/i.test(q));

  const notes = $derived(
    q
      ? matchingNotes(q, hits, (n) => scope(n.notebook_id, n.role) || (app.view.kind === "shared-notes" && n.shared))
      : sections.length || app.view.kind === "shared-notes"
        ? []
        : notesFor(app.view, app.tree),
  );

  /** Apple Notes-style buckets: Today, Yesterday, Previous 7 Days, Previous 30 Days, then by month. */
  const groups = $derived.by(() => {
    const now = new Date();
    const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
    const day = 24 * 60 * 60 * 1000;
    const out: { label: string; notes: TreeNote[] }[] = [];
    if (q) return notes.length ? [{ label: "Notes", notes }] : [];
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
    <div class="start">
      <!-- Just a chevron: the path under the headerbar already names where it goes. -->
      {#if up}
      <button class="flat icon circular back-icon phone-only" title="Back to {backLabel}" aria-label="Back to {backLabel}" onclick={goBack}>
        <Icon name="back" />
      </button>
      {/if}
    </div>
    <!-- Phones have no sidebar, so the top of Notes carries the app's name and icon instead. -->
    <div class="title" class:shown={titleShown} aria-hidden={!titleShown}>
      {#if brand}
        <strong class="brand"><img src="/icon.svg" alt="" width="22" height="22" />Gnotes</strong>
      {:else}
        <strong>{title}</strong>
      {/if}
    </div>
    <div class="end">
    {#if canMakeFolder}
      <button class="flat icon new-folder" title="New notebook" aria-label="New notebook" onclick={() => (creatingFolder = true)}>
        <Icon name="newfolder" />
      </button>
    {/if}
    {#if notebook?.role === "owner"}
      <button class="flat share" class:icon={media.phone} title="Share this notebook" aria-label="Share" onclick={() => (sharing = true)}>
        <Icon name="share" />{#if !media.phone}<span>Share</span>{/if}
      </button>
    {/if}
    {#if menuItems.length}
      <Menu label="Notebook menu" items={menuItems} />
    {/if}
    {#if canCreate}
      <!-- Phones make notes from the + in the tab bar. -->
      <button class="suggested icon new wide-only" title="New note" aria-label="New note" onclick={() => composeNote()}>
        <Icon name="compose" />
      </button>
    {/if}
    {#if !up}
      <!-- The top of each phone tab: your avatar opens Account, at the far right. -->
      <AccountButton class="phone-only" />
    {/if}
    </div>
  </header>

  <div class="scroll" use:scrollEdge onscroll={(e) => (compact = e.currentTarget.scrollTop > 56)}>
    <!-- Where you are: the notebook's path, icon and name, big enough to notice. -->
    {#key app.view.kind === "notebook" ? app.view.id : app.view.kind}
      <div class="hero">
        {#if notebook}
          <!-- Where this notebook sits: every level above it, squeezed or folded when deep. -->
          {@const home = notebook.role === "owner" ? { label: "Notes", icon: "home" as const, view: { kind: "root" as const } } : { label: "Shared", icon: "people" as const, view: { kind: "shared-notes" as const } }}
          <nav class="crumbs" aria-label="Notebook path">
            <button class="crumb root" class:icon-only={path.length > 1} title={home.label} aria-label={home.label} onclick={() => navigate(home.view)}>
              <Icon name={home.icon} size={14} />{#if path.length <= 1}<span>{home.label}</span>{/if}
            </button>
            <!-- Separators go between levels only; the title under the path is where it ends. -->
            {#snippet crumb(nb: { id: string; name: string })}
              <span class="sep"><Icon name="next" size={12} /></span>
              <button class="crumb" title={nb.name} onclick={() => navigate({ kind: "notebook", id: nb.id })}><span>{nb.name}</span></button>
            {/snippet}
            {#each crumbs.head as nb (nb.id)}{@render crumb(nb)}{/each}
            {#if crumbs.hidden.length}
              <span class="sep"><Icon name="next" size={12} /></span>
              <Menu
                label="{crumbs.hidden.length} more levels"
                class="crumb more"
                items={crumbs.hidden.map((nb) => ({ label: nb.name, icon: "folder" as const, onselect: () => navigate({ kind: "notebook", id: nb.id }) }))}
              >
                {#snippet trigger()}<span>…</span>{/snippet}
              </Menu>
            {/if}
            {#each crumbs.tail as nb (nb.id)}{@render crumb(nb)}{/each}
          </nav>
        {/if}
        <div class="hero-text">
          <h1>{title}</h1>
          {#if subtitle}<span class="dim">{subtitle}</span>{/if}
        </div>
      </div>
    {/key}

    <label class="search">
      <Icon name="search" />
      <input bind:this={searchField} type="search" placeholder={searchHint} aria-label={searchHint} bind:value={query} />
      {#if query}
        <button class="flat icon circular clear" aria-label="Clear search" onclick={() => (query = "")}><Icon name="close" /></button>
      {/if}
    </label>

    {#snippet noteRow(note: TreeNote, byMe = false)}
      <!-- On desktop a note drags onto a notebook in the sidebar to move there. -->
      <button
        class="flat note"
        class:selected={app.noteId === note.id}
        onclick={() => openNote(note.id)}
        use:longPress={(x, y) => openRow(note.id, x, y)}
        draggable={!media.phone && note.role !== "viewer"}
        ondragstart={(e) => {
          drag.item = { kind: "note", id: note.id };
          e.dataTransfer?.setData("application/x-gnotes", note.id);
          if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
        }}
        ondragend={() => (drag.item = null)}
      >
        <span class="note-title">
          <span class="t">{note.title || "New Note"}</span>
          {#if note.shared}<span class="shared-badge" title="Shared"><Icon name="person" size={16} /></span>{/if}
        </span>
        <span class="meta">
          <span class="time">{when(note.updated_at)}</span>
          {#if hits.get(note.id)?.snippet.some((p) => p.hit)}
            <!-- Where the words matched, which may be far down the note. -->
            <span class="dim snippet">{#each hits.get(note.id)!.snippet as part, i (i)}{#if part.hit}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}</span>
          {:else}
            <span class="dim preview">{note.preview || (note.role !== "owner" ? note.owner : "No additional text")}</span>
          {/if}
        </span>
        {#if byMe && note.shared_with?.length}
          <span class="where dim"><Icon name="people" size={12} /><span>{sharedWithLabel(note.shared_with)}</span></span>
        {:else if showWhere}
          <span class="where dim"><Icon name={note.notebook_id ? "folder" : note.role === "owner" ? "home" : "people"} size={12} /><span>{where(note)}</span></span>
        {/if}
      </button>
    {/snippet}

    {#snippet askRow(first: boolean)}
      <ul class="boxed-list ask" class:first transition:reveal>
        <li>
          <button class="flat place-row" onclick={() => openAsk(query.trim())}>
            <Icon name="sparkle" />
            <span class="name">Ask Your Notes<span class="dim sub">“{query.trim()}”</span></span>
            <span class="dim chev"><Icon name="next" /></span>
          </button>
        </li>
      </ul>
    {/snippet}

    <!-- A new list for each view; only adds and removes within one view animate. -->
    {#key app.view.kind === "notebook" ? app.view.id : app.view.kind}
    {#if asking && questionLike}{@render askRow(true)}{/if}
    <FolderList {folders} showWhere={!!q} />
    {#each sections as section (section.label)}
      <h3 class="group-title">{section.label}</h3>
      <FolderList folders={section.folders} title="" showSharedWith={section.byMe} />
      {#if section.notes.length}
        <ul class="boxed-list">
          {#each section.notes as note (note.id)}
            <li transition:reveal>{@render noteRow(note, section.byMe)}</li>
          {/each}
        </ul>
      {/if}
    {/each}
    {#each groups as group (group.label)}
      <h3 class="group-title">{group.label}</h3>
      <ul class="boxed-list">
        {#each group.notes as note (note.id)}
          <li transition:reveal>{@render noteRow(note)}</li>
        {/each}
      </ul>
    {:else}
      {#if folders.length || sections.length}
        <!-- The folders above are enough; no empty state over them. -->
      {:else if query && asking && !questionLike}
        <StatusPage icon="search" title="No Results" description="No note has the words “{query}”." tone="neutral">
          <button class="pill" onclick={() => openAsk(query.trim())}><Icon name="sparkle" />Ask Your Notes</button>
        </StatusPage>
      {:else if query}
        <StatusPage icon="search" title="No Results" description="Nothing matches “{query}”." tone="neutral" />
      {:else if app.view.kind === "shared-notes"}
        <StatusPage icon="people" title="Nothing Shared Yet" description="Notes and notebooks you share, and ones people share with you, show up here." tone="neutral" />
      {:else if notebook}
        <!-- Phones add notes with the + in the tab bar, wider screens with the pencil in the headerbar. -->
        <StatusPage
          icon="folder"
          title="Empty Notebook"
          description={!canEdit ? "Nothing has been added here yet." : media.phone ? "Tap + to add the first note." : "Add the first note with the pencil button up top."}
          tone="neutral"
        />
      {:else}
        <StatusPage icon="note" title="No Notes Yet" description={media.phone ? "Tap + to start one." : "Start one with the pencil button up top."} />
      {/if}
    {/each}
    {#if asking && !questionLike && (groups.length || folders.length)}{@render askRow(false)}{/if}
    {/key}
    {#if media.phone && app.view.kind === "root" && !q}
      <!-- Last, where the desktop sidebar keeps it too. -->
      <ul class="boxed-list trash-link">
        <li>
          <button class="flat place-row" onclick={() => navigate({ kind: "trash" })}>
            <Icon name="trash" />
            <span class="name">Trash</span>
            <span class="dim chev"><Icon name="next" /></span>
          </button>
        </li>
      </ul>
    {/if}
  </div>

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

{#if creatingFolder}
  <NewNotebookDialog parent={notebook?.id ?? null} onclose={() => (creatingFolder = false)} />
{/if}

{#if moving && notebook}
  <MoveDialog kind="notebook" id={notebook.id} name={notebook.name} onclose={() => (moving = false)} />
{/if}

{#if sharing && notebook}
  <ShareDialog kind="notebook" id={notebook.id} name={notebook.name} onclose={() => (sharing = false)} />
{/if}

{#if pressed}
  <RowMenu press={pressed} onclose={() => (pressed = null)} />
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

  /* Three columns, so the title sits in the middle of the headerbar whatever buttons are on each side. */
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

  /* On phones the small headerbar title only appears once the big one scrolls away. */
  .headerbar .title {
    align-items: center;
    text-align: center;
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
  .crumbs :global(.crumb:last-child) {
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
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-weight: 700;
  }

  .note-title .t {
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

  .preview {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Up to two lines, so the match shows even when it's mid-sentence. */
  .snippet {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
    overflow-wrap: anywhere;
  }

  /* Matched words stand out like a highlighter pen. */
  .snippet mark {
    padding: 0 2px;
    border-radius: 3px;
    background: var(--highlight-bg);
    color: var(--highlight-fg);
    box-decoration-break: clone;
  }





  form input {
    width: 100%;
  }

  /* Trash at the bottom of Notes on phones: a row like a notebook's, in a plain color since it isn't one. */
  .trash-link {
    margin-top: 20px;
  }

  .place-row {
    width: 100%;
    justify-content: flex-start;
    gap: 12px;
    min-height: 56px;
    padding: 0 14px 0 16px;
    border-radius: 0;
    font-size: var(--text-lg);
    font-weight: 500;
    text-align: left;
  }

  .place-row:active:not(:disabled) {
    transform: none;
  }

  .place-row > :global(svg:first-child) {
    flex: none;
    width: var(--icon-touch);
    height: var(--icon-touch);
    color: var(--dim-fg);
  }

  .place-row .name {
    flex: 1;
  }

  /* Ask: above the results for a question, after them otherwise. The question goes under the label. */
  .ask {
    margin-top: 20px;
  }

  .ask.first {
    margin: 2px 0 12px;
  }

  /* Sized like the note rows around it: regular text on wide screens, larger on phones. */
  .ask .place-row {
    min-height: 52px;
    font-size: var(--text-md);
  }

  .ask .name {
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: 8px 0;
  }

  .ask .sub {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
    font-weight: 400;
  }

  .place-row .chev {
    display: flex;
  }

  @media (max-width: 700px) {

    .scroll {
      padding: 0 16px 96px;
    }

    .search input {
      min-height: 42px;
    }


    .note {
      padding: 13px 16px;
    }
    .note-title {
      font-size: var(--text-lg);
    }

    .ask .place-row {
      min-height: 56px;
      font-size: var(--text-lg);
    }

    .note.selected {
      background: transparent;
      box-shadow: none;
    }
  }
</style>
