<script lang="ts">
  import Editor from "./Editor.svelte";
</script>

<div class="window">
  <header class="headerbar">
    <span class="title">Gnotes</span>
  </header>
  <main class="content">
    <Editor />
  </main>
</div>

<style>
  .window {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .headerbar {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 47px;
    padding: 0 16px;
    padding-top: env(safe-area-inset-top);
    background: var(--headerbar-bg);
    border-bottom: 1px solid var(--border);
  }

  .title {
    font-weight: 700;
  }

  .content {
    flex: 1;
    min-height: 0;
    display: flex;
    justify-content: center;
    background: var(--view-bg);
  }
</style>
