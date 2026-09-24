<script lang="ts">
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import type { TreeNotebook } from "./lib/api";
  import Icon from "./lib/Icon.svelte";
  import { app, navigate, notesFor } from "./lib/store.svelte";
  import { media } from "./lib/ui.svelte";

  /** The notebooks in one place, top level or inside a notebook. Every level looks the same. */
  let { folders, title = "Notebooks" }: { folders: TreeNotebook[]; title?: string } = $props();

  const inside = (id: string) => app.tree.notebooks.filter((n) => n.parent_id === id).length;
  const reveal = (node: Element) => slide(node, { duration: media.reduced ? 0 : 200, easing: cubicOut });
</script>

{#if folders.length}
  <h3 class="group-title">{title}</h3>
  <ul class="boxed-list folders">
    {#each folders as nb (nb.id)}
      <li transition:reveal>
        <button class="flat folder-row" onclick={() => navigate({ kind: "notebook", id: nb.id })}>
          <Icon name="folder" />
          <span class="name">{nb.name}</span>
          {#if nb.role !== "owner" && nb.parent_id === null}<span class="dim owner">{nb.owner}</span>{/if}
          <!-- Everything directly inside, notebooks and notes alike, like a file manager. -->
          <span class="dim count" title="Items inside">{inside(nb.id) + notesFor({ kind: "notebook", id: nb.id }, app.tree).length}</span>
          <span class="dim chev"><Icon name="next" /></span>
        </button>
      </li>
    {/each}
  </ul>
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
