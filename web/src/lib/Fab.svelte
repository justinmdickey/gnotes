<script lang="ts">
  import Icon, { type IconName } from "./Icon.svelte";
  import type { MenuItem } from "./Menu.svelte";
  import { fadeIn, ms } from "./ui.svelte";
  import { backOut } from "svelte/easing";

  // A floating action button that fans out labeled actions above it (a Material speed dial), for
  // a screen's own actions. It sits in the bottom right corner of its positioned parent.
  let { label, icon, items }: { label: string; icon: IconName; items: MenuItem[] } = $props();
  let open = $state(false);
  let button = $state<HTMLButtonElement>();

  function choose(item: MenuItem) {
    open = false;
    item.onselect();
  }

  function onkey(e: KeyboardEvent) {
    if (!open || e.key !== "Escape") return;
    open = false;
    button?.focus();
  }

  /** Each action rises into place a moment after the one under it. */
  function fan(_node: Element, { index }: { index: number }) {
    return {
      delay: ms(index * 30),
      duration: ms(200),
      easing: backOut,
      css: (t: number) => `opacity: ${Math.min(1, t * 1.5)}; transform: translateY(${(1 - t) * 16}px) scale(${0.85 + 0.15 * t})`,
    };
  }
</script>

<svelte:window onkeydown={onkey} />

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={() => (open = false)} transition:fadeIn={{ duration: 150 }}></div>
{/if}
<div class="fab-box">
  {#if open}
    <div class="actions" role="menu" aria-label={label}>
      {#each items as item, i (item.label)}
        <button class="action" class:destructive={item.destructive} role="menuitem" onclick={() => choose(item)}
          in:fan={{ index: items.length - 1 - i }} out:fadeIn={{ duration: 100 }}>
          {#if item.icon}<Icon name={item.icon} />{/if}<span>{item.label}</span>
        </button>
      {/each}
    </div>
  {/if}
  <button bind:this={button} class="fab" class:open title={label} aria-label={label} aria-expanded={open} aria-haspopup="menu" onclick={() => (open = !open)}>
    <Icon name={open ? "close" : icon} />
  </button>
</div>

<style>
  .scrim {
    position: absolute;
    inset: 0;
    z-index: 5;
    background: var(--fab-scrim);
  }

  .fab-box {
    position: absolute;
    right: var(--fab-inset, 16px);
    bottom: var(--fab-inset, 16px);
    z-index: 6;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 12px;
    /* Pops in when it appears; a CSS animation, so it never holds up the screen it's on leaving. */
    animation: fab-in 220ms var(--ease-spring) both;
  }

  @keyframes fab-in {
    from {
      opacity: 0;
      transform: scale(0.5);
    }
  }

  .fab :global(svg) {
    transition: transform var(--fast) var(--ease-out);
  }

  .fab.open :global(svg) {
    transform: rotate(90deg);
  }

  .actions {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 10px;
  }

  /* Labeled pills, their icons lined up over the button's. */
  .action {
    gap: 10px;
    min-height: 44px;
    padding: 0 18px 0 16px;
    border-radius: var(--radius-pill);
    background: var(--popover-bg);
    color: var(--fg);
    box-shadow: var(--shadow-md);
    font-weight: 600;
    white-space: nowrap;
    transform-origin: right center;
  }

  .action:hover {
    background: color-mix(in srgb, var(--popover-bg) 92%, var(--fg));
  }

  .action :global(svg) {
    color: var(--dim-fg);
  }

  .action.destructive,
  .action.destructive :global(svg) {
    color: var(--destructive);
  }
</style>
