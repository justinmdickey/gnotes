<script lang="ts">
  import { api, ApiError, type TreeNotebook } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import { app, moveTo, refreshTree } from "./lib/store.svelte";
  import { toast } from "./lib/ui.svelte";

  let {
    kind,
    id,
    name,
    onclose,
  }: { kind: "note" | "notebook"; id: string; name: string; onclose: () => void } = $props();

  const item = $derived(
    kind === "note" ? app.tree.notes.find((n) => n.id === id) : app.tree.notebooks.find((n) => n.id === id),
  );
  /** Where it lives now: a notebook id, or null at the top level. */
  const current = $derived(
    item ? ("notebook_id" in item ? item.notebook_id : (item as TreeNotebook).parent_id) : null,
  );
  // Only the owner can put things at the top level; the server refuses anyone else.
  const canTop = $derived(item?.role === "owner");

  /** A notebook can't move into itself or anything inside it. */
  const blocked = $derived.by(() => {
    const out = new Set<string>();
    if (kind !== "notebook") return out;
    out.add(id);
    let grew = true;
    while (grew) {
      grew = false;
      for (const nb of app.tree.notebooks) {
        if (nb.parent_id && out.has(nb.parent_id) && !out.has(nb.id)) (out.add(nb.id), (grew = true));
      }
    }
    return out;
  });

  /** Notebooks you can edit in the same owner's tree, flattened in tree order with their depth. */
  const targets = $derived.by(() => {
    const pool = app.tree.notebooks.filter((nb) => nb.owner === item?.owner && nb.role !== "viewer");
    const ids = new Set(pool.map((n) => n.id));
    const kids = new Map<string | null, TreeNotebook[]>();
    for (const nb of pool) {
      const parent = nb.parent_id && ids.has(nb.parent_id) ? nb.parent_id : null;
      kids.set(parent, [...(kids.get(parent) ?? []), nb]);
    }
    const out: { nb: TreeNotebook; depth: number }[] = [];
    const walk = (parent: string | null, depth: number) => {
      for (const nb of (kids.get(parent) ?? []).sort((a, b) => a.name.localeCompare(b.name))) {
        out.push({ nb, depth });
        walk(nb.id, depth + 1);
      }
    };
    walk(null, 0);
    return out;
  });

  let busy = $state(false);

  /** Moves it, with Undo in the toast like a drag-and-drop move. */
  async function move(target: string | null) {
    busy = true;
    if (await moveTo({ kind, id }, target)) onclose();
    else busy = false;
  }

  const topLabel = $derived(kind === "note" ? "No Notebook" : "Top Level");

  /** Naming a new notebook to move into, instead of picking one. */
  let naming = $state(false);
  let newName = $state("");
  /**
   * Where the new notebook goes: the top level for the owner, otherwise beside the item, in the
   * notebook it's in now, since only the owner can make notebooks at the top of their tree.
   */
  const newParent = $derived(canTop ? null : current);
  const newWhere = $derived(newParent ? app.tree.notebooks.find((n) => n.id === newParent)?.name : null);
  const canMake = $derived(canTop || app.tree.notebooks.some((n) => n.id === current && n.role !== "viewer"));

  async function createAndMove(e: SubmitEvent) {
    e.preventDefault();
    if (!newName.trim()) return;
    busy = true;
    try {
      const { id: target } = await api.createNotebook(newName.trim(), newParent);
      await refreshTree();
      await move(target);
    } catch (err) {
      toast(err instanceof ApiError ? err.message : "Couldn't make the notebook");
      busy = false;
    }
  }
</script>

<Dialog title="Move “{name || 'New Note'}”" {onclose}>
  {#if naming}
    <form id="move-new-notebook" onsubmit={createAndMove}>
      <p class="dim where">Into a new notebook{#if newWhere} inside “{newWhere}”{/if}</p>
      <!-- svelte-ignore a11y_autofocus -->
      <input placeholder="Notebook name" aria-label="Notebook name" bind:value={newName} autofocus />
    </form>
  {:else}
  <ul class="boxed-list targets" aria-label="Destinations">
    {#if canTop}
      <li>
        <button class="flat row" disabled={busy || current === null} onclick={() => move(null)}>
          <Icon name={kind === "note" ? "note" : "home"} />
          <span class="label">{topLabel}</span>
          {#if current === null}<span class="here"><Icon name="check" /></span>{/if}
        </button>
      </li>
    {/if}
    {#each targets as { nb, depth } (nb.id)}
      <li>
        <button
          class="flat row"
          style:--depth={depth}
          disabled={busy || blocked.has(nb.id) || current === nb.id}
          onclick={() => move(nb.id)}
        >
          <Icon name="folder" />
          <span class="label">{nb.name}</span>
          {#if current === nb.id}<span class="here"><Icon name="check" /></span>{/if}
        </button>
      </li>
    {/each}
  </ul>
  {#if canMake}
    <!-- Its own list under the scrolling one, so it's in reach however many notebooks there are. -->
    <ul class="boxed-list new">
      <li>
        <button class="flat row" disabled={busy} onclick={() => (naming = true)}>
          <Icon name="newfolder" />
          <span class="label">New Notebook…</span>
        </button>
      </li>
    </ul>
  {/if}
  {/if}

  {#snippet actions()}
    {#if naming}
      <button onclick={() => (naming = false)}>Back</button>
      <button class="suggested" type="submit" form="move-new-notebook" disabled={busy || !newName.trim()}>Create and Move</button>
    {:else}
      <button onclick={onclose}>Cancel</button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .targets {
    max-height: min(48vh, 420px);
    overflow-y: auto;
  }

  .new {
    margin-top: 12px;
  }

  form input {
    width: 100%;
  }

  .where {
    margin: -8px 0 14px;
    font-size: var(--text-sm);
    text-align: center;
  }

  .row {
    width: 100%;
    justify-content: flex-start;
    gap: 12px;
    min-height: 48px;
    padding: 0 14px 0 calc(14px + var(--depth, 0) * 20px);
    border-radius: 0;
    font-weight: 500;
    text-align: left;
  }

  .row :global(svg) {
    flex: none;
    color: var(--accent);
  }

  .row:disabled {
    opacity: 1;
    color: var(--dim-fg);
  }

  .row:disabled :global(svg) {
    color: var(--dim-fg);
  }

  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .here :global(svg) {
    color: var(--accent);
  }
</style>
