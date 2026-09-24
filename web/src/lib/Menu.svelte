<script lang="ts" module>
  export interface MenuItem {
    label: string;
    icon?: IconName;
    destructive?: boolean;
    onselect: () => void;
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import Dialog from "./Dialog.svelte";
  import Icon, { type IconName } from "./Icon.svelte";
  import { media, pop } from "./ui.svelte";

  let {
    label,
    items,
    trigger,
    class: buttonClass = "flat icon",
  }: { label: string; items: MenuItem[]; trigger?: Snippet; class?: string } = $props();
  let open = $state(false);
  let button: HTMLButtonElement;
  /** Fixed position under the button, so the popover escapes clipped lists. */
  let at = $state<{ top: number; right?: number; left?: number }>({ top: 0, right: 0 });

  function toggle() {
    const r = button.getBoundingClientRect();
    // Hang from whichever edge of the button is nearer the middle of the screen.
    at =
      r.left < innerWidth / 2
        ? { top: r.bottom + 6, left: Math.max(8, r.left) }
        : { top: r.bottom + 6, right: Math.max(8, innerWidth - r.right) };
    open = !open;
  }

  function choose(item: MenuItem) {
    open = false;
    item.onselect();
  }

  function onkey(e: KeyboardEvent) {
    if (open && e.key === "Escape") open = false;
  }
</script>

<svelte:window onkeydown={onkey} />

<button bind:this={button} class={buttonClass} class:open title={label} aria-label={label} aria-expanded={open} onclick={toggle}>
  {#if trigger}{@render trigger()}{:else}<Icon name="more" />{/if}
</button>

{#snippet list()}
  {#each items as item (item.label)}
    <button class="flat item" class:destructive={item.destructive} role="menuitem" onclick={() => choose(item)}>
      {#if item.icon}<Icon name={item.icon} />{/if}
      <span>{item.label}</span>
    </button>
  {/each}
{/snippet}

{#if open && media.phone}
  <!-- Phones get an action sheet with big targets. -->
  <Dialog onclose={() => (open = false)}>
    <div class="sheet" role="menu">{@render list()}</div>
    {#snippet actions()}
      <button onclick={() => (open = false)}>Cancel</button>
    {/snippet}
  </Dialog>
{:else if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={() => (open = false)}></div>
  <div class="popover" role="menu" class:from-left={at.left !== undefined} style:top="{at.top}px" style:right={at.right !== undefined ? `${at.right}px` : null} style:left={at.left !== undefined ? `${at.left}px` : null} transition:pop>
    {@render list()}
  </div>
{/if}

<style>
  .open {
    background: var(--active);
  }

  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
  }

  .popover {
    position: fixed;
    z-index: 41;
    display: flex;
    flex-direction: column;
    min-width: 200px;
    padding: 6px;
    border-radius: var(--radius-md);
    background: var(--popover-bg);
    box-shadow: var(--shadow-lg);
    transform-origin: top right;
  }

  .popover.from-left {
    transform-origin: top left;
  }

  .item {
    justify-content: flex-start;
    gap: 12px;
    min-height: 38px;
    padding: 0 12px;
    font-weight: 400;
    border-radius: var(--radius-sm);
  }

  .item:active:not(:disabled) {
    transform: none;
  }

  .sheet {
    display: flex;
    flex-direction: column;
    margin: 0 -6px;
  }

  .sheet .item {
    min-height: 52px;
    font-size: var(--text-lg);
    border-radius: var(--radius);
  }

  .sheet .item :global(svg) {
    width: var(--icon-touch);
    height: var(--icon-touch);
  }
</style>
