<script lang="ts">
  import { untrack } from "svelte";
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { api, ApiError, type TrashItem } from "./lib/api";
  import Icon from "./lib/Icon.svelte";
  import StatusPage from "./lib/StatusPage.svelte";
  import { app, goBack, navigate, refreshTree } from "./lib/store.svelte";
  import { media, scrollEdge, toast } from "./lib/ui.svelte";

  /** What you deleted, newest first; null until the first load. */
  let items = $state<TrashItem[] | null>(null);
  /** The big title has scrolled away, so the headerbar shows the name instead. */
  let compact = $state(false);

  async function load() {
    try {
      items = await api.trash();
    } catch {
      items ??= [];
    }
  }

  // The tree changes whenever something is deleted or restored, here or on another device.
  $effect(() => {
    void app.tree;
    untrack(load);
  });

  const label = (item: TrashItem) => item.name || (item.resource_type === "note" ? "New Note" : "Notebook");
  const DAY = 24 * 60 * 60 * 1000;
  const day = (ms: number) => new Date(ms).toLocaleDateString([], { month: "short", day: "numeric" });
  function left(item: TrashItem) {
    const days = Math.ceil((item.purge_at - Date.now()) / DAY);
    return days <= 1 ? "last day" : `${days} days left`;
  }

  /** Puts it back where it was (or at the top, if that notebook is gone too) and offers to go there. */
  async function restore(item: TrashItem) {
    try {
      await api.restore(item.resource_type, item.id);
    } catch (err) {
      toast(err instanceof ApiError ? err.message : "Couldn't reach the server");
      return;
    }
    items = items?.filter((i) => i !== item) ?? null;
    await refreshTree();
    toast(`“${label(item)}” restored`, { label: "Show", run: () => show(item) });
  }

  function show(item: TrashItem) {
    if (item.resource_type === "notebook") return navigate({ kind: "notebook", id: item.id });
    const notebook = app.tree.notes.find((n) => n.id === item.id)?.notebook_id;
    navigate(notebook ? { kind: "notebook", id: notebook } : { kind: "root" }, item.id);
  }

  const reveal = (node: Element) => slide(node, { duration: media.reduced ? 0 : 200, easing: cubicOut });
</script>

<section>
  <header class="headerbar">
    <button class="flat icon tablet-only" title="Show notebooks" aria-label="Show notebooks" onclick={() => (app.drawer = true)}>
      <Icon name="sidebar" />
    </button>
    <!-- Phones come here from the Account tab. -->
    <button class="flat icon circular back-icon phone-only" title="Back to Account" aria-label="Back to Account" onclick={goBack}>
      <Icon name="back" />
    </button>
    <div class="title" class:shown={compact} aria-hidden={!compact}>
      <strong>Trash</strong>
    </div>
  </header>

  <div class="scroll" use:scrollEdge onscroll={(e) => (compact = e.currentTarget.scrollTop > 56)}>
    <div class="hero">
      <h1>Trash</h1>
      <span class="dim">Deleted notes and notebooks stay here for 30 days</span>
    </div>

    {#if items?.length}
      <ul class="boxed-list">
        {#each items as item (item.resource_type + item.id)}
          <li transition:reveal>
            <div class="row">
              <span class="kind {item.resource_type}"><Icon name={item.resource_type === "notebook" ? "folder" : "note"} /></span>
              <span class="name">
                <span class="t">{label(item)}</span>
                <small class="dim">Deleted {day(item.deleted_at)} · {left(item)}</small>
              </span>
              <button onclick={() => restore(item)}>Restore</button>
            </div>
          </li>
        {/each}
      </ul>
    {:else if items}
      <StatusPage icon="trash" title="Trash Is Empty" description="Notes and notebooks you delete show up here, ready to restore." tone="neutral" />
    {/if}
  </div>
</section>

<style>
  section {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--view-bg);
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

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 0 12px 16px;
  }

  .hero {
    display: flex;
    flex-direction: column;
    padding: 4px 4px 14px;
    animation: rise 260ms var(--ease-out) both;
  }

  .hero h1 {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 800;
    line-height: 1.2;
  }

  .hero span {
    font-size: var(--text-sm);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 56px;
    padding: 8px 10px 8px 14px;
  }

  .kind {
    flex: none;
    display: flex;
    color: var(--dim-fg);
  }

  .kind.notebook {
    color: var(--accent);
  }

  .name {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .name .t,
  .name small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .name .t {
    font-weight: 600;
  }

  .name small {
    font-size: var(--text-xs);
  }

  .row button {
    flex: none;
  }

  @media (max-width: 700px) {
    .headerbar .title {
      align-items: center;
      text-align: center;
    }

    .scroll {
      padding: 0 16px 96px;
    }

    .row {
      min-height: 64px;
      padding-left: 16px;
    }

    .name .t {
      font-size: var(--text-lg);
      font-weight: 500;
    }

    .kind :global(svg) {
      width: var(--icon-touch);
      height: var(--icon-touch);
    }
  }
</style>
