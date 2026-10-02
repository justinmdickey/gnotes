<script lang="ts">
  import Dialog from "./Dialog.svelte";
  import { dismissToast, media, rise, ui } from "./ui.svelte";
</script>

<!-- App-wide toasts and confirmations. -->
<div class="toasts" class:phone={media.phone} aria-live="polite">
  {#each ui.toasts as t (t.id)}
    <div class="toast" transition:rise>
      <span>{t.text}</span>
      {#if t.action}
        <button class="flat action" onclick={() => (dismissToast(t.id), t.action!.run())}>{t.action.label}</button>
      {/if}
    </div>
  {/each}
</div>

{#if ui.confirm}
  {@const c = ui.confirm}
  <Dialog title={c.title} onclose={() => c.resolve(false)}>
    {#if c.body}<p class="body dim">{c.body}</p>{/if}
    {#snippet actions()}
      <button onclick={() => c.resolve(false)}>Cancel</button>
      <button class={c.destructive ? "destructive-action" : "suggested"} onclick={() => c.resolve(true)}>{c.confirm}</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .toasts {
    position: fixed;
    left: 0;
    right: 0;
    bottom: calc(24px + env(safe-area-inset-bottom));
    z-index: 100;
    display: grid;
    justify-items: center;
    pointer-events: none;
  }

  /* Above the phone's bottom toolbars. */
  .toasts.phone {
    bottom: calc(76px + env(safe-area-inset-bottom));
  }

  /* While a voice memo records, toasts sit above its bar instead of on it. */
  :global(body:has(.recorder)) .toasts {
    bottom: calc(84px + env(safe-area-inset-bottom));
  }

  :global(body:has(.recorder)) .toasts.phone {
    bottom: calc(136px + env(safe-area-inset-bottom));
  }

  .toast {
    grid-area: 1 / 1;
    display: flex;
    align-items: center;
    gap: 12px;
    max-width: calc(100vw - 32px);
    min-height: 44px;
    padding: 6px 6px 6px 18px;
    border-radius: var(--radius-pill);
    background: rgb(40 40 44 / 96%);
    color: #fff;
    box-shadow: 0 6px 24px rgb(0 0 0 / 30%);
    pointer-events: auto;
    font-weight: 600;
  }

  .toast:not(:has(.action)) {
    padding-right: 18px;
  }

  .action {
    min-height: 32px;
    border-radius: var(--radius-pill);
    color: #99c1f1;
  }

  .action:hover {
    background: rgb(255 255 255 / 12%);
  }

  .body {
    margin: -6px 0 8px;
    text-align: center;
    line-height: 1.45;
  }
</style>
