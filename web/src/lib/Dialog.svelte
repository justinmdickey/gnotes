<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    onclose,
    children,
    actions,
  }: { title: string; onclose: () => void; children: Snippet; actions?: Snippet } = $props();
  let dialog: HTMLDialogElement;

  $effect(() => {
    dialog.showModal();
  });
</script>

<dialog bind:this={dialog} onclose={onclose} onclick={(e) => e.target === dialog && dialog.close()}>
  <div class="body">
    <h2>{title}</h2>
    {@render children()}
  </div>
  {#if actions}
    <div class="actions">{@render actions()}</div>
  {/if}
</dialog>

<style>
  dialog {
    width: min(420px, calc(100vw - 32px));
    padding: 0;
    border: none;
    border-radius: var(--radius-lg);
    background: var(--dialog-bg);
    color: var(--fg);
    box-shadow: 0 8px 32px rgb(0 0 0 / 30%);
  }

  dialog::backdrop {
    background: rgb(0 0 0 / 35%);
  }

  .body {
    padding: 24px 24px 16px;
  }

  h2 {
    margin: 0 0 16px;
    font-size: 1.2rem;
    text-align: center;
  }

  .actions {
    display: flex;
    gap: 8px;
    padding: 0 24px 24px;
  }

  .actions :global(button) {
    flex: 1;
  }
</style>
