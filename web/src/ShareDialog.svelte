<script lang="ts">
  import { api, ApiError, type ShareInfo, type UserSummary } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import { app } from "./lib/store.svelte";

  let { kind, id, name, onclose }: { kind: "note" | "notebook"; id: string; name: string; onclose: () => void } =
    $props();

  let shares = $state<ShareInfo[]>([]);
  let users = $state<UserSummary[]>([]);
  let username = $state("");
  let role = $state("editor");
  let error = $state("");

  const candidates = $derived(
    users.filter((u) => u.id !== app.user?.id && !shares.some((s) => s.username === u.username)),
  );

  async function load() {
    [shares, users] = await Promise.all([api.shares(kind, id), api.users()]);
    if (!candidates.some((u) => u.username === username)) username = candidates[0]?.username ?? "";
  }
  load();

  async function add(e: SubmitEvent) {
    e.preventDefault();
    error = "";
    try {
      await api.share(kind, id, username, role);
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : "Couldn't share";
    }
  }

  async function setRole(share: ShareInfo, next: string) {
    await api.setShareRole(share.id, next);
    await load();
  }

  async function remove(share: ShareInfo) {
    await api.unshare(share.id);
    await load();
  }
</script>

<Dialog title="Share “{name || 'Untitled'}”" {onclose}>
  <ul>
    {#each shares as share (share.id)}
      <li>
        <span class="who">{share.display_name} <span class="dim">@{share.username}</span></span>
        <select value={share.role} onchange={(e) => setRole(share, e.currentTarget.value)} aria-label="Role">
          <option value="editor">Can edit</option>
          <option value="viewer">Can view</option>
        </select>
        <button class="flat destructive" onclick={() => remove(share)}>Remove</button>
      </li>
    {:else}
      <li class="dim">Only you can see this {kind}.</li>
    {/each}
  </ul>

  {#if candidates.length}
    <form onsubmit={add}>
      <select bind:value={username} aria-label="Person">
        {#each candidates as u (u.id)}
          <option value={u.username}>{u.display_name}</option>
        {/each}
      </select>
      <select bind:value={role} aria-label="Role">
        <option value="editor">Can edit</option>
        <option value="viewer">Can view</option>
      </select>
      <button class="suggested" type="submit">Share</button>
    </form>
  {:else if users.length}
    <p class="dim">Everyone on this server already has access.</p>
  {/if}
  {#if error}<p class="error">{error}</p>{/if}

  {#snippet actions()}
    <button onclick={onclose}>Done</button>
  {/snippet}
</Dialog>

<style>
  ul {
    margin: 0 0 16px;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
  }

  .who {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  form {
    display: flex;
    gap: 8px;
  }

  form select:first-child {
    flex: 1;
    min-width: 0;
  }

  .error {
    color: var(--destructive);
  }
</style>
