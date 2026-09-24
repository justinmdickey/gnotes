<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon, { type IconName } from "./Icon.svelte";

  let {
    icon,
    title,
    description,
    tone = "accent",
    fill = false,
    children,
  }: {
    icon: IconName;
    title: string;
    description?: string;
    /** Accent for invitations to act, neutral for dead ends. */
    tone?: "accent" | "neutral";
    /** Fill and center in the parent instead of sitting at the top. */
    fill?: boolean;
    children?: Snippet;
  } = $props();
</script>

<!-- An AdwStatusPage: one icon, a title, a line of explanation and an optional action. -->
<div class="status" class:fill>
  <div class="badge {tone}"><Icon name={icon} size={36} /></div>
  <strong>{title}</strong>
  {#if description}<p class="dim">{description}</p>{/if}
  {#if children}<div class="actions">{@render children()}</div>{/if}
</div>

<style>
  .status {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 56px 24px 24px;
    text-align: center;
    animation: rise 260ms var(--ease-out) both;
  }

  .status.fill {
    justify-content: center;
    height: 100%;
    padding-top: 24px;
  }

  .badge {
    display: grid;
    place-items: center;
    width: 80px;
    height: 80px;
    margin-bottom: 12px;
    border-radius: 50%;
  }

  .badge.accent {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .badge.neutral {
    background: var(--hover);
    color: var(--dim-fg);
  }

  strong {
    font-size: var(--text-lg);
    font-weight: 800;
  }

  p {
    max-width: 320px;
    margin: 0;
    line-height: 1.45;
  }

  .actions {
    margin-top: 16px;
  }
</style>
