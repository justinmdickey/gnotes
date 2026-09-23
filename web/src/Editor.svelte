<script lang="ts">
  import { onMount } from "svelte";
  import { Compartment, EditorState } from "@codemirror/state";
  import { EditorView, keymap } from "@codemirror/view";
  import { defaultKeymap } from "@codemirror/commands";
  import { markdown } from "@codemirror/lang-markdown";
  import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
  import { tags } from "@lezer/highlight";
  import { EphemeralStore, LoroDoc, UndoManager, type VersionVector } from "loro-crdt";
  import { LoroExtensions, getCursorEphemeralKey, getUserEphemeralKey } from "loro-codemirror";
  import { api, type Role } from "./lib/api";
  import Icon from "./lib/Icon.svelte";
  import { sync } from "./lib/sync";
  import { app, colorFor, openNote } from "./lib/store.svelte";
  import ShareDialog from "./ShareDialog.svelte";

  let { noteId }: { noteId: string } = $props();

  const note = $derived(app.tree.notes.find((n) => n.id === noteId));
  let role = $state<Role | null>(null);
  let loaded = $state(false);
  let lost = $state<"revoked" | "not_found" | null>(null);
  let peers = $state<{ name: string; color: string }[]>([]);
  let sharing = $state(false);
  let parent: HTMLDivElement;

  const doc = new LoroDoc();
  const ephemeral = new EphemeralStore(30_000);
  const undoManager = new UndoManager(doc, {});
  const editable = new Compartment();
  let view: EditorView;
  let hasData = false;

  const markdownStyle = HighlightStyle.define([
    { tag: tags.heading1, fontSize: "1.6em", fontWeight: "800" },
    { tag: tags.heading2, fontSize: "1.35em", fontWeight: "800" },
    { tag: tags.heading3, fontSize: "1.15em", fontWeight: "700" },
    { tag: [tags.heading4, tags.heading5, tags.heading6], fontWeight: "700" },
    { tag: tags.strong, fontWeight: "700" },
    { tag: tags.emphasis, fontStyle: "italic" },
    { tag: tags.strikethrough, textDecoration: "line-through" },
    { tag: [tags.link, tags.url], color: "var(--accent)" },
    { tag: tags.monospace, fontFamily: "'Adwaita Mono', 'Source Code Pro', monospace", fontSize: "0.92em" },
    { tag: [tags.processingInstruction, tags.contentSeparator, tags.meta], color: "var(--dim-fg)" },
    { tag: tags.quote, color: "var(--dim-fg)", fontStyle: "italic" },
  ]);

  /** Editable only once content has arrived, so nothing is typed into a doc that's about to be replaced. */
  function refreshEditable() {
    const on = loaded && !lost && role !== null && role !== "viewer";
    view?.dispatch({ effects: editable.reconfigure(EditorView.editable.of(on)) });
  }

  function setRole(r: Role) {
    role = r;
    refreshEditable();
  }

  function updatePeers() {
    const mine = doc.peerIdStr;
    const seen = new Map<string, { name: string; color: string }>();
    for (const [key, value] of Object.entries(ephemeral.getAllStates())) {
      if (!key.endsWith("-cm-user") || key.startsWith(`${mine}-`) || !value) continue;
      const user = value as { name: string; colorClassName: string };
      seen.set(user.name, { name: user.name, color: user.colorClassName });
    }
    peers = [...seen.values()];
  }

  onMount(() => {
    view = new EditorView({
      parent,
      state: EditorState.create({
        extensions: [
          markdown(),
          syntaxHighlighting(markdownStyle),
          EditorView.lineWrapping,
          keymap.of(defaultKeymap),
          editable.of(EditorView.editable.of(false)),
          EditorView.contentAttributes.of({ "aria-label": "Note text", autocapitalize: "sentences", spellcheck: "true" }),
          LoroExtensions(
            doc,
            { ephemeral, user: { name: app.user!.display_name, colorClassName: colorFor(app.user!.id) } },
            undoManager,
            (d) => d.getText("body"),
          ),
        ],
      }),
    });

    const offDoc = doc.subscribeLocalUpdates((bytes) => sync.sendUpdate(noteId, bytes));
    const offPresence = ephemeral.subscribeLocalUpdates((bytes) => sync.sendPresence(noteId, bytes));
    const offPeers = ephemeral.subscribe(updatePeers);

    sync.open(noteId, {
      version: () => (hasData ? doc.oplogVersion() : null),
      joined(r: Role, server: VersionVector) {
        setRole(r);
        // Upload anything typed while disconnected.
        const cmp = doc.oplogVersion().compare(server);
        if (hasData && r !== "viewer" && (cmp === undefined || cmp > 0)) {
          sync.sendUpdate(noteId, doc.export({ mode: "update", from: server }));
        }
      },
      update(data) {
        doc.import(data);
        hasData = true;
        if (!loaded) {
          loaded = true;
          refreshEditable();
        }
      },
      presence: (data) => ephemeral.apply(data),
      role: setRole,
      lost(reason) {
        lost = reason;
        refreshEditable();
      },
      outOfSync: () => sync.rejoin(noteId),
    });

    // Presence expires after 30s without a refresh, so re-announce while the note is open.
    const heartbeat = setInterval(() => {
      for (const key of [getCursorEphemeralKey(doc), getUserEphemeralKey(doc)]) {
        const value = ephemeral.get(key);
        if (value !== undefined) ephemeral.set(key, value);
      }
    }, 10_000);

    return () => {
      clearInterval(heartbeat);
      sync.close(noteId);
      offDoc();
      offPresence();
      offPeers();
      view.destroy();
      ephemeral.destroy();
    };
  });

  async function remove() {
    if (!confirm("Move this note to the trash?")) return;
    await api.deleteNote(noteId);
    openNote(null);
  }
</script>

<section>
  <header>
    <button class="flat icon narrow-only" aria-label="Back" onclick={() => openNote(null)}><Icon name="back" /></button>
    <span class="title">{note?.title || "Untitled"}</span>
    <div class="peers">
      {#each peers as peer (peer.name)}
        <span class="avatar {peer.color}" title="{peer.name} is here">{peer.name.slice(0, 1).toUpperCase()}</span>
      {/each}
    </div>
    {#if role === "viewer"}<span class="badge dim">View only</span>{/if}
    {#if role === "owner"}
      <button class="flat icon" title="Share" aria-label="Share" onclick={() => (sharing = true)}><Icon name="share" /></button>
      <button class="flat icon" title="Move to trash" aria-label="Move to trash" onclick={remove}><Icon name="trash" /></button>
    {/if}
  </header>

  {#if lost}
    <div class="banner">
      {lost === "revoked" ? "You no longer have access to this note." : "This note doesn't exist or was moved to the trash."}
    </div>
  {:else if !loaded}
    <div class="banner dim">{app.status === "online" ? "Loading…" : "Waiting for the server…"}</div>
  {/if}

  <div class="scroll">
    <div class="editor" class:hidden={!loaded} bind:this={parent}></div>
  </div>
</section>

{#if sharing}
  <ShareDialog kind="note" id={noteId} name={note?.title ?? ""} onclose={() => (sharing = false)} />
{/if}

<style>
  section {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--view-bg);
  }

  header {
    display: flex;
    align-items: center;
    gap: 2px;
    min-height: 47px;
    padding: 0 6px;
    padding-top: env(safe-area-inset-top);
    border-bottom: 1px solid var(--border);
  }

  .title {
    flex: 1;
    min-width: 0;
    padding-left: 10px;
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .peers {
    display: flex;
    gap: 4px;
    padding: 0 6px;
  }

  .avatar {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    background: var(--user-color);
    color: #fff;
    font-size: 0.8rem;
    font-weight: 700;
  }

  .badge {
    padding: 0 8px;
    font-size: 0.85rem;
  }

  .banner {
    padding: 10px 16px;
    background: var(--hover);
    text-align: center;
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    display: flex;
    justify-content: center;
  }

  .editor {
    width: 100%;
    max-width: 760px;
    padding: 24px 16px 40vh;
  }

  .hidden {
    visibility: hidden;
  }

  .editor :global(.cm-editor) {
    background: transparent;
    color: var(--fg);
    font-size: 1rem;
  }

  .editor :global(.cm-editor.cm-focused) {
    outline: none;
  }

  .editor :global(.cm-content) {
    caret-color: var(--accent);
    line-height: 1.6;
  }

  .editor :global(.cm-scroller) {
    overflow: visible;
    font-family: inherit;
  }
</style>
