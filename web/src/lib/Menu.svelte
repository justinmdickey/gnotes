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
  import { untrack, type Snippet } from "svelte";
  import Dialog from "./Dialog.svelte";
  import Icon, { type IconName } from "./Icon.svelte";
  import { media, pop, portal } from "./ui.svelte";

  let {
    label,
    items,
    trigger,
    class: buttonClass = "flat icon",
    point,
    onclose,
  }: {
    label: string;
    items: MenuItem[];
    trigger?: Snippet;
    class?: string;
    /** A context menu: opens at once at this spot (a long-press or right-click), with no button of its own. */
    point?: { x: number; y: number };
    /** A context menu has closed, after any choice has run. */
    onclose?: () => void;
  } = $props();
  let open = $state(untrack(() => !!point));
  let button = $state<HTMLButtonElement>();
  /** Fixed position under the button, so the popover escapes clipped lists. */
  let at = $state<{ top?: number; bottom?: number; right?: number; left?: number }>(
    untrack(() => (point ? place(point.x, point.y, point.y, point.x) : { top: 0, right: 0 })),
  );

  /**
   * Hangs from whichever edge of the box is nearer the middle of the screen, and opens upward
   * when there isn't room below (e.g. a button on a bar at the bottom).
   */
  function place(left: number, top: number, bottom: number, right: number) {
    const height = items.length * 40 + 12;
    const vertical = bottom + 6 + height > innerHeight - 8 && top > height ? { bottom: innerHeight - top + 6 } : { top: bottom + 6 };
    return left < innerWidth / 2 ? { ...vertical, left: Math.max(8, left) } : { ...vertical, right: Math.max(8, innerWidth - right) };
  }

  function toggle() {
    const r = button!.getBoundingClientRect();
    at = place(r.left, r.top, r.bottom, r.right);
    open = !open;
  }

  function close() {
    open = false;
    onclose?.();
  }

  function choose(item: MenuItem) {
    open = false;
    item.onselect();
    onclose?.();
  }

  function onkey(e: KeyboardEvent) {
    if (open && e.key === "Escape") close();
  }
</script>

<!-- The popover is placed once, under the button; a resize would leave it behind, so close it. -->
<svelte:window onkeydown={onkey} onresize={() => open && close()} />

{#if !point}
  <button bind:this={button} class={buttonClass} class:open title={label} aria-label={label} aria-expanded={open} onclick={toggle}>
    {#if trigger}{@render trigger()}{:else}<Icon name="more" />{/if}
  </button>
{/if}

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
  <Dialog title={point ? label : undefined} onclose={close}>
    <div class="sheet" role="menu" aria-label={label}>{@render list()}</div>
    {#snippet actions()}
      <button onclick={close}>Cancel</button>
    {/snippet}
  </Dialog>
{:else if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" use:portal onclick={close} oncontextmenu={(e) => (e.preventDefault(), close())}></div>
  <div class="popover" use:portal role="menu" aria-label={label} class:from-left={at.left !== undefined} class:upward={at.bottom !== undefined} style:top={at.top !== undefined ? `${at.top}px` : null} style:bottom={at.bottom !== undefined ? `${at.bottom}px` : null} style:right={at.right !== undefined ? `${at.right}px` : null} style:left={at.left !== undefined ? `${at.left}px` : null} transition:pop>
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
