<script lang="ts">
  import { api, ApiError } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import { app, navigate } from "./lib/store.svelte";
  import { refreshTasks } from "./lib/tasks.svelte";
  import { toast } from "./lib/ui.svelte";

  /** A task in one step, from the tab bar's + or Tasks: it goes on the end of your Inbox note. */
  let { onclose }: { onclose: () => void } = $props();

  let text = $state("");
  let due = $state<string | null>(null);
  let error = $state("");
  let busy = $state(false);

  const iso = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  const inDays = (n: number) => {
    const d = new Date();
    d.setDate(d.getDate() + n);
    return iso(d);
  };
  /** The coming Saturday; on a Saturday, the next one. */
  const weekend = () => inDays((6 - new Date().getDay() + 7) % 7 || 7);
  const dates = [
    { label: "No Date", value: null },
    { label: "Today", value: inDays(0) },
    { label: "Tomorrow", value: inDays(1) },
    { label: "Weekend", value: weekend() },
    { label: "Next Week", value: inDays(7) },
  ];

  async function add(e: SubmitEvent) {
    e.preventDefault();
    if (!text.trim() || busy) return;
    busy = true;
    try {
      const { note } = await api.addTask(text.trim(), due);
      onclose();
      void refreshTasks();
      toast("Added to Inbox", { label: "Show", run: () => navigate(app.view.kind === "tasks" ? app.view : { kind: "root" }, note) });
    } catch (err) {
      error = err instanceof ApiError ? err.message : "Tasks can't be added offline yet";
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title="New Task" {onclose}>
  <form id="new-task" onsubmit={add}>
    <!-- svelte-ignore a11y_autofocus -->
    <input placeholder="What needs doing?" aria-label="Task" bind:value={text} autofocus enterkeyhint="done" />
    <div class="dates" role="group" aria-label="Due">
      {#each dates as d (d.label)}
        <button type="button" class="chip" class:on={due === d.value} aria-pressed={due === d.value} onclick={() => (due = d.value)}>{d.label}</button>
      {/each}
    </div>
    <p class="dim where">Goes to your <strong>Inbox</strong> note</p>
    {#if error}<p class="error">{error}</p>{/if}
  </form>
  {#snippet actions()}
    <button onclick={onclose}>Cancel</button>
    <button class="suggested" type="submit" form="new-task" disabled={!text.trim() || busy}>Add</button>
  {/snippet}
</Dialog>

<style>
  input {
    width: 100%;
  }

  .dates {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 12px 0 0;
  }

  .chip {
    min-height: 30px;
    padding: 0 12px;
    border-radius: var(--radius-pill);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .chip.on {
    background: var(--accent-bg);
    color: var(--accent-fg);
  }

  .where {
    margin: 12px 0 0;
    font-size: var(--text-sm);
  }

  .where strong {
    color: var(--accent);
  }

  .error {
    color: var(--destructive);
  }
</style>
