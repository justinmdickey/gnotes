<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "./lib/api";
  import Icon from "./lib/Icon.svelte";
  import Overlays from "./lib/Overlays.svelte";
  import StatusPage from "./lib/StatusPage.svelte";
  import { app, closeDrawer, composeNote, goBack, parentView, readHash, startSession } from "./lib/store.svelte";
  import { fadeIn, media, page, standalone } from "./lib/ui.svelte";
  import Editor from "./Editor.svelte";
  import Join from "./Join.svelte";
  import Login from "./Login.svelte";
  import Settings from "./Settings.svelte";
  import NoteList from "./NoteList.svelte";
  import Sidebar from "./Sidebar.svelte";
  import TabBar from "./TabBar.svelte";

  type Pane = "sidebar" | "list" | "editor";
  const order: Pane[] = ["sidebar", "list", "editor"];

  let checked = $state(false);
  // Invite links are real paths (/join/<token>) so they survive being texted and opened cold.
  const joinToken = location.pathname.match(/^\/join\/([A-Za-z0-9_-]+)$/)?.[1] ?? null;

  /** The pane we just left, so only it and the new one animate. */
  let previous = $state<Pane>(app.pane);
  let current = $state<Pane>(app.pane);
  $effect.pre(() => {
    const next = app.pane;
    if (next === current) return;
    previous = current;
    current = next;
  });

  function position(p: Pane) {
    const d = order.indexOf(p) - order.indexOf(current);
    return d === 0 ? "current" : d < 0 ? "left" : "right";
  }

  // On phones the editor keeps showing the old note while it slides away.
  let shownNote = $state<string | null>(app.noteId);
  $effect(() => {
    const id = app.noteId;
    if (id) {
      shownNote = id;
      return;
    }
    if (!media.phone) {
      shownNote = null;
      return;
    }
    const t = setTimeout(() => (shownNote = null), 380);
    return () => clearTimeout(t);
  });

  const drawerOpen = $derived(app.drawer);

  // Edge swipe back on phones, for home-screen installs where the browser has no back gesture.
  let shell = $state<HTMLDivElement>();
  let drag = $state<number | null>(null);
  $effect(() => {
    const el = shell;
    if (!el || !standalone) return;
    let startX = 0;
    let startY = 0;
    let tracking = false;
    let lastX = 0;
    let lastT = 0;
    let speed = 0;
    const start = (e: TouchEvent) => {
      const t = e.touches[0];
      const canGoBack = app.noteId !== null || parentView(app.view) !== null;
      tracking = media.phone && canGoBack && !app.settings && e.touches.length === 1 && t.clientX < 28;
      startX = lastX = t.clientX;
      startY = t.clientY;
      lastT = e.timeStamp;
      speed = 0;
    };
    const move = (e: TouchEvent) => {
      if (!tracking) return;
      const t = e.touches[0];
      const dx = t.clientX - startX;
      if (drag === null) {
        if (Math.abs(t.clientY - startY) > Math.abs(dx) && Math.abs(t.clientY - startY) > 8) return void (tracking = false);
        if (dx < 8) return;
      }
      e.preventDefault();
      speed = (t.clientX - lastX) / Math.max(1, e.timeStamp - lastT);
      lastX = t.clientX;
      lastT = e.timeStamp;
      drag = Math.max(0, dx);
    };
    const end = () => {
      if (drag !== null && (drag > innerWidth * 0.35 || speed > 0.4)) goBack();
      tracking = false;
      drag = null;
    };
    el.addEventListener("touchstart", start, { passive: true });
    el.addEventListener("touchmove", move, { passive: false });
    el.addEventListener("touchend", end);
    el.addEventListener("touchcancel", end);
    return () => {
      el.removeEventListener("touchstart", start);
      el.removeEventListener("touchmove", move);
      el.removeEventListener("touchend", end);
      el.removeEventListener("touchcancel", end);
    };
  });

  function dragStyle(p: Pane) {
    if (drag === null) return undefined;
    const d = order.indexOf(p) - order.indexOf(current);
    if (d === 0) return `transform: translateX(${drag}px)`;
    if (d === -1) return `transform: translateX(calc(-30% + ${drag * 0.3}px)); visibility: visible`;
    return undefined;
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape" && drawerOpen && !media.phone && app.drawer) closeDrawer();
  }

  onMount(() => {
    api
      .me()
      .then(startSession)
      .catch(() => {})
      .finally(() => (checked = true));
    window.addEventListener("popstate", readHash);
    return () => window.removeEventListener("popstate", readHash);
  });
</script>

<svelte:window onkeydown={onkey} />

{#if !checked}
  <div class="splash"><span class="spinner dim"></span></div>
{:else if joinToken}
  <Join token={joinToken} />
{:else if !app.user}
  <Login />
{:else}
  <div class="app-frame">
  <div class="main">
  <div class="shell" class:dragging={drag !== null} class:drawer-open={drawerOpen} bind:this={shell}>
    {#each order as p (p)}
      <div
        class="pane {p}"
        data-pos={position(p)}
        class:moving={p === current || p === previous}
        style={dragStyle(p)}
        inert={media.phone && position(p) !== "current"}
      >
        {#if p === "sidebar"}
          <Sidebar />
        {:else if p === "list"}
          <NoteList />
        {:else if shownNote}
          {#key shownNote}
            <div class="note-swap" in:fadeIn={{ duration: 160 }}>
              <Editor noteId={shownNote} />
            </div>
          {/key}
        {:else}
          <StatusPage icon="note" title="No Note Selected" description="Pick a note from the list, or start a new one." fill>
            <button class="suggested pill" onclick={() => composeNote()}><Icon name="compose" /> New Note</button>
          </StatusPage>
        {/if}
      </div>
    {/each}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="drawer-scrim" onclick={closeDrawer}></div>
  </div>
  {#if app.settings && media.phone}
    <!-- On phones Account is a tab like the others: it fills the page area and the tab bar stays. -->
    <div class="tab-page" in:fadeIn={{ duration: 140 }}>
      <Settings tab />
    </div>
  {/if}
  </div>
  {#if media.phone && !app.typing}
    <TabBar />
  {/if}
  </div>

  {#if app.settings && !media.phone}
    <div class="settings-layer" transition:page>
      <Settings />
    </div>
  {/if}
{/if}

<Overlays />

<style>
  .splash {
    display: grid;
    place-items: center;
    height: 100%;
  }

  .app-frame {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .main {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
  }

  .tab-page {
    position: absolute;
    inset: 0;
    z-index: 10;
  }

  .shell {
    position: relative;
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: grid;
    grid-template-columns: 260px 340px 1fr;
    overflow: hidden;
    background: var(--view-bg);
  }

  .pane {
    min-width: 0;
    height: 100%;
    overflow: hidden;
  }

  .list {
    border-right: 1px solid var(--border);
  }

  .note-swap {
    height: 100%;
  }

  .drawer-scrim {
    display: none;
  }





  .settings-layer {
    position: fixed;
    inset: 0;
    z-index: 30;
  }

  @media (max-width: 1000px) {
    .shell {
      grid-template-columns: 320px 1fr;
    }
  }

  /* Tablet: the sidebar is a drawer that slides over the list. */
  @media (min-width: 701px) and (max-width: 1000px) {

    .sidebar {
      position: absolute;
      inset: 0 auto 0 0;
      z-index: 21;
      width: 290px;
      transform: translateX(-100%);
      visibility: hidden;
      transition:
        transform var(--slide) var(--ease-out),
        visibility 0s linear var(--slide),
        box-shadow var(--slide) ease;
    }

    .drawer-open .sidebar {
      transform: none;
      visibility: visible;
      box-shadow: var(--shadow-lg);
      transition:
        transform var(--slide) var(--ease-out),
        visibility 0s,
        box-shadow var(--slide) ease;
    }

    .drawer-scrim {
      display: block;
      position: absolute;
      inset: 0;
      z-index: 20;
      background: var(--scrim);
      opacity: 0;
      pointer-events: none;
      transition: opacity var(--slide) ease;
    }

    .drawer-open .drawer-scrim {
      opacity: 1;
      pointer-events: auto;
    }
  }

  /*
    Phone: one pane at a time, like an AdwNavigationView. Panes to the left of the
    current one sit dimmed and slightly offset; panes to the right wait offscreen.
  */
  @media (max-width: 700px) {
    .shell {
      display: block;
    }

    .pane,
    .sidebar {
      position: absolute;
      inset: 0;
      width: auto;
      border-right: none;
      box-shadow: none;
      transform: translateX(100%);
      visibility: hidden;
      transition: none;
    }

    .pane::after {
      content: "";
      position: absolute;
      inset: 0;
      background: rgb(0 0 0 / 0%);
      pointer-events: none;
      transition: background var(--slide) ease;
    }

    .sidebar { z-index: 1; }
    .list { z-index: 2; }
    .editor { z-index: 3; }

    .pane[data-pos="left"] {
      transform: translateX(-30%);
    }

    .pane[data-pos="left"]::after {
      background: rgb(0 0 0 / 14%);
    }

    .pane.moving {
      transition:
        transform var(--slide) var(--ease-out),
        visibility 0s linear var(--slide);
    }

    .pane[data-pos="current"] {
      transform: none;
      visibility: visible;
    }

    .pane.moving[data-pos="current"] {
      box-shadow: -6px 0 24px rgb(0 0 0 / 12%);
      transition:
        transform var(--slide) var(--ease-out),
        visibility 0s;
    }

    .dragging .pane {
      transition: none !important;
    }
  }

  /* .narrow-only: shown while the sidebar is hidden. .tablet-only: drawer layout. .phone-only / .wide-only: one pane vs several. */
  @media (min-width: 1001px) {
    .shell :global(.narrow-only),
    .shell :global(.tablet-only) {
      display: none;
    }
  }

  @media (min-width: 701px) {
    :global(.phone-only) {
      display: none !important;
    }
  }

  @media (max-width: 700px) {
    .shell :global(.wide-only),
    .shell :global(.tablet-only) {
      display: none !important;
    }
  }
</style>
