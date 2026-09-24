<script lang="ts">
  import type { Snippet } from "svelte";
  import { fadeIn, sheet } from "./ui.svelte";

  let {
    title,
    onclose,
    children,
    actions,
    wide = false,
  }: { title?: string; onclose: () => void; children: Snippet; actions?: Snippet; wide?: boolean } = $props();
  let dialog: HTMLDialogElement;

  $effect(() => {
    dialog.showModal();
  });

  // Never close natively: the parent removes the dialog, which lets the exit animation play.
  function cancel(e: Event) {
    e.preventDefault();
    onclose();
  }
</script>

<!-- An AdwDialog: centered on wide screens, a bottom sheet on phones. -->
<dialog bind:this={dialog} oncancel={cancel}>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" transition:fadeIn onclick={onclose}></div>
  <div class="panel" class:wide transition:sheet>
    <div class="handle" aria-hidden="true"></div>
    <div class="body">
      {#if title}<h2>{title}</h2>{/if}
      {@render children()}
    </div>
    {#if actions}
      <div class="actions">{@render actions()}</div>
    {/if}
  </div>
</dialog>

<style>
  dialog {
    position: fixed;
    inset: 0;
    width: 100%;
    height: 100%;
    max-width: none;
    max-height: none;
    margin: 0;
    padding: 16px;
    border: none;
    background: transparent;
    color: var(--fg);
    overflow: hidden;
  }

  dialog[open] {
    display: grid;
    place-items: center;
  }

  dialog::backdrop {
    background: transparent;
  }

  .scrim {
    position: absolute;
    inset: 0;
    background: var(--scrim);
  }

  .panel {
    position: relative;
    display: flex;
    flex-direction: column;
    width: min(420px, 100%);
    max-height: calc(100dvh - 32px);
    border-radius: var(--radius-lg);
    background: var(--dialog-bg);
    box-shadow: var(--shadow-lg);
  }

  .panel.wide {
    width: min(480px, 100%);
  }

  .handle {
    display: none;
  }

  .body {
    overflow-y: auto;
    padding: 24px 24px 16px;
  }

  h2 {
    margin: 0 0 18px;
    font-size: var(--text-lg);
    font-weight: 800;
    text-align: center;
  }

  .actions {
    display: flex;
    gap: 10px;
    padding: 4px 24px 24px;
  }

  .actions :global(button) {
    flex: 1;
    min-height: 42px;
  }

  @media (max-width: 700px) {
    dialog[open] {
      place-items: end stretch;
      padding: 0;
    }

    .panel,
    .panel.wide {
      width: 100%;
      max-height: calc(100dvh - 48px - env(safe-area-inset-top));
      border-radius: 18px 18px 0 0;
      padding-bottom: env(safe-area-inset-bottom);
    }

    .handle {
      display: block;
      flex: none;
      width: 36px;
      height: 5px;
      margin: 8px auto 0;
      border-radius: var(--radius-sm);
      background: var(--border);
    }

    .body {
      padding: 14px 18px 12px;
    }

    .actions {
      padding: 4px 18px 16px;
    }

    .actions :global(button) {
      min-height: 48px;
    }
  }
</style>
