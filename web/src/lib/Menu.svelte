<script lang="ts" module>
  export interface MenuItem {
    label: string;
    icon?: IconName;
    destructive?: boolean;
    onselect: () => void;
  }
</script>

<script lang="ts">
  import Dialog from "./Dialog.svelte";
  import Icon, { type IconName } from "./Icon.svelte";
  import { media, pop } from "./ui.svelte";

  let { label, items }: { label: string; items: MenuItem[] } = $props();
  let open = $state(false);
  let button: HTMLButtonElement;
  /** Fixed position under the button, so the popover escapes clipped lists. */
  let at = $state({ top: 0, right: 0 });

  function toggle() {
    const r = button.getBoundingClientRect();
    at = { top: r.bottom + 6, right: Math.max(8, innerWidth - r.right) };
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

<button bind:this={button} class="flat icon" class:open title={label} aria-label={label} aria-expanded={open} onclick={toggle}>
  <Icon name="more" />
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
  <div class="popover" role="menu" style:top="{at.top}px" style:right="{at.right}px" transition:pop>
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
