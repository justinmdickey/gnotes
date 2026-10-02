<script lang="ts" module>
  export interface MenuItem {
    label: string;
    icon?: IconName;
    destructive?: boolean;
    /** A choice among several: shows a check when on, and a blank where the check goes when off. */
    checked?: boolean;
    onselect: () => void;
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import Dialog from "./Dialog.svelte";
  import Icon, { type IconName } from "./Icon.svelte";
  import { media, pop, portal } from "./ui.svelte";

  let {
    label,
    items,
    trigger,
    class: buttonClass = "flat icon",
  }: { label: string; items: MenuItem[]; trigger?: Snippet; class?: string } = $props();
  let open = $state(false);
  let button: HTMLButtonElement;
  /** Fixed position under the button, so the popover escapes clipped lists. */
  let at = $state<{ top?: number; bottom?: number; right?: number; left?: number }>({ top: 0, right: 0 });

  function toggle() {
    const r = button.getBoundingClientRect();
    // Hang from whichever edge of the button is nearer the middle of the screen, and open upward
    // when there isn't room below (e.g. a button on a bar at the bottom).
    const height = items.length * 40 + 12;
    const vertical = r.bottom + 6 + height > innerHeight - 8 && r.top > height ? { bottom: innerHeight - r.top + 6 } : { top: r.bottom + 6 };
    at = r.left < innerWidth / 2 ? { ...vertical, left: Math.max(8, r.left) } : { ...vertical, right: Math.max(8, innerWidth - r.right) };
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

<!-- The popover is placed once, under the button; a resize would leave it behind, so close it. -->
<svelte:window onkeydown={onkey} onresize={() => (open = false)} />

<button bind:this={button} class={buttonClass} class:open title={label} aria-label={label} aria-expanded={open} onclick={toggle}>
  {#if trigger}{@render trigger()}{:else}<Icon name="more" />{/if}
</button>

{#snippet list()}
  {#each items as item (item.label)}
    <button
      class="flat item"
      class:destructive={item.destructive}
      role={item.checked === undefined ? "menuitem" : "menuitemradio"}
      aria-checked={item.checked}
      onclick={() => choose(item)}
    >
      {#if item.checked !== undefined}
        <span class="check">{#if item.checked}<Icon name="check" />{/if}</span>
      {:else if item.icon}<Icon name={item.icon} />{/if}
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
  <div class="scrim" use:portal onclick={() => (open = false)}></div>
  <div class="popover" use:portal role="menu" class:from-left={at.left !== undefined} class:upward={at.bottom !== undefined} style:top={at.top !== undefined ? `${at.top}px` : null} style:bottom={at.bottom !== undefined ? `${at.bottom}px` : null} style:right={at.right !== undefined ? `${at.right}px` : null} style:left={at.left !== undefined ? `${at.left}px` : null} transition:pop>
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

  .popover.upward {
    transform-origin: bottom right;
  }

  .popover.upward.from-left {
    transform-origin: bottom left;
  }

  .item {
    justify-content: flex-start;
    gap: 12px;
    min-height: 38px;
    padding: 0 12px;
    font-weight: 400;
    border-radius: var(--radius-sm);
  }

  .check {
    display: flex;
    width: 16px;
    color: var(--accent);
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
