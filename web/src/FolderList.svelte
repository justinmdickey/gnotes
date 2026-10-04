<script lang="ts">
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import type { TreeNotebook } from "./lib/api";
  import Icon from "./lib/Icon.svelte";
  import { app, navigate, notesFor, pathOf, sharedWithLabel } from "./lib/store.svelte";
  import { media } from "./lib/ui.svelte";
  import { longPress } from "./lib/longPress";
  import RowMenu, { hasActions, type RowPress } from "./RowMenu.svelte";

  /** The notebooks in one place, top level or inside a notebook. Every level looks the same. */
  let {
    folders,
    title = "Notebooks",
    showWhere = false,
    showSharedWith = false,
  }: { folders: TreeNotebook[]; title?: string; showWhere?: boolean; showSharedWith?: boolean } = $props();

  const inside = (id: string) => app.tree.notebooks.filter((n) => n.parent_id === id).length;
  const reveal = (node: Element) => slide(node, { duration: media.reduced ? 0 : 200, easing: cubicOut });

  /** A notebook whose actions are open, from a long-press or right-click on its row. */
  let pressed = $state<RowPress | null>(null);
  function openRow(id: string, x: number, y: number) {
    if (!hasActions("notebook", id)) return false;
    pressed = { kind: "notebook", id, x, y };
    return true;
  }
</script>

{#if folders.length}
  {#if title}<h3 class="group-title">{title}</h3>{/if}
  <ul class="boxed-list folders">
    {#each folders as nb (nb.id)}
      <li transition:reveal>
        <button class="flat folder-row" onclick={() => navigate({ kind: "notebook", id: nb.id })} use:longPress={(x, y) => openRow(nb.id, x, y)}>
          <Icon name="folder" />
          <span class="name">
            {nb.name}
            {#if showSharedWith && nb.shared_with?.length}<small class="dim">{sharedWithLabel(nb.shared_with)}</small>{/if}
            {#if showWhere}<small class="dim">in {pathOf(nb.parent_id).join(" › ") || (nb.role === "owner" ? "Notes" : nb.owner)}</small>{/if}
          </span>
          {#if nb.shared}<span class="shared-badge" title="Shared"><Icon name="person" size={16} /></span>{/if}
          {#if nb.role !== "owner" && nb.parent_id === null}<span class="dim owner">{nb.owner}</span>{/if}
          <!-- Everything directly inside, notebooks and notes alike, like a file manager. -->
          <span class="dim count" title="Items inside">{inside(nb.id) + notesFor({ kind: "notebook", id: nb.id }, app.tree).length}</span>
          <span class="dim chev"><Icon name="next" /></span>
        </button>
      </li>
    {/each}
  </ul>
{/if}

{#if pressed}
  <RowMenu press={pressed} onclose={() => (pressed = null)} />
{/if}

<style>
  .folders {
    margin-bottom: 4px;
  }

  .folder-row {
    width: 100%;
    justify-content: flex-start;
    gap: 12px;
    min-height: 48px;
    padding: 0 12px 0 14px;
    border-radius: 0;
    font-weight: 600;
    text-align: left;
  }

  .folder-row:active:not(:disabled) {
    transform: none;
  }

  .folder-row > :global(svg:first-child) {
    flex: none;
    color: var(--accent);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .name small {
    display: block;
    font-size: var(--text-xs);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .owner {
    font-size: var(--text-xs);
    font-weight: 400;
  }

  .count {
    font-weight: 400;
  }

  .chev {
    display: flex;
  }

  @media (max-width: 700px) {
    .folder-row {
      min-height: 56px;
      padding: 0 14px 0 16px;
      font-size: var(--text-lg);
      font-weight: 500;
    }

    .folder-row > :global(svg:first-child) {
      width: var(--icon-touch);
      height: var(--icon-touch);
    }
  }
</style>
