<script lang="ts">
  import Icon, { type IconName } from "./lib/Icon.svelte";
  import { app, colorFor, navigate, openSettings, type View } from "./lib/store.svelte";

  /** Which tab the current screen belongs to. Shared notebooks count as Shared, wherever you are in them. */
  const active = $derived.by(() => {
    if (app.settings) return "account";
    const v = app.view;
    if (v.kind === "all") return "recent";
    if (v.kind === "shared-notes") return "shared";
    if (v.kind === "notebook" && app.tree.notebooks.find((n) => n.id === v.id)?.role !== "owner") return "shared";
    return "notes";
  });

  const tabs: { key: string; label: string; icon: IconName; view: View }[] = [
    { key: "notes", label: "Notes", icon: "folder", view: { kind: "root" } },
    { key: "recent", label: "Recent", icon: "clock", view: { kind: "all" } },
    { key: "shared", label: "Shared", icon: "people", view: { kind: "shared-notes" } },
  ];
</script>

<!-- The phone's one bottom bar, the same on every screen. Tapping a tab goes to its top. -->
<nav class="tabbar" aria-label="Sections">
  {#each tabs as tab (tab.key)}
    <button
      class="flat tab"
      class:on={active === tab.key}
      aria-current={active === tab.key ? "page" : undefined}
      onclick={() => navigate(tab.view)}
    >
      <span class="ind"><Icon name={tab.icon} /></span><span>{tab.label}</span>
    </button>
  {/each}
  <button class="flat tab" class:on={active === "account"} onclick={openSettings}>
    <span class="ind"><span class="avatar tiny {colorFor(app.user?.id ?? '')}">{app.user?.display_name.slice(0, 1).toUpperCase()}</span></span>
    <span>Account</span>
  </button>
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

  .avatar.tiny {
    width: 22px;
    height: 22px;
    font-size: 0.7rem;
  }
</style>
