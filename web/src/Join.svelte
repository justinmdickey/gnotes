<script lang="ts">
  import { api, ApiError, type InvitePreview } from "./lib/api";
  import { app, endSession } from "./lib/store.svelte";

  let { token }: { token: string } = $props();

  let preview = $state<InvitePreview | null>(null);
  let invalid = $state(false);
  let name = $state("");
  let username = $state("");
  let usernameTouched = $state(false);
  let password = $state("");
  let error = $state("");
  let busy = $state(false);

  $effect(() => {
    api.previewJoin(token).then((p) => (preview = p), () => (invalid = true));
  });

  // Suggest a username from the name until they edit it themselves.
  $effect(() => {
    if (!usernameTouched) username = name.trim().toLowerCase().replace(/[^a-z0-9._-]+/g, "").slice(0, 32);
  });

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    error = "";
    try {
      await api.join(token, username, name, password);
      // The session cookie is set; reload into the app proper.
      location.replace("/#/");
    } catch (err) {
      if (err instanceof ApiError && err.status === 404) invalid = true;
      else error = err instanceof ApiError ? err.message : "Couldn't reach the server";
    } finally {
      busy = false;
    }
  }
</script>

<main>
  <div class="card">
    <img src="/icon.svg" alt="" width="80" height="80" />
    {#if invalid}
      <h1>Invite not valid</h1>
      <p class="dim">This link has expired or was already used. Ask for a new one.</p>
      <a class="button pill-link" href="/">Go to Gnotes</a>
    {:else if app.user}
      <h1>Already signed in</h1>
      <p class="dim">You're signed in as {app.user.display_name}. Log out to use this invite for a new account.</p>
      <button onclick={endSession}>Log Out</button>
    {:else if preview}
      <h1>Join Gnotes</h1>
      <p class="dim">
        {preview.inviter} invited you{#if preview.shared}&nbsp;to share <strong>{preview.shared}</strong>{/if}.
      </p>
      <form onsubmit={submit}>
        <input name="name" placeholder="Your name" autocomplete="name" bind:value={name} required />
        <input
          name="username"
          placeholder="Username"
          autocomplete="username"
          autocapitalize="none"
          bind:value={username}
          oninput={() => (usernameTouched = true)}
          required
        />
        <input name="password" type="password" placeholder="Password (8+ characters)" autocomplete="new-password" minlength="8" bind:value={password} required />
        {#if error}{#key error}<p class="error">{error}</p>{/key}{/if}
        <button class="suggested pill" type="submit" disabled={busy}>
          {#if busy}<span class="spinner"></span>{:else}Create Account{/if}
        </button>
      </form>
    {/if}
  </div>
</main>

<style>
  main {
    display: grid;
    place-items: center;
    min-height: 100%;
    padding: 16px;
  }

  .card {
    display: flex;
    flex-direction: column;
    align-items: center;
    width: min(360px, 100%);
    text-align: center;
    animation: rise 400ms var(--ease-out) both;
  }

  img {
    filter: drop-shadow(0 6px 16px rgb(0 0 6 / 18%));
  }

  input {
    min-height: 46px;
    padding: 0 16px;
  }

  h1 {
    margin: 12px 0 4px;
    font-size: var(--text-display);
  }

  p {
    margin: 0 0 20px;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    width: 100%;
  }

  form button {
    min-height: 48px;
    margin-top: 4px;
  }

  .pill-link {
    min-height: 42px;
    padding: 0 22px;
    border-radius: var(--radius-pill);
    color: var(--accent);
  }

  .error {
    margin: 0;
    color: var(--destructive);
    animation: shake 300ms ease;
  }
</style>
