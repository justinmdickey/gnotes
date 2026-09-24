<script lang="ts">
  import { api, ApiError } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import { app, navigate, refreshTree } from "./lib/store.svelte";

  /** One way to make a notebook, at the top level (`parent` null) or inside another one. */
  let { parent, onclose }: { parent: string | null; onclose: () => void } = $props();

  let name = $state("");
  let error = $state("");
  const where = $derived(parent ? app.tree.notebooks.find((n) => n.id === parent)?.name : null);

  async function create(e: SubmitEvent) {
    e.preventDefault();
    if (!name.trim()) return;
    try {
      const { id } = await api.createNotebook(name.trim(), parent);
      await refreshTree();
      onclose();
      navigate({ kind: "notebook", id });
    } catch (err) {
      error = err instanceof ApiError ? err.message : "Couldn't make the notebook";
    }
  }
</script>

<Dialog title="New Notebook" {onclose}>
  <form id="new-notebook" onsubmit={create}>
    {#if where}<p class="dim where">Inside “{where}”</p>{/if}
    <!-- svelte-ignore a11y_autofocus -->
    <input placeholder="Notebook name" aria-label="Notebook name" bind:value={name} autofocus />
    {#if error}<p class="error">{error}</p>{/if}
  </form>
  {#snippet actions()}
    <button onclick={onclose}>Cancel</button>
    <button class="suggested" type="submit" form="new-notebook" disabled={!name.trim()}>Create</button>
  {/snippet}
</Dialog>

<style>
  input {
    width: 100%;
  }

  .where {
    margin: -8px 0 14px;
    font-size: var(--text-sm);
    text-align: center;
  }

  .error {
    color: var(--destructive);
  }
</style>
