<script lang="ts">
  import { app, colorFor, openSettings } from "./store.svelte";

  let { class: className = "" }: { class?: string } = $props();
</script>

<!-- Your avatar, at the right of a phone tab's headerbar: it opens Account and settings, as in most phone apps.
     Sized like the other headerbar tools: an icon-sized circle on a touch-sized button. -->
<button class="flat icon circular account {className}" title="Account and settings" aria-label="Account" onclick={openSettings}>
  <span class="avatar {colorFor(app.user?.id ?? '')}">{app.user?.display_name.slice(0, 1).toUpperCase()}</span>
</button>

<style>
  .avatar {
    width: var(--icon-touch);
    height: var(--icon-touch);
    font-size: 0.66rem;
  }
</style>
