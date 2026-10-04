<script lang="ts">
  import { api, ApiError, inviteUrl, type ShareInfo, type UserSummary } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import { app, colorFor } from "./lib/store.svelte";
  import { bloom, toast } from "./lib/ui.svelte";

  let { kind, id, name, onclose }: { kind: "note" | "notebook"; id: string; name: string; onclose: () => void } =
    $props();

  let shares = $state<ShareInfo[]>([]);
  let users = $state<UserSummary[]>([]);
  let query = $state("");
  /** Role for people added from the picker. */
  let role = $state<"editor" | "viewer">("editor");
  let error = $state("");
  let invite = $state<string | null>(null);
  let loaded = $state(false);
  // Both only exist in secure contexts (HTTPS), so plain-HTTP dev falls back to a selectable link.
  const canShare = "share" in navigator;
  const canCopy = "clipboard" in navigator;

  const byUsername = $derived(new Map(users.map((u) => [u.username, u])));
  /** No one else has an account yet, so an invite is the only way to share. */
  const alone = $derived(loaded && !users.some((u) => u.id !== app.user?.id));
  const candidates = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return users.filter(
      (u) =>
        u.id !== app.user?.id &&
        !shares.some((s) => s.username === u.username) &&
        (!q || u.display_name.toLowerCase().includes(q) || u.username.includes(q)),
    );
  });

  async function load() {
    [shares, users] = await Promise.all([api.shares(kind, id), api.users()]);
    loaded = true;
  }
  load();

  async function run(action: () => Promise<unknown>) {
    error = "";
    try {
      await action();
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : "Something went wrong";
    }
  }

  async function makeInvite() {
    const created = await api.invite({ kind, id }, role);
    invite = inviteUrl(created.token);
  }

  async function sendInvite() {
    if (!invite) return;
    const text = `Join me on Gnotes to share “${name || "a note"}”: ${invite}`;
    // The share sheet and clipboard only exist over HTTPS; otherwise the link stays selectable below.
    if (canShare) {
      await navigator.share({ title: "Gnotes invite", text, url: invite }).catch(() => {});
    } else if (canCopy) {
      await navigator.clipboard.writeText(invite);
      toast("Invite link copied");
    }
  }

  const initial = (n: string) => n.slice(0, 1).toUpperCase();
</script>

<Dialog title="Share “{name || 'New Note'}”" {onclose} wide>
  <section>
    <h3 class="group-title">People with access</h3>
    <ul class="boxed">
      <li>
        <span class="avatar {colorFor(app.user?.id ?? '')}">{initial(app.user?.display_name ?? "")}</span>
        <span class="who">{app.user?.display_name} <span class="dim">(you)</span></span>
        <span class="dim role-label">Owner</span>
      </li>
      {#each shares as share (share.id)}
        <li in:bloom>
          <span class="avatar {colorFor(byUsername.get(share.username)?.id ?? share.username)}">{initial(share.display_name)}</span>
          <span class="who">{share.display_name}</span>
          <select value={share.role} onchange={(e) => run(() => api.setShareRole(share.id, e.currentTarget.value))} aria-label="Access for {share.display_name}">
            <option value="editor">Can edit</option>
            <option value="viewer">Can view</option>
          </select>
          <button class="flat icon" title="Remove {share.display_name}" aria-label="Remove {share.display_name}" onclick={() => run(() => api.unshare(share.id))}>
            <Icon name="trash" />
          </button>
        </li>
      {/each}
    </ul>
  </section>

  <section>
    <div class="add-head">
      <h3 class="group-title">Add people</h3>
      <div class="segmented" class:right={role === "viewer"} role="radiogroup" aria-label="Access for new people">
        <span class="thumb" aria-hidden="true"></span>
        <button class:on={role === "editor"} role="radio" aria-checked={role === "editor"} onclick={() => (role = "editor")}>Can edit</button>
        <button class:on={role === "viewer"} role="radio" aria-checked={role === "viewer"} onclick={() => (role = "viewer")}>Can view</button>
      </div>
    </div>
    {#if users.length > 6}
      <input class="search" type="search" placeholder="Search people" aria-label="Search people" bind:value={query} />
    {/if}
    <ul class="boxed">
      {#each candidates as u (u.id)}
        <li>
          <span class="avatar {colorFor(u.id)}">{initial(u.display_name)}</span>
          <span class="who">{u.display_name} <span class="dim">@{u.username}</span></span>
          <button class="suggested add pill" onclick={() => run(() => api.share(kind, id, u.username, role))}>Add</button>
        </li>
      {:else}
        {#if loaded}
          <li class="dim empty">
            {#if query}No one matches{:else if alone}No one else is on this server yet{:else}Everyone on this server already has access{/if}
          </li>
        {/if}
      {/each}
    </ul>
  </section>

  {#if app.user?.is_admin}
    <section>
      <h3 class="group-title">Someone new</h3>
      {#if invite}
        <p class="dim hint">Send this link. It works once and expires in 7 days.</p>
        <div class="link-row">
          <input class="link" readonly value={invite} onfocus={(e) => e.currentTarget.select()} aria-label="Invite link" />
          {#if canShare || canCopy}
            <button class="suggested" onclick={sendInvite}>{canShare ? "Send" : "Copy"}</button>
          {/if}
        </div>
      {:else}
        <!-- With no one else to add, an invite is the way to share, so it's the main action. -->
        <button class="invite" class:suggested={alone} onclick={() => run(makeInvite)}>
          <Icon name="person_add" /> Invite someone new
        </button>
      {/if}
    </section>
  {/if}

  {#if error}<p class="error">{error}</p>{/if}

  {#snippet actions()}
    <button onclick={onclose}>Done</button>
  {/snippet}
</Dialog>

<style>
  section + section {
    margin-top: 20px;
  }

  /* Inside a dialog the groups sit closer together. */
  .group-title {
    margin: 0 4px 8px;
  }

  .boxed {
    margin: 0;
    padding: 0;
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--card-bg);
    box-shadow: 0 0 0 1px var(--border);
    overflow: hidden;
  }

  .boxed li {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 52px;
    padding: 6px 8px 6px 12px;
  }

  .boxed li + li {
    border-top: 1px solid var(--border);
  }

  .who {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .role-label {
    padding-right: 8px;
  }

  .empty {
    justify-content: center;
  }

  .add-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 8px;
  }

  .add-head h3 {
    margin: 0;
  }

  /* Two equal segments with a thumb that slides between them. */
  .segmented {
    position: relative;
    display: grid;
    grid-template-columns: 1fr 1fr;
    padding: 3px;
    border-radius: var(--radius);
    background: var(--hover);
  }

  .thumb {
    position: absolute;
    top: 3px;
    bottom: 3px;
    left: 3px;
    width: calc(50% - 3px);
    border-radius: var(--radius-sm);
    background: var(--view-bg);
    box-shadow: 0 1px 3px rgb(0 0 6 / 18%);
    transition: transform 240ms var(--ease-out);
  }

  .segmented.right .thumb {
    transform: translateX(100%);
  }

  .segmented button {
    position: relative;
    min-height: 30px;
    padding: 0 12px;
    background: transparent;
    font-weight: 500;
    font-size: var(--text-sm);
  }

  .segmented button:active:not(:disabled) {
    background: transparent;
  }

  .segmented button.on {
    font-weight: 700;
  }

  .search {
    width: 100%;
    margin-bottom: 8px;
  }

  .add {
    min-height: 32px;
    padding: 0 16px;
  }

  .invite {
    width: 100%;
    min-height: 46px;
    color: var(--accent);
  }

  .invite.suggested {
    color: var(--accent-fg);
  }

  .hint {
    margin: 0 0 8px;
    font-size: var(--text-sm);
  }

  .link-row {
    display: flex;
    gap: 8px;
  }

  .link {
    flex: 1;
    min-width: 0;
    font-size: var(--text-sm);
  }

  .error {
    color: var(--destructive);
  }
</style>
