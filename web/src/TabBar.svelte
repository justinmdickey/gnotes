<script lang="ts">
  import Icon, { type IconName } from "./lib/Icon.svelte";
  import { app, composeNote, navigate, type View } from "./lib/store.svelte";

  /**
   * Which tab the current screen belongs to. Shared notebooks count as Shared, wherever you are in them;
   * Trash is in Notes. Account, opened from the avatar in the headerbar, isn't a tab.
   */
  const active = $derived.by(() => {
    if (app.settings) return null;
    const v = app.view;
    if (v.kind === "all") return "recent";
    if (v.kind === "ask") return "ask";
    if (v.kind === "shared-notes") return "shared";
    if (v.kind === "notebook" && app.tree.notebooks.find((n) => n.id === v.id)?.role !== "owner") return "shared";
    return "notes";
  });

  type Tab = { key: string; label: string; icon: IconName; view: View };
  const left: Tab[] = [
    { key: "notes", label: "Notes", icon: "folder", view: { kind: "root" } },
    { key: "recent", label: "Recent", icon: "clock", view: { kind: "all" } },
  ];
  // Ask is plain Search until AI search is set up.
  const right: Tab[] = $derived([
    app.features.ask
      ? { key: "ask", label: "Ask", icon: "sparkle", view: { kind: "ask" } }
      : { key: "ask", label: "Search", icon: "search", view: { kind: "ask" } },
    { key: "shared", label: "Shared", icon: "people", view: { kind: "shared-notes" } },
  ]);
</script>

<!-- The phone's one bottom bar, the same on every screen. Tapping a tab goes to its top; Notes also clears a search. -->
<nav class="tabbar" aria-label="Sections">
  {#snippet tabButton(tab: Tab)}
    <button
      class="flat tab"
      class:on={active === tab.key}
      aria-current={active === tab.key ? "page" : undefined}
      onclick={() => {
        if (tab.key === "notes") app.listReset++;
        navigate(tab.view);
      }}
    >
      <span class="ind"><Icon name={tab.icon} /></span><span>{tab.label}</span>
    </button>
  {/snippet}
  {#each left as tab (tab.key)}{@render tabButton(tab)}{/each}
  <!-- One tap, a new note, in the folder you're in. -->
  <div class="compose-slot">
    <button class="suggested compose" title="New note" aria-label="New note" onclick={() => composeNote()}>
      <Icon name="plus" size={26} />
    </button>
  </div>
  {#each right as tab (tab.key)}{@render tabButton(tab)}{/each}
</nav>

<style>
  .tabbar {
    display: flex;
    padding: 2px 6px env(safe-area-inset-bottom);
    border-top: 1px solid var(--border);
    background: var(--headerbar-bg);
  }

  .tab {
    flex: 1;
    flex-direction: column;
    gap: 3px;
    min-height: 56px;
    padding: 4px 0;
    color: var(--dim-fg);
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .tab:active:not(:disabled) {
    transform: none;
  }

  .tab :global(svg) {
    width: var(--icon-touch);
    height: var(--icon-touch);
  }

  .compose-slot {
    flex: 1;
    display: grid;
    place-items: center;
  }

  .compose {
    width: 56px;
    height: 44px;
    min-height: 0;
    padding: 0;
    border-radius: var(--radius-lg);
  }

  .compose :global(svg) {
    stroke-width: 2px;
  }

  .ind {
    display: grid;
    place-items: center;
    width: 52px;
    height: 30px;
    border-radius: var(--radius-pill);
    transition: background var(--fast) ease;
  }

  /* The current section: blue icon on a soft pill and a white label, like an Adwaita view switcher. */
  .tab.on {
    color: var(--fg);
  }

  .tab.on .ind {
    background: var(--accent-soft);
    color: var(--accent);
  }
</style>
