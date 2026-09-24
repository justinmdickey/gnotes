<script lang="ts">
  import type { TreeNotebook } from "./lib/api";
  import Icon, { type IconName } from "./lib/Icon.svelte";
  import { app, colorFor, navigate, notesFor, openSettings, type View } from "./lib/store.svelte";
  import { scrollEdge } from "./lib/ui.svelte";

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

  /** Desktop: notebooks whose sub-notebooks are folded away. Remembered per browser. */
  let collapsed = $state<Set<string>>(loadCollapsed());
  function loadCollapsed() {
    try {
      return new Set<string>(JSON.parse(localStorage.getItem("gnotes.collapsed") ?? "[]"));
    } catch {
      return new Set<string>();
    }
  }
  function toggle(id: string) {
    const next = new Set(collapsed);
    if (!next.delete(id)) next.add(id);
    collapsed = next;
    try {
      localStorage.setItem("gnotes.collapsed", JSON.stringify([...next]));
    } catch {}
  }

  const ROOT = "@root";
  const SHARED = "@shared";

  function isSelected(view: View) {
    return app.view.kind === view.kind && (view.kind !== "notebook" || (app.view as { id: string }).id === view.id);
  }

</script>

{#snippet row(view: View, icon: IconName, label: string, owner = "", depth = 0, fold: string | null = null, shared = false)}
  <li class:foldable={fold}>
    <button class="row flat" class:selected={isSelected(view)} style:--depth={depth} onclick={() => navigate(view)}>
      <Icon name={icon} />
      <span class="label">{label}</span>
      {#if shared}<span class="shared-badge" title="Shared"><Icon name="person" size={14} /></span>{/if}
      {#if owner}<span class="dim owner">{owner}</span>{/if}
      <span class="count">{count(view)}</span>
      <span class="chevron phone-only"><Icon name="next" /></span>
    </button>
    {#if fold}
      <button
        class="flat icon circular fold"
        class:closed={collapsed.has(fold)}
        aria-label="{collapsed.has(fold) ? 'Show' : 'Hide'} notebooks in {label}"
        aria-expanded={!collapsed.has(fold)}
        onclick={() => toggle(fold)}
      >
        <Icon name="expand" size={14} />
      </button>
    {/if}
  </li>
{/snippet}

<!-- Desktop shows the whole tree; the phone opens one level at a time. -->
{#snippet notebookRows(nb: TreeNotebook, depth: number)}
  {@const kids = byParent.get(nb.id) ?? []}
  {@render row({ kind: "notebook", id: nb.id }, "folder", nb.name, nb.role !== "owner" ? nb.owner : "", depth, kids.length ? nb.id : null, nb.shared)}
  {#if !collapsed.has(nb.id)}
    {#each kids as child (child.id)}
      {@render notebookRows(child, depth + 1)}
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
    <!-- The same places as the phone's tab bar: your folders, Recent, and what's shared with you. -->
    <ul class="group">
      {@render row({ kind: "root" }, "home", "Notes", "", 0, ownRoots.length ? ROOT : null)}
      {#if !collapsed.has(ROOT)}
        {#each ownRoots as nb (nb.id)}
          {@render notebookRows(nb, 1)}
        {/each}
      {/if}
      {@render row({ kind: "all" }, "clock", "Recent")}
      {#if sharedRoots.length || hasSharedNotes}
        {@render row({ kind: "shared-notes" }, "people", "Shared with Me", "", 0, sharedRoots.length ? SHARED : null)}
        {#if !collapsed.has(SHARED)}
          {#each sharedRoots as nb (nb.id)}
            {@render notebookRows(nb, 1)}
          {/each}
        {/if}
      {/if}
    </ul>
  </div>

  <footer class="dim" title={app.status === "online" ? "Changes sync live" : "Changes will sync when the server is back"}>
    <span class="dot {app.status}"></span>
    {app.status === "online" ? "Connected" : app.status === "connecting" ? "Connecting…" : "Offline"}
  </footer>
</nav>


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
    font-size: var(--text-lg);
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

  .foldable {
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

  .fold :global(svg) {
    transition: rotate var(--fast) var(--ease-out);
  }

  .fold.closed :global(svg) {
    rotate: -90deg;
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
