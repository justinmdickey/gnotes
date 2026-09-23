<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "./lib/api";
  import { app, readHash, startSession } from "./lib/store.svelte";
  import Editor from "./Editor.svelte";
  import Join from "./Join.svelte";
  import Login from "./Login.svelte";
  import NoteList from "./NoteList.svelte";
  import Sidebar from "./Sidebar.svelte";

  let checked = $state(false);
  // Invite links are real paths (/join/<token>) so they survive being texted and opened cold.
  const joinToken = location.pathname.match(/^\/join\/([A-Za-z0-9_-]+)$/)?.[1] ?? null;

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

{#if !checked}
  <div class="splash"></div>
{:else if joinToken}
  <Join token={joinToken} />
{:else if !app.user}
  <Login />
{:else}
  <div class="shell" data-pane={app.pane}>
    <aside class="pane sidebar"><Sidebar /></aside>
    <div class="pane list"><NoteList /></div>
    <main class="pane editor">
      {#if app.noteId}
        {#key app.noteId}
          <Editor noteId={app.noteId} />
        {/key}
      {:else}
        <div class="empty dim">Select or create a note</div>
      {/if}
    </main>
  </div>
{/if}

<style>
  .splash {
    height: 100%;
  }

  .shell {
    display: grid;
    grid-template-columns: 240px 320px 1fr;
    height: 100%;
    overflow: hidden;
  }

  .pane {
    min-width: 0;
    height: 100%;
    overflow: hidden;
  }

  .list {
    border-right: 1px solid var(--border);
  }

  .empty {
    display: grid;
    place-items: center;
    height: 100%;
    background: var(--view-bg);
  }

  /* Tablet: the sidebar slides over the list. */
  @media (max-width: 1000px) {
    .shell {
      grid-template-columns: 300px 1fr;
    }

    .sidebar {
      display: none;
    }

    .shell[data-pane="sidebar"] {
      grid-template-columns: 240px 300px 1fr;
    }

    .shell[data-pane="sidebar"] .sidebar {
      display: block;
    }
  }

  /* Phone: one pane at a time, like an AdwNavigationView. */
  @media (max-width: 700px) {
    .shell,
    .shell[data-pane="sidebar"] {
      grid-template-columns: 1fr;
    }

    .pane {
      display: none;
    }

    .list {
      border-right: none;
    }

    .shell[data-pane="sidebar"] .sidebar,
    .shell[data-pane="list"] .list,
    .shell[data-pane="editor"] .editor {
      display: block;
    }
  }

  /* .narrow-only: shown while the sidebar is hidden. .phone-only / .wide-only: one pane vs several. */
  @media (min-width: 1001px) {
    .shell :global(.narrow-only) {
      display: none;
    }
  }

  @media (min-width: 701px) {
    .shell :global(.phone-only) {
      display: none;
    }
  }

  @media (max-width: 700px) {
    .shell :global(.wide-only) {
      display: none;
    }
  }
</style>
