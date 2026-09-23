<script lang="ts">
  import { api, inviteUrl, type UserSummary } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import { colorFor } from "./lib/store.svelte";

  let { onclose }: { onclose: () => void } = $props();

  let users = $state<UserSummary[]>([]);
  let invite = $state<string | null>(null);
  let copied = $state(false);
  // Both only exist in secure contexts (HTTPS), so plain-HTTP dev falls back to a selectable link.
  const canShare = "share" in navigator;
  const canCopy = "clipboard" in navigator;
  api.users().then((u) => (users = u));

  async function makeInvite() {
    invite = inviteUrl((await api.invite()).token);
  }

  async function send() {
    if (!invite) return;
    if (canShare) {
      await navigator.share({ title: "Gnotes invite", text: `Join me on Gnotes: ${invite}`, url: invite }).catch(() => {});
    } else if (canCopy) {
      await navigator.clipboard.writeText(invite);
      copied = true;
    }
  }
</script>

<Dialog title="People" {onclose}>
  <ul class="boxed">
    {#each users as u (u.id)}
      <li>
        <span class="avatar {colorFor(u.id)}">{u.display_name.slice(0, 1).toUpperCase()}</span>
        <span class="who">{u.display_name} <span class="dim">@{u.username}</span></span>
      </li>
    {/each}
  </ul>
  {#if invite}
    <p class="dim hint">Send this link. It works once and expires in 7 days.</p>
    <div class="link-row">
      <input class="link" readonly value={invite} onfocus={(e) => e.currentTarget.select()} aria-label="Invite link" />
      {#if canShare || canCopy}
        <button class="suggested" onclick={send}>{canShare ? "Send" : copied ? "Copied" : "Copy"}</button>
      {/if}
    </div>
  {:else}
    <button class="invite" onclick={makeInvite}><Icon name="person_add" /> Invite someone new</button>
  {/if}
  {#snippet actions()}
    <button onclick={onclose}>Done</button>
  {/snippet}
</Dialog>

<style>
  .boxed {
    margin: 0 0 16px;
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
    padding: 6px 12px;
  }

  .boxed li + li {
    border-top: 1px solid var(--border);
  }

  .avatar {
    flex: none;
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: var(--user-color, var(--accent));
    color: #fff;
    font-weight: 700;
  }

  .who {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .invite {
    width: 100%;
    min-height: 44px;
  }

  .hint {
    margin: 0 0 8px;
    font-size: 0.9rem;
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
</style>
