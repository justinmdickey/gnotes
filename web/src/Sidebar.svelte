<script lang="ts">
  import { api, type TreeNotebook } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Icon, { type IconName } from "./lib/Icon.svelte";
  import { app, colorFor, composeNote, navigate, notesFor, openSettings, type View } from "./lib/store.svelte";
  import { scrollEdge } from "./lib/ui.svelte";

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

  function isSelected(view: View) {
    return app.view.kind === view.kind && (view.kind !== "notebook" || (app.view as { id: string }).id === view.id);
  }

  function startCreate() {
    name = "";
    creating = { parent: null };
  }

  async function create(e: SubmitEvent) {
    e.preventDefault();
    if (!creating || !name.trim()) return;
    const { id } = await api.createNotebook(name.trim(), creating.parent);
    creating = null;
    navigate({ kind: "notebook", id });
  }
</script>

{#snippet row(view: View, icon: IconName, label: string, owner = "", depth = 0)}
  <li>
    <button class="row flat" class:selected={isSelected(view)} style:--depth={depth} onclick={() => navigate(view)}>
      <Icon name={icon} />
      <span class="label">{label}</span>
      {#if owner}<span class="dim owner">{owner}</span>{/if}
      <span class="count">{count(view)}</span>
      <span class="chevron phone-only"><Icon name="next" /></span>
    </button>
  </li>
{/snippet}

{#snippet notebookRows(nb: TreeNotebook, depth: number)}
  {@render row({ kind: "notebook", id: nb.id }, "folder", nb.name, nb.role !== "owner" ? nb.owner : "", depth)}
  {#each byParent.get(nb.id) ?? [] as child (child.id)}
    {@render notebookRows(child, depth + 1)}
  {/each}
{/snippet}

<nav>
  <header class="headerbar">
    <div class="title"><strong class="brand"><img src="/icon.svg" alt="" width="22" height="22" />Gnotes</strong></div>
    <!-- Your avatar opens account settings, as in most phone apps. -->
    <button class="flat icon circular account" title="Account and settings" aria-label="Settings" onclick={openSettings}>
      <span class="avatar {colorFor(app.user?.id ?? '')}">{app.user?.display_name.slice(0, 1).toUpperCase()}</span>
    </button>
  </header>

  <div class="scroll" use:scrollEdge>
    <ul class="group">
      {@render row({ kind: "all" }, "note", "All Notes")}
    </ul>

    <h3 class="section">Notebooks</h3>
    <ul class="group">
      {#each ownRoots as nb (nb.id)}
        {@render notebookRows(nb, 0)}
      {/each}
      <li>
        <button class="row flat add" onclick={startCreate}>
          <Icon name="newfolder" /><span class="label">New Notebook</span>
        </button>
      </li>
    </ul>

    {#if sharedRoots.length || hasSharedNotes}
      <h3 class="section">Shared with Me</h3>
      <ul class="group">
        {#if hasSharedNotes}
          {@render row({ kind: "shared-notes" }, "people", "Shared Notes")}
        {/if}
        {#each sharedRoots as nb (nb.id)}
          {@render notebookRows(nb, 0)}
        {/each}
      </ul>
    {/if}
  </div>

  <footer class="dim" title={app.status === "online" ? "Changes sync live" : "Changes will sync when the server is back"}>
    <span class="dot {app.status}"></span>
    {app.status === "online" ? "Connected" : app.status === "connecting" ? "Connecting…" : "Offline"}
  </footer>

  <button class="fab phone-only" onclick={() => composeNote({ kind: "all" })}><Icon name="compose" /> New Note</button>
</nav>

{#if creating}
  <Dialog title="New Notebook" onclose={() => (creating = null)}>
    <form id="new-notebook" onsubmit={create}>
      <!-- svelte-ignore a11y_autofocus -->
      <input placeholder="Notebook name" aria-label="Notebook name" bind:value={name} autofocus />
    </form>
    {#snippet actions()}
      <button onclick={() => (creating = null)}>Cancel</button>
      <button class="suggested" type="submit" form="new-notebook" disabled={!name.trim()}>Create</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  nav {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--sidebar-bg);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 1.05rem;
  }

  .avatar {
    width: 30px;
    height: 30px;
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
    width: 100%;
    justify-content: flex-start;
    gap: 12px;
    padding: 0 10px 0 calc(12px + var(--depth, 0) * 18px);
    min-height: 40px;
    font-weight: 500;
  }

  .row:active:not(:disabled) {
    transform: none;
  }

  .row.selected {
    background: var(--active);
    font-weight: 700;
  }

  .row :global(svg) {
    color: var(--dim-fg);
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
    font-size: 0.8rem;
    font-weight: 400;
  }

  .count {
    min-width: 22px;
    padding: 1px 7px;
    border-radius: 999px;
    font-size: 0.78rem;
    font-weight: 700;
    color: var(--dim-fg);
    text-align: center;
  }

  .row.selected .count {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .section {
    margin: 18px 12px 6px;
    font-size: 0.82rem;
    font-weight: 800;
    color: var(--dim-fg);
  }

  form input {
    width: 100%;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 18px;
    padding-bottom: max(10px, env(safe-area-inset-bottom));
    font-size: 0.82rem;
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
    background: var(--success);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--success) 20%, transparent);
  }

  .dot.connecting {
    animation: pulse 1.2s ease-in-out infinite;
  }

  .dot.offline {
    background: #e5a50a;
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
      font-size: 1.15rem;
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

    .group + .section {
      margin-top: 22px;
    }

    .group:first-child {
      margin-top: 8px;
    }

    .row {
      min-height: 54px;
      padding-right: 8px;
      border-radius: 0;
      font-size: 1.05rem;
    }

    .row :global(svg) {
      width: 20px;
      height: 20px;
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
      font-size: 0.9rem;
      font-weight: 400;
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

    .section {
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
