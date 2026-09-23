<script lang="ts" module>
  export interface MenuItem {
    label: string;
    destructive?: boolean;
    onselect: () => void;
  }
</script>

<script lang="ts">
  import Icon from "./Icon.svelte";

  let { label, items }: { label: string; items: MenuItem[] } = $props();
  let open = $state(false);
</script>

<!-- A GTK-style popover menu behind a "⋯" button. -->
<div class="wrap">
  <button class="flat icon" title={label} aria-label={label} aria-expanded={open} onclick={() => (open = !open)}>
    <Icon name="more" />
  </button>
  {#if open}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="scrim" onclick={() => (open = false)}></div>
    <div class="popover" role="menu">
      {#each items as item (item.label)}
        <button class="flat" class:destructive={item.destructive} role="menuitem" onclick={() => ((open = false), item.onselect())}>
          {item.label}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
  }

  .scrim {
    position: fixed;
    inset: 0;
    z-index: 9;
  }

  .popover {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    z-index: 10;
    display: flex;
    flex-direction: column;
    min-width: 190px;
    padding: 6px;
    border-radius: var(--radius-lg);
    background: var(--dialog-bg);
    box-shadow: 0 2px 12px rgb(0 0 0 / 25%), 0 0 0 1px var(--border);
  }

  .popover button {
    justify-content: flex-start;
    min-height: 40px;
    font-weight: 400;
  }
</style>
