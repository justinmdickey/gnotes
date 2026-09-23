<script lang="ts">
  import { api, ApiError, inviteUrl, type AdminUser, type PendingInvite } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import Menu from "./lib/Menu.svelte";
  import { app, colorFor, endSession, goBack } from "./lib/store.svelte";

  let name = $state(app.user?.display_name ?? "");
  let savedName = $state(app.user?.display_name ?? "");
  let notice = $state("");
  let error = $state("");

  let changingPassword = $state(false);
  let current = $state("");
  let next = $state("");
  let confirmNext = $state("");

  let users = $state<AdminUser[]>([]);
  let invites = $state<PendingInvite[]>([]);
  let resetting = $state<AdminUser | null>(null);
  let resetTo = $state("");
  let newInvite = $state<string | null>(null);
  const canShare = "share" in navigator;
  const canCopy = "clipboard" in navigator;
  let copied = $state(false);

  async function loadAdmin() {
    if (!app.user?.is_admin) return;
    [users, invites] = await Promise.all([api.adminUsers(), api.invites()]);
  }
  loadAdmin();

  /** Runs an action, showing its error or an optional success notice. */
  async function run(action: () => Promise<unknown>, done = "") {
    error = "";
    notice = "";
    try {
      await action();
      notice = done;
      return true;
    } catch (err) {
      error = err instanceof ApiError ? err.message : "Couldn't reach the server";
      return false;
    }
  }

  async function saveName(e: SubmitEvent) {
    e.preventDefault();
    await run(async () => {
      const user = await api.updateMe(name);
      app.user = user;
      savedName = user.display_name;
    }, "Name saved");
  }

  async function savePassword(e: SubmitEvent) {
    e.preventDefault();
    if (next !== confirmNext) {
      error = "The new passwords don't match";
      return;
    }
    if (await run(() => api.changePassword(current, next), "Password changed. Your other devices were signed out.")) {
      changingPassword = false;
      current = next = confirmNext = "";
    }
  }

  async function adminUpdate(user: AdminUser, patch: { is_admin?: boolean; disabled?: boolean }, done: string) {
    await run(() => api.adminUpdateUser(user.id, patch), done);
    await loadAdmin();
  }

  async function saveReset(e: SubmitEvent) {
    e.preventDefault();
    const user = resetting;
    if (!user) return;
    if (await run(() => api.adminResetPassword(user.id, resetTo), `${user.display_name}'s password was reset`)) {
      resetting = null;
      resetTo = "";
    }
  }

  async function invite() {
    await run(async () => {
      newInvite = inviteUrl((await api.invite()).token);
    });
    await loadAdmin();
  }

  async function sendInvite() {
    if (!newInvite) return;
    if (canShare) await navigator.share({ title: "Gnotes invite", text: `Join me on Gnotes: ${newInvite}`, url: newInvite }).catch(() => {});
    else if (canCopy) {
      await navigator.clipboard.writeText(newInvite);
      copied = true;
    }
  }

  function inviteLabel(inv: PendingInvite) {
    if (!inv.resource_id) return "New account";
    const shared =
      inv.resource_type === "notebook"
        ? app.tree.notebooks.find((n) => n.id === inv.resource_id)?.name
        : app.tree.notes.find((n) => n.id === inv.resource_id)?.title;
    return `Shares “${shared || "a note"}”`;
  }

  const day = (ms: number) => new Date(ms).toLocaleDateString([], { month: "short", day: "numeric" });
</script>

<div class="page">
  <header>
    <button class="flat back" onclick={goBack}><Icon name="back" /><span>Back</span></button>
    <span class="title">Settings</span>
    <span class="spacer"></span>
  </header>

  <div class="scroll">
    <div class="column">
      {#if notice}<p class="notice">{notice}</p>{/if}
      {#if error}<p class="error">{error}</p>{/if}

      <h2>Account</h2>
      <ul class="boxed">
        <li>
          <form class="row-form" onsubmit={saveName}>
            <label for="display-name">Name</label>
            <input id="display-name" bind:value={name} autocomplete="name" />
            {#if name.trim() && name.trim() !== savedName}<button class="suggested" type="submit">Save</button>{/if}
          </form>
        </li>
        <li>
          <span class="label">Username</span>
          <span class="dim value">@{app.user?.username}</span>
        </li>
        <li>
          <span class="label">Password</span>
          <button onclick={() => (changingPassword = true)}>Change…</button>
        </li>
      </ul>

      <h2>Devices</h2>
      <ul class="boxed">
        <li>
          <span class="label">Other devices<span class="dim sub">Sign out everywhere except here, e.g. a lost phone</span></span>
          <button onclick={() => run(api.logoutOthers, "Other devices were signed out")}>Log Out Others</button>
        </li>
        <li>
          <span class="label">This device</span>
          <button class="destructive" onclick={endSession}>Log Out</button>
        </li>
      </ul>

      {#if app.user?.is_admin}
        <h2>People</h2>
        <ul class="boxed">
          {#each users as u (u.id)}
            <li class:disabled={u.disabled}>
              <span class="avatar {colorFor(u.id)}">{u.display_name.slice(0, 1).toUpperCase()}</span>
              <span class="label">
                <span>{u.display_name}{#if u.id === app.user?.id}<span class="dim">&nbsp;(you)</span>{/if}</span>
                <span class="dim sub">{[`@${u.username}`, u.is_admin && "Admin", u.disabled && "Disabled"].filter(Boolean).join(" · ")}</span>
              </span>
              {#if u.id !== app.user?.id}
                <Menu
                  label="Manage {u.display_name}"
                  items={[
                    u.is_admin
                      ? { label: "Remove Admin", onselect: () => adminUpdate(u, { is_admin: false }, `${u.display_name} is no longer an admin`) }
                      : { label: "Make Admin", onselect: () => adminUpdate(u, { is_admin: true }, `${u.display_name} is now an admin`) },
                    { label: "Reset Password…", onselect: () => ((resetting = u), (resetTo = "")) },
                    u.disabled
                      ? { label: "Enable Account", onselect: () => adminUpdate(u, { disabled: false }, `${u.display_name} can log in again`) }
                      : { label: "Disable Account", destructive: true, onselect: () => adminUpdate(u, { disabled: true }, `${u.display_name} was signed out and can't log in`) },
                  ]}
                />
              {/if}
            </li>
          {/each}
          <li>
            {#if newInvite}
              <div class="invite">
                <span class="dim sub">Send this link. It works once and expires in 7 days.</span>
                <div class="link-row">
                  <input class="link" readonly value={newInvite} onfocus={(e) => e.currentTarget.select()} aria-label="Invite link" />
                  {#if canShare || canCopy}
                    <button class="suggested" onclick={sendInvite}>{canShare ? "Send" : copied ? "Copied" : "Copy"}</button>
                  {/if}
                </div>
              </div>
            {:else}
              <button class="flat add-person" onclick={invite}><Icon name="person_add" /> Invite Someone New</button>
            {/if}
          </li>
        </ul>

        {#if invites.length}
          <h2>Unused Invites</h2>
          <ul class="boxed">
            {#each invites as inv (inv.id)}
              <li>
                <span class="label">{inviteLabel(inv)}<span class="dim sub">Made {day(inv.created_at)} · expires {day(inv.expires_at)}</span></span>
                <button class="destructive" onclick={async () => (await run(() => api.revokeInvite(inv.id), "Invite cancelled"), await loadAdmin())}>Cancel</button>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
    </div>
  </div>
</div>

{#if changingPassword}
  <Dialog title="Change Password" onclose={() => (changingPassword = false)}>
    <form id="change-password" class="stack" onsubmit={savePassword}>
      <input type="password" placeholder="Current password" autocomplete="current-password" bind:value={current} required />
      <input type="password" placeholder="New password (8+ characters)" autocomplete="new-password" minlength="8" bind:value={next} required />
      <input type="password" placeholder="Confirm new password" autocomplete="new-password" bind:value={confirmNext} required />
      {#if error}<p class="error">{error}</p>{/if}
    </form>
    {#snippet actions()}
      <button onclick={() => (changingPassword = false)}>Cancel</button>
      <button class="suggested" type="submit" form="change-password">Change</button>
    {/snippet}
  </Dialog>
{/if}

{#if resetting}
  <Dialog title="Reset {resetting.display_name}'s Password" onclose={() => (resetting = null)}>
    <form id="reset-password" class="stack" onsubmit={saveReset}>
      <p class="dim">They'll be signed out everywhere. Tell them the new password so they can log in.</p>
      <input type="text" placeholder="New password (8+ characters)" autocomplete="off" minlength="8" bind:value={resetTo} required />
      {#if error}<p class="error">{error}</p>{/if}
    </form>
    {#snippet actions()}
      <button onclick={() => (resetting = null)}>Cancel</button>
      <button class="suggested" type="submit" form="reset-password">Reset</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--window-bg);
  }

  header {
    display: flex;
    align-items: center;
    min-height: 47px;
    padding: 0 6px;
    padding-top: env(safe-area-inset-top);
    border-bottom: 1px solid var(--border);
    background: var(--headerbar-bg);
  }

  .back {
    gap: 4px;
    padding: 0 10px 0 6px;
    color: var(--accent);
    font-weight: 400;
  }

  .title {
    font-weight: 700;
  }

  .back,
  .spacer {
    flex: 1;
    justify-content: flex-start;
  }

  .back span {
    margin-right: auto;
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
  }

  /* An AdwPreferencesPage: a centered column of boxed groups. */
  .column {
    max-width: 640px;
    margin: 0 auto;
    padding: 12px 16px calc(32px + env(safe-area-inset-bottom));
  }

  h2 {
    margin: 24px 4px 8px;
    font-size: 0.95rem;
    font-weight: 800;
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
    gap: 12px;
    min-height: 54px;
    padding: 8px 12px;
  }

  .boxed li + li {
    border-top: 1px solid var(--border);
  }

  .boxed li.disabled .label {
    opacity: 0.6;
  }

  .label {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .sub {
    font-size: 0.85rem;
  }

  .value {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .row-form {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
  }

  .row-form label {
    flex: none;
  }

  .row-form input {
    flex: 1;
    min-width: 0;
    background: transparent;
    text-align: right;
  }

  .row-form input:focus {
    background: var(--hover);
    text-align: left;
  }

  .avatar {
    flex: none;
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: 50%;
    background: var(--user-color, var(--accent));
    color: #fff;
    font-weight: 700;
  }

  .add-person {
    width: 100%;
    justify-content: flex-start;
    gap: 12px;
    color: var(--accent);
  }

  .invite {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
  }

  .link-row {
    display: flex;
    gap: 8px;
  }

  .link {
    flex: 1;
    min-width: 0;
    font-size: 0.85rem;
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .stack p {
    margin: 0;
  }

  .notice,
  .error {
    margin: 12px 4px 0;
    padding: 10px 12px;
    border-radius: var(--radius);
  }

  .notice {
    background: color-mix(in srgb, var(--success) 18%, transparent);
  }

  .error {
    background: color-mix(in srgb, var(--destructive) 15%, transparent);
    color: var(--destructive);
  }

  .stack .error {
    margin: 0;
  }
</style>
