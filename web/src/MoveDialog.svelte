<script lang="ts">
  import { api, ApiError, type TreeNotebook } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import { app, refreshTree } from "./lib/store.svelte";
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

  async function move(target: string | null, label: string) {
    busy = true;
    try {
      if (kind === "note") await api.moveNote(id, target);
      else await api.moveNotebook(id, target);
      await refreshTree();
      toast(`Moved to ${label}`);
      onclose();
    } catch (err) {
      toast(err instanceof ApiError ? err.message : "Couldn't move it");
      busy = false;
    }
  }

  const topLabel = $derived(kind === "note" ? "No Notebook" : "Top Level");
</script>

<Dialog title="Move “{name || 'New Note'}”" {onclose}>
  <ul class="boxed-list targets" aria-label="Destinations">
    {#if canTop}
      <li>
        <button class="flat row" disabled={busy || current === null} onclick={() => move(null, topLabel)}>
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
          onclick={() => move(nb.id, nb.name)}
        >
          <Icon name="folder" />
          <span class="label">{nb.name}</span>
          {#if current === nb.id}<span class="here"><Icon name="check" /></span>{/if}
        </button>
      </li>
    {/each}
  </ul>

  {#snippet actions()}
    <button onclick={onclose}>Cancel</button>
  {/snippet}
</Dialog>

<style>
  .targets {
    max-height: min(56vh, 480px);
    overflow-y: auto;
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
