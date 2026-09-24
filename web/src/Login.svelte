<script lang="ts">
  import { api, ApiError } from "./lib/api";
  import { app, startSession } from "./lib/store.svelte";

  let username = $state("");
  let password = $state("");
  let error = $state("");
  let busy = $state(false);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    error = "";
    try {
      startSession(await api.login(username, password));
      app.signedOutReason = "";
    } catch (err) {
      error = err instanceof ApiError && err.status === 401 ? "Wrong username or password" : "Couldn't reach the server";
    } finally {
      busy = false;
    }
  }
</script>

<main>
  <form onsubmit={submit}>
    <img src="/icon.svg" alt="" width="96" height="96" />
    <h1>Gnotes</h1>
    {#if app.signedOutReason}<p class="reason">{app.signedOutReason}</p>{/if}
    <input name="username" placeholder="Username" autocomplete="username" autocapitalize="none" bind:value={username} required />
    <input name="password" type="password" placeholder="Password" autocomplete="current-password" bind:value={password} required />
    {#if error}{#key error}<p class="error">{error}</p>{/key}{/if}
    <button class="suggested pill" type="submit" disabled={busy}>
      {#if busy}<span class="spinner"></span>{:else}Log In{/if}
    </button>
  </form>
</main>

<style>
  main {
    display: grid;
    place-items: center;
    min-height: 100%;
    padding: 16px;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    width: min(340px, 100%);
    animation: rise 400ms var(--ease-out) both;
  }

  img {
    align-self: center;
    filter: drop-shadow(0 6px 16px rgb(0 0 6 / 18%));
  }

  input {
    min-height: 46px;
    padding: 0 16px;
  }

  h1 {
    margin: 0 0 12px;
    text-align: center;
    font-size: var(--text-display);
  }

  button {
    margin-top: 8px;
    min-height: 48px;
  }

  .reason {
    margin: 0 0 4px;
    text-align: center;
    color: var(--dim-fg);
  }

  .error {
    margin: 0;
    color: var(--destructive);
    text-align: center;
    animation: shake 300ms ease;
  }
</style>
