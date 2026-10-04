<script lang="ts">
  import Dialog from "./lib/Dialog.svelte";
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

  type Tab = { key: string; label: string; icon: IconName; view: View; find?: boolean };
  const left: Tab[] = [
    { key: "notes", label: "Notes", icon: "folder", view: { kind: "root" } },
    { key: "recent", label: "Recent", icon: "clock", view: { kind: "all" } },
  ];
  // Until AI search is set up, Ask's place is Search: the Notes list, with the cursor in its search field.
  const right: Tab[] = $derived([
    app.features.ask
      ? { key: "ask", label: "Ask", icon: "sparkle", view: { kind: "ask" } }
      : { key: "search", label: "Search", icon: "search", view: { kind: "root" }, find: true },
    { key: "shared", label: "Shared", icon: "people", view: { kind: "shared-notes" } },
  ]);

  // Holding the + offers what else a new note can start with: a voice memo or a photo.
  let adding = $state(false);
  let photoInput: HTMLInputElement;
  let holdTimer: ReturnType<typeof setTimeout> | undefined;
  /** The press that opened the menu may also end in a click, which mustn't make a note or pick from the menu. */
  let held = false;
  const swallow = (e: Event) => (e.preventDefault(), e.stopPropagation());

  function press(e: PointerEvent) {
    if (e.button !== 0) return;
    held = false;
    clearTimeout(holdTimer);
    holdTimer = setTimeout(hold, 450);
  }

  function hold() {
    clearTimeout(holdTimer);
    held = true;
    adding = true;
    // That click lands wherever the finger lifts, which is now on the menu.
    addEventListener("click", swallow, true);
    navigator.vibrate?.(10);
  }

  function release() {
    clearTimeout(holdTimer);
    if (held) setTimeout(() => removeEventListener("click", swallow, true), 300);
  }

  function tap() {
    if (!held) void composeNote();
  }

  /** Straight from the tap on Photo, so the browser lets the picker open; the note is made once there's a photo. */
  function pickPhoto() {
    adding = false;
    photoInput.click();
  }

  function onPhotos() {
    const files = [...(photoInput.files ?? [])];
    photoInput.value = "";
    if (files.length) void composeNote(undefined, files);
  }

  function memo() {
    adding = false;
    void composeNote(undefined, "memo");
  }
</script>

<!-- The phone's one bottom bar, the same on every screen. Tapping a tab goes to its top; Notes also clears a search. -->
<nav class="tabbar" aria-label="Sections">
  {#snippet tabButton(tab: Tab)}
    <button
      class="flat tab"
      class:on={active === tab.key}
      aria-current={active === tab.key ? "page" : undefined}
      onclick={() => {
        if (tab.key === "notes" || tab.find) app.listReset++;
        if (tab.find) app.findNotes = true;
        navigate(tab.view);
      }}
    >
      <span class="ind"><Icon name={tab.icon} /></span><span>{tab.label}</span>
    </button>
  {/snippet}
  {#each left as tab (tab.key)}{@render tabButton(tab)}{/each}
  <!-- One tap, a new note, in the folder you're in. Holding it offers a memo or photo note. -->
  <div class="compose-slot">
    <button class="suggested compose" title="New note" aria-label="New note" aria-haspopup="menu"
      onclick={tap} onpointerdown={press} onpointerup={release} onpointerleave={release} onpointercancel={release}
      oncontextmenu={(e) => (e.preventDefault(), hold())}>
      <Icon name="plus" size={26} />
    </button>
  </div>
  {#each right as tab (tab.key)}{@render tabButton(tab)}{/each}
</nav>

<input bind:this={photoInput} class="file" type="file" accept="image/*" multiple onchange={onPhotos} aria-hidden="true" tabindex="-1" />

{#if adding}
  <Dialog onclose={() => (adding = false)}>
    <div class="sheet" role="menu" aria-label="New">
      <button class="flat item" role="menuitem" onclick={() => ((adding = false), void composeNote())}><Icon name="compose" /><span>New Note</span></button>
      <button class="flat item" role="menuitem" onclick={memo}><Icon name="mic" /><span>Voice Memo</span></button>
      <button class="flat item" role="menuitem" onclick={pickPhoto}><Icon name="camera" /><span>Photo</span></button>
    </div>
    {#snippet actions()}
      <button onclick={() => (adding = false)}>Cancel</button>
    {/snippet}
  </Dialog>
{/if}

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

  .compose {
    /* A long press opens the menu, not the browser's own. */
    -webkit-touch-callout: none;
    user-select: none;
  }

  .compose :global(svg) {
    stroke-width: 2px;
  }

  .file {
    display: none;
  }

  /* The long-press menu, an action sheet like Menu's. */
  .sheet {
    display: flex;
    flex-direction: column;
    margin: 0 -6px;
  }

  .item {
    justify-content: flex-start;
    gap: 12px;
    min-height: 52px;
    padding: 0 12px;
    border-radius: var(--radius);
    font-size: var(--text-lg);
    font-weight: 400;
  }

  .item:active:not(:disabled) {
    transform: none;
  }

  .item :global(svg) {
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
</style>
