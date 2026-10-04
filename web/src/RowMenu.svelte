<script lang="ts" module>
  import { app } from "./lib/store.svelte";

  /** A row whose actions are open: what it is, and where the press was. */
  export type RowPress = { kind: "note" | "notebook"; id: string; x: number; y: number };

  /** Whether a row has anything to offer: viewers can't move, share or trash. */
  export function hasActions(kind: "note" | "notebook", id: string) {
    const item = kind === "note" ? app.tree.notes.find((n) => n.id === id) : app.tree.notebooks.find((n) => n.id === id);
    return !!item && item.role !== "viewer";
  }
</script>

<script lang="ts">
  import { untrack } from "svelte";
  import { api } from "./lib/api";
  import Dialog from "./lib/Dialog.svelte";
  import Menu, { type MenuItem } from "./lib/Menu.svelte";
  import { trashNote, trashNotebook } from "./lib/store.svelte";
  import MoveDialog from "./MoveDialog.svelte";
  import ShareDialog from "./ShareDialog.svelte";

  /**
   * A note's or notebook's actions from its row in a list, the same ones its own screen offers:
   * a popover at the pointer on wide screens, an action sheet on phones.
   */
  let { press, onclose }: { press: RowPress; onclose: () => void } = $props();
  // Pinned at open, so a tree refresh mid-dialog can't swap the item.
  const { kind, id, x, y } = untrack(() => press);

  const item = $derived(kind === "note" ? app.tree.notes.find((n) => n.id === id) : app.tree.notebooks.find((n) => n.id === id));
  const name = $derived(!item ? "" : "name" in item ? item.name : item.title || "New Note");

  /** What's showing: the menu, or the dialog one of its choices opened. */
  let step = $state<"menu" | "move" | "share" | "rename">("menu");
  let newName = $state("");

  const items = $derived.by(() => {
    const out: MenuItem[] = [];
    if (!item || item.role === "viewer") return out;
    out.push({ label: "Move to…", icon: "move", onselect: () => (step = "move") });
    if (item.role !== "owner") return out;
    if (kind === "notebook") out.push({ label: "Rename…", icon: "rename", onselect: () => ((newName = name), (step = "rename")) });
    out.push({ label: "Share…", icon: "share", onselect: () => (step = "share") });
    out.push({
      label: "Move to Trash",
      icon: "trash",
      destructive: true,
      onselect: () => void (kind === "note" ? trashNote(id) : trashNotebook(id, name)),
    });
    return out;
  });

  async function rename(e: SubmitEvent) {
    e.preventDefault();
    if (newName.trim()) await api.renameNotebook(id, newName.trim());
    onclose();
  }
</script>

{#if step === "menu"}
  <Menu label={name} {items} point={{ x, y }} onclose={() => step === "menu" && onclose()} />
{:else if step === "move"}
  <MoveDialog {kind} {id} {name} {onclose} />
{:else if step === "share"}
  <ShareDialog {kind} {id} {name} {onclose} />
{:else}
  <Dialog title="Rename Notebook" {onclose}>
    <form id="rename-row" onsubmit={rename}>
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={newName} aria-label="Notebook name" autofocus />
    </form>
    {#snippet actions()}
      <button onclick={onclose}>Cancel</button>
      <button class="suggested" type="submit" form="rename-row" disabled={!newName.trim()}>Rename</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  input {
    width: 100%;
  }
</style>
