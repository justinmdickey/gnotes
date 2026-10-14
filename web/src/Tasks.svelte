<script lang="ts">
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import type { TaskItem } from "./lib/api";
  import AccountButton from "./lib/AccountButton.svelte";
  import Icon from "./lib/Icon.svelte";
  import StatusPage from "./lib/StatusPage.svelte";
  import { app, navigate } from "./lib/store.svelte";
  import { openTasks, setDone, tasks } from "./lib/tasks.svelte";
  import { media, scrollEdge, toast } from "./lib/ui.svelte";

  openTasks();

  /** The big title has scrolled away, so the headerbar shows the name instead. */
  let compact = $state(false);
  let query = $state("");
  const q = $derived(query.trim().toLowerCase());
  /** Which part of your notes to show: everything, one of your top notebooks, or what's shared with you. */
  let scope = $state<string>("all");
  let showDone = $state(false);
  /** Just ticked: they stay a moment, crossed out, so a tick can be seen (and undone) before the row goes. */
  let leaving = $state(new Set<string>());
  const key = (t: TaskItem) => `${t.note}:${t.line}`;

  /** Today as "YYYY-MM-DD" in local time, the form due dates are written in. */
  function isoDay(d: Date) {
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }
  const today = isoDay(new Date());
  const DAY = 24 * 60 * 60 * 1000;
  const dateOf = (s: string) => new Date(`${s}T12:00:00`);
  function dueLabel(due: string): { text: string; tone: "late" | "today" | "" } {
    const days = Math.round((dateOf(due).getTime() - dateOf(today).getTime()) / DAY);
    if (days === 0) return { text: "Today", tone: "today" };
    if (days === 1) return { text: "Tomorrow", tone: "" };
    if (days === -1) return { text: "Yesterday", tone: "late" };
    if (days < 0) return { text: `${-days} days late`, tone: "late" };
    const d = dateOf(due);
    return { text: d.toLocaleDateString([], days < 7 ? { weekday: "short" } : { month: "short", day: "numeric" }), tone: "" };
  }

  /** A note's place: its notebooks, outermost first, and the top one it sits in. */
  function placeOf(notebookId: string | null) {
    const path = [];
    for (let id = notebookId; id; ) {
      const nb = app.tree.notebooks.find((n) => n.id === id);
      if (!nb) break;
      path.unshift(nb);
      id = nb.parent_id;
    }
    return path;
  }
  function scopeOf(t: TaskItem) {
    const note = app.tree.notes.find((n) => n.id === t.note);
    const top = placeOf(t.notebook)[0];
    if (top) return top.role === "owner" ? top.id : "shared";
    return note && note.role !== "owner" ? "shared" : "notes";
  }
  function crumb(t: TaskItem) {
    const names = placeOf(t.notebook).map((n) => n.name);
    return [names.join(" › "), [t.title || "New Note", t.column].filter(Boolean).join(" › ")].filter(Boolean).join(" · ");
  }

  /** Chips for the places that have tasks: your top notebooks A–Z, then Shared. Only worth showing for two or more. */
  const scopes = $derived.by(() => {
    const used = new Set((tasks.items ?? []).filter((t) => !t.done).map(scopeOf));
    const out = app.tree.notebooks
      .filter((n) => n.role === "owner" && !n.parent_id && used.has(n.id))
      .toSorted((a, b) => a.name.localeCompare(b.name))
      .map((n) => ({ id: n.id, label: n.name }));
    if (used.has("shared")) out.push({ id: "shared", label: "Shared" });
    return out.length + (used.has("notes") ? 1 : 0) > 1 ? out : [];
  });
  $effect(() => {
    if (scope !== "all" && !scopes.some((s) => s.id === scope)) scope = "all";
  });

  const shown = $derived(
    (tasks.items ?? []).filter(
      (t) => (scope === "all" || scopeOf(t) === scope) && (!q || t.text.toLowerCase().includes(q) || t.title.toLowerCase().includes(q)),
    ),
  );
  const open = $derived(shown.filter((t) => !t.done || leaving.has(key(t))));
  const done = $derived(shown.filter((t) => t.done && !leaving.has(key(t))));
  const byDue = (a: TaskItem, b: TaskItem) => (a.due ?? "").localeCompare(b.due ?? "");
  const groups = $derived(
    [
      { label: "Overdue", late: true, items: open.filter((t) => t.due && t.due < today) },
      { label: "Today", late: false, items: open.filter((t) => t.due === today) },
      { label: "Upcoming", late: false, items: open.filter((t) => t.due && t.due > today).toSorted(byDue) },
      { label: "No Date", late: false, items: open.filter((t) => !t.due) },
    ].filter((g) => g.items.length),
  );
  const counts = $derived.by(() => {
    const all = (tasks.items ?? []).filter((t) => !t.done);
    const late = all.filter((t) => t.due && t.due < today).length;
    return [all.length ? `${all.length} open` : "", late ? `${late} overdue` : ""].filter(Boolean).join(" · ");
  });

  async function tick(t: TaskItem) {
    const ticking = !t.done;
    const k = key(t);
    if (ticking) leaving = new Set(leaving).add(k);
    const ok = await setDone(t, ticking);
    if (!ok || !ticking) {
      leaving.delete(k);
      leaving = new Set(leaving);
      return;
    }
    setTimeout(() => {
      leaving.delete(k);
      leaving = new Set(leaving);
    }, 1400);
    toast("Checked off", { label: "Undo", run: () => void setDone({ ...t, done: true }, false) });
  }

  /** Opens the note at the task's line, remembering Tasks as where it came from. */
  function jump(t: TaskItem) {
    app.jump = { note: t.note, line: t.line };
    navigate({ kind: "tasks" }, t.note);
  }

  const reveal = (node: Element) => slide(node, { duration: media.reduced ? 0 : 200, easing: cubicOut });
</script>

<section>
  <header class="headerbar">
    <div class="start"></div>
    <!-- Like the other phone tabs: the app's name until the big title scrolls away, then the place's. -->
    <div class="title shown">
      {#if media.phone && !compact}
        <strong class="brand"><img src="/icon.svg" alt="" width="22" height="22" />Gnotes</strong>
      {:else}
        <strong>Tasks</strong>
      {/if}
    </div>
    <div class="end">
      <AccountButton class="phone-only" />
    </div>
  </header>

  <div class="scroll" use:scrollEdge onscroll={(e) => (compact = e.currentTarget.scrollTop > 56)}>
    <div class="hero">
      <h1>Tasks</h1>
      <span class="dim">{counts || "Checkboxes from all your notes"}</span>
    </div>

    <label class="search">
      <Icon name="search" />
      <input type="search" placeholder="Search tasks" aria-label="Search tasks" bind:value={query} />
      {#if query}
        <button class="flat icon circular clear" aria-label="Clear search" onclick={() => (query = "")}><Icon name="close" /></button>
      {/if}
    </label>

    {#if scopes.length}
      <div class="chips" role="group" aria-label="Show tasks from">
        <button class="chip" class:on={scope === "all"} aria-pressed={scope === "all"} onclick={() => (scope = "all")}>All</button>
        {#each scopes as s (s.id)}
          <button class="chip" class:on={scope === s.id} aria-pressed={scope === s.id} onclick={() => (scope = s.id)}>{s.label}</button>
        {/each}
      </div>
    {/if}

    {#snippet row(t: TaskItem)}
      {@const due = t.due ? dueLabel(t.due) : null}
      <div class="task" class:done={t.done}>
        <button
          class="flat box"
          class:checked={t.done}
          role="checkbox"
          aria-checked={t.done}
          aria-label={t.done ? `Mark “${t.text}” not done` : `Mark “${t.text}” done`}
          onclick={() => tick(t)}
        ></button>
        <button class="flat what" onclick={() => jump(t)}>
          <span class="text">{t.text}</span>
          <span class="where">{crumb(t)}</span>
        </button>
        {#if due}<span class="due {due.tone}">{due.text}</span>{/if}
      </div>
    {/snippet}

    {#each groups as group (group.label)}
      <h3 class="group-title" class:late={group.late}>{group.label}</h3>
      <ul class="boxed-list">
        {#each group.items as t (key(t))}
          <li transition:reveal>{@render row(t)}</li>
        {/each}
      </ul>
    {:else}
      {#if tasks.items && q}
        <StatusPage icon="search" title="No Results" description="No open task has “{query}”." tone="neutral" />
      {:else if tasks.items}
        <StatusPage icon="checklist" title="Nothing to Do" description="Checkboxes in your notes show up here. Type “- [ ]” in any note to add one." tone="neutral" />
      {/if}
    {/each}

    {#if done.length}
      {#if showDone}
        <h3 class="group-title">Done</h3>
        <ul class="boxed-list">
          {#each done as t (key(t))}
            <li transition:reveal>{@render row(t)}</li>
          {/each}
        </ul>
      {/if}
      <button class="flat more" onclick={() => (showDone = !showDone)}>{showDone ? "Hide Done" : `Show ${done.length} Done`}</button>
    {/if}
  </div>
</section>

<style>
  section {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--view-bg);
  }

  /* Three columns, so the title sits in the middle of the headerbar, as on the notes lists. */
  .headerbar {
    display: grid;
    grid-template-columns: 1fr minmax(0, auto) 1fr;
  }

  .start,
  .end {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .end {
    justify-content: flex-end;
  }

  .headerbar .title {
    align-items: center;
    text-align: center;
    opacity: 0;
    transform: translateY(4px);
    transition:
      opacity var(--fast) ease,
      transform var(--fast) ease;
  }

  .headerbar .title.shown {
    opacity: 1;
    transform: none;
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 0 12px 16px;
  }

  .hero {
    display: flex;
    flex-direction: column;
    padding: 4px 4px 14px;
    animation: rise 260ms var(--ease-out) both;
  }

  .hero h1 {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 800;
    line-height: 1.2;
  }

  .hero span {
    font-size: var(--text-sm);
  }

  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 2px 0 6px;
    padding: 0 4px 0 12px;
    border-radius: var(--radius);
    background: var(--entry-bg);
    color: var(--dim-fg);
    transition: box-shadow var(--fast) ease;
  }

  .search:focus-within {
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-bg) 60%, transparent);
  }

  .search input {
    flex: 1;
    min-width: 0;
    background: transparent;
    padding-left: 0;
    box-shadow: none;
    color: var(--fg);
  }

  .search input::-webkit-search-cancel-button {
    display: none;
  }

  .clear {
    min-width: 28px;
    min-height: 28px;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 8px 0 2px;
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

  .group-title.late {
    color: var(--destructive);
  }

  .task {
    display: flex;
    align-items: flex-start;
    gap: 4px;
    padding: 4px 12px 4px 4px;
    min-height: 56px;
  }

  /* The editor's checkbox, round, with room around it to tap. */
  .box {
    flex: none;
    width: 44px;
    height: 44px;
    min-height: 0;
    padding: 0;
    border-radius: 50%;
  }

  .box::before {
    content: "";
    width: 20px;
    height: 20px;
    border: 1.5px solid var(--dim-fg);
    border-radius: 50%;
    box-sizing: border-box;
    transition:
      background var(--fast) ease,
      border-color var(--fast) ease;
  }

  .box.checked::before {
    border-color: var(--accent-bg);
    background: var(--accent-bg)
      url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'%3E%3Cpath d='M4 8.5l2.5 2.5L12 5.5' fill='none' stroke='white' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E")
      center / 80% no-repeat;
  }

  .what {
    flex: 1;
    min-width: 0;
    flex-direction: column;
    align-items: stretch;
    gap: 2px;
    padding: 9px 0;
    border-radius: 0;
    background: none;
    font-weight: 400;
    text-align: left;
  }

  .what:active:not(:disabled),
  .box:active:not(:disabled) {
    transform: none;
  }

  .text {
    overflow-wrap: anywhere;
  }

  .done .text {
    color: var(--dim-fg);
    text-decoration: line-through;
  }

  .where {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--accent);
    font-size: var(--text-xs);
  }

  .due {
    flex: none;
    padding-top: 11px;
    color: var(--dim-fg);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }

  .due.today {
    color: var(--accent);
    font-weight: 700;
  }

  .due.late {
    color: var(--destructive);
    font-weight: 700;
  }

  .more {
    margin: 10px auto 0;
    display: flex;
    color: var(--accent);
    font-size: var(--text-sm);
  }
</style>
