<script lang="ts">
  import { api, ApiError, inviteUrl, type AdminUser, type PendingInvite, type ServiceSettings } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import Menu from "./lib/Menu.svelte";
  import ServiceForm from "./ServiceForm.svelte";
  import { app, colorFor, endSession, goBack, importable, importNotes } from "./lib/store.svelte";
  import { ask, scrollEdge, toast } from "./lib/ui.svelte";

  /** Shown as the phone's Account tab rather than a full screen with Back. */
  let { tab = false }: { tab?: boolean } = $props();

  let name = $state(app.user?.display_name ?? "");
  let savedName = $state(app.user?.display_name ?? "");
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
  let picker = $state<HTMLInputElement>();
  const canShare = "share" in navigator;
  const canCopy = "clipboard" in navigator;

  async function loadAdmin() {
    if (!app.user?.is_admin) return;
    let settings;
    [users, invites, settings] = await Promise.all([api.adminUsers(), api.invites(), api.adminSettings()]);
    whisper = settings.whisper;
    vision = settings.vision;
    summary = settings.summary;
  }

  let whisper = $state<ServiceSettings | null>(null);
  let vision = $state<ServiceSettings | null>(null);
  let summary = $state<ServiceSettings | null>(null);

  loadAdmin();

  /** Runs an action, showing its error or an optional success notice. */
  async function run(action: () => Promise<unknown>, done = "") {
    error = "";
    try {
      await action();
      if (done) toast(done);
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
      toast("Invite link copied");
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

<div class="page" class:tab>
  <header class="headerbar">
    {#if !tab}
      <div class="side"><button class="flat back" onclick={goBack}><Icon name="back" /><span>Back</span></button></div>
      <strong class="heading">Settings</strong>
      <div class="side"></div>
    {/if}
  </header>

  <div class="scroll" use:scrollEdge>
    <div class="column">
      {#if tab}
        <!-- Titled like the other tabs: a big heading at the top of the page. -->
        <div class="hero"><h1>Account</h1></div>
      {/if}
      <div class="profile">
        <span class="avatar large {colorFor(app.user?.id ?? '')}">{app.user?.display_name.slice(0, 1).toUpperCase()}</span>
        <strong>{savedName}</strong>
        <span class="dim">@{app.user?.username}{#if app.user?.is_admin}&nbsp;· Admin{/if}</span>
      </div>
      {#if error && !changingPassword && !resetting}<p class="error">{error}</p>{/if}

      <h2 class="group-title">Account</h2>
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

      <h2 class="group-title">Notes</h2>
      <ul class="boxed">
        <li>
          <span class="label">Import notes<span class="dim sub">Markdown files, or a .zip of folders from Obsidian, Bear or another app</span></span>
          <button onclick={() => picker?.click()}>Import…</button>
          <input
            bind:this={picker}
            class="picker"
            type="file"
            multiple
            accept=".md,.markdown,.txt,.zip,text/markdown,text/plain,application/zip"
            onchange={(e) => {
              const files = [...(e.currentTarget.files ?? [])].filter(importable);
              e.currentTarget.value = "";
              if (files.length) void importNotes(files);
            }}
          />
        </li>
      </ul>

      <h2 class="group-title">Devices</h2>
      <ul class="boxed">
        <li>
          <span class="label">Other devices<span class="dim sub">Sign out everywhere except here, e.g. a lost phone</span></span>
          <button onclick={() => run(api.logoutOthers, "Other devices were signed out")}>Log Out Others</button>
        </li>
        <li>
          <span class="label">This device</span>
          <button class="destructive" onclick={endSession}><Icon name="logout" /> Log Out</button>
        </li>
      </ul>

      {#if app.user?.is_admin}
        <h2 class="group-title">People</h2>
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
                      ? { label: "Remove Admin", icon: "shield", onselect: () => adminUpdate(u, { is_admin: false }, `${u.display_name} is no longer an admin`) }
                      : { label: "Make Admin", icon: "shield", onselect: () => adminUpdate(u, { is_admin: true }, `${u.display_name} is now an admin`) },
                    { label: "Reset Password…", icon: "key", onselect: () => ((resetting = u), (resetTo = "")) },
                    u.disabled
                      ? { label: "Enable Account", icon: "check", onselect: () => adminUpdate(u, { disabled: false }, `${u.display_name} can log in again`) }
                      : {
                          label: "Disable Account",
                          icon: "close",
                          destructive: true,
                          onselect: async () => {
                            const ok = await ask({
                              title: `Disable ${u.display_name}?`,
                              body: "They'll be signed out everywhere and can't log in until you enable the account again.",
                              confirm: "Disable",
                              destructive: true,
                            });
                            if (ok) adminUpdate(u, { disabled: true }, `${u.display_name} was signed out and can't log in`);
                          },
                        },
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
                    <button class="suggested" onclick={sendInvite}>{canShare ? "Send" : "Copy"}</button>
                  {/if}
                </div>
              </div>
            {:else}
              <button class="flat add-person" onclick={invite}><Icon name="person_add" /> Invite Someone New</button>
            {/if}
          </li>
        </ul>

        {#if whisper}
          <h2 class="group-title">Speech-to-Text</h2>
          <ServiceForm
            name="stt"
            explain="Voice memos are sent here to be transcribed. Any OpenAI-compatible service works, such as faster-whisper-server or OpenAI. Leave the URL empty to turn it off. Add a live URL to see words in the note while you record."
            urlPlaceholder="http://whisper:8000/v1"
            modelPlaceholder="whisper-1"
            live
            bind:settings={whisper}
            test={api.testWhisper}
            save={async (body) => {
              const saved = (await api.saveWhisper(body)).whisper;
              app.features = { ...app.features, transcription: saved.enabled, live_transcription: saved.enabled && !!saved.realtime_url };
              return saved;
            }}
            saved="Speech-to-text saved"
            off="Transcription turned off"
          />
        {/if}

        {#if vision}
          <h2 class="group-title">Text from Photos</h2>
          <ServiceForm
            name="vision"
            explain="Get Text on a photo sends it here, and the text in it is written under the photo. Any OpenAI-compatible service with a vision model works, such as Ollama, llama.cpp or vLLM on your own hardware. Leave the URL empty to turn it off."
            urlPlaceholder="http://ollama:11434/v1"
            modelPlaceholder="qwen2.5vl"
            bind:settings={vision}
            test={(body) => api.testChat("vision", body)}
            save={async (body) => {
              const saved = await api.saveChat("vision", body);
              app.features = { ...app.features, photo_text: saved.enabled };
              return saved;
            }}
            saved="Text from photos saved"
            off="Text from photos turned off"
          />
        {/if}

        {#if summary}
          <h2 class="group-title">AI Summaries</h2>
          <ServiceForm
            name="summaries"
            explain="A note's Summary tab sends the note here and shows what comes back: the gist, key points and action items. Nothing is sent until someone opens that tab. Any OpenAI-compatible chat service works, such as Ollama, llama.cpp or vLLM on your own hardware. Leave the URL empty to turn it off."
            urlPlaceholder="http://ollama:11434/v1"
            modelPlaceholder="llama3.1"
            bind:settings={summary}
            test={(body) => api.testChat("summary", body)}
            save={async (body) => {
              const saved = await api.saveChat("summary", body);
              app.features = { ...app.features, summaries: saved.enabled };
              return saved;
            }}
            saved="Summaries saved"
            off="Summaries turned off"
          />
        {/if}

        {#if invites.length}
          <h2 class="group-title">Unused Invites</h2>
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

  .headerbar {
    background: var(--window-bg);
  }

  /* As a tab it matches the note lists: same background, empty headerbar, big title. */
  .page.tab,
  .page.tab .headerbar {
    background: var(--view-bg);
  }

  .page.tab .column {
    padding-top: 0;
  }

  .hero {
    padding: 4px 4px 6px;
    animation: rise 260ms var(--ease-out) both;
  }

  .hero h1 {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 800;
    line-height: 1.2;
  }

  .side {
    flex: 1;
    display: flex;
  }

  .heading {
    font-size: var(--text-md);
  }

  .profile {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 20px 0 4px;
    animation: rise 300ms var(--ease-out) both;
  }

  .profile strong {
    margin-top: 10px;
    font-size: var(--text-xl);
  }

  .avatar.large {
    box-shadow: var(--shadow-md);
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

  .group-title {
    margin-top: 26px;
  }

  .boxed {
    margin: 0;
    padding: 0;
    list-style: none;
    border-radius: var(--radius-lg);
    background: var(--card-bg);
    box-shadow: var(--shadow-sm), 0 0 0 1px var(--border);
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
    font-size: var(--text-sm);
  }

  .picker {
    display: none;
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
    font-size: var(--text-sm);
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .stack p {
    margin: 0;
  }

  .error {
    margin: 12px 4px 0;
    padding: 10px 12px;
    border-radius: var(--radius);
    animation: shake 300ms ease;
  }

  .error {
    background: color-mix(in srgb, var(--destructive) 15%, transparent);
    color: var(--destructive);
  }

  .stack .error {
    margin: 0;
  }
</style>
