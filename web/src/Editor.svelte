<script lang="ts">
  import { onMount } from "svelte";
  import { Compartment, EditorSelection, EditorState } from "@codemirror/state";
  import { EditorView, keymap, placeholder } from "@codemirror/view";
  import { defaultKeymap } from "@codemirror/commands";
  import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
  import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
  import { tags } from "@lezer/highlight";
  import { EphemeralStore, LoroDoc, UndoManager, type VersionVector } from "loro-crdt";
  import { LoroExtensions, getCursorEphemeralKey, getUserEphemeralKey } from "loro-codemirror";
  import { api, type Role } from "./lib/api";
  import { activeFormats, formatKeymap, type Block, type Inline } from "./lib/format";
  import Icon from "./lib/Icon.svelte";
  import { livePreview } from "./lib/livePreview";
  import { sync } from "./lib/sync";
  import { app, colorFor, openNote, viewTitle } from "./lib/store.svelte";
  import FormatBar from "./FormatBar.svelte";
  import ShareDialog from "./ShareDialog.svelte";

  let { noteId }: { noteId: string } = $props();

  const note = $derived(app.tree.notes.find((n) => n.id === noteId));
  let role = $state<Role | null>(null);
  let loaded = $state(false);
  let lost = $state<"revoked" | "not_found" | null>(null);
  let peers = $state<{ name: string; color: string }[]>([]);
  let sharing = $state(false);
  let focused = $state(false);
  let block = $state<Block>("body");
  let inline = $state<Set<Inline>>(new Set());
  /** Height covered by the on-screen keyboard, so the phone toolbar can sit on top of it. */
  let keyboard = $state(0);
  let view = $state<EditorView>();
  let parent: HTMLDivElement;

  const canEdit = $derived(loaded && !lost && role !== null && role !== "viewer");

  const doc = new LoroDoc();
  const ephemeral = new EphemeralStore(30_000);
  const undoManager = new UndoManager(doc, {});
  const editable = new Compartment();
  let hasData = false;

  const markdownStyle = HighlightStyle.define([
    { tag: [tags.heading1, tags.heading2, tags.heading3, tags.heading4, tags.heading5, tags.heading6], fontWeight: "inherit" },
    { tag: tags.strong, fontWeight: "700" },
    { tag: tags.emphasis, fontStyle: "italic" },
    { tag: tags.strikethrough, textDecoration: "line-through" },
    { tag: [tags.link, tags.url], color: "var(--accent)" },
    { tag: tags.monospace, fontFamily: "'Adwaita Mono', 'Source Code Pro', monospace", fontSize: "0.92em" },
    { tag: [tags.processingInstruction, tags.contentSeparator, tags.meta], color: "var(--dim-fg)" },
  ]);

  /** Editable only once content has arrived, so nothing is typed into a doc that's about to be replaced. */
  function refreshEditable() {
    view?.dispatch({ effects: editable.reconfigure(EditorView.editable.of(canEdit)) });
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

  /** Tapping blank space below the text puts the cursor at the end, like a sheet of paper. */
  function focusEnd(e: MouseEvent) {
    if (!view || !canEdit || view.contentDOM.contains(e.target as Node)) return;
    const end = view.state.doc.length;
    view.dispatch({ selection: EditorSelection.cursor(end), scrollIntoView: true });
    view.focus();
  }

  onMount(() => {
    const v = new EditorView({
      parent,
      state: EditorState.create({
        extensions: [
          markdown({ base: markdownLanguage }),
          syntaxHighlighting(markdownStyle),
          livePreview,
          EditorView.lineWrapping,
          placeholder("Title"),
          keymap.of([...formatKeymap, ...defaultKeymap]),
          editable.of(EditorView.editable.of(false)),
          EditorView.contentAttributes.of({ "aria-label": "Note text", autocapitalize: "sentences", spellcheck: "true" }),
          EditorView.updateListener.of((u) => {
            if (u.focusChanged) focused = u.view.hasFocus;
            if (u.docChanged || u.selectionSet) ({ block, inline } = activeFormats(u.state));
          }),
          LoroExtensions(
            doc,
            { ephemeral, user: { name: app.user!.display_name, colorClassName: colorFor(app.user!.id) } },
            undoManager,
            (d) => d.getText("body"),
          ),
        ],
      }),
    });
    view = v;

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
        if (loaded) return;
        loaded = true;
        refreshEditable();
        if (app.freshNote === noteId && canEdit) {
          app.freshNote = null;
          v.focus();
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

    const vv = window.visualViewport;
    const onViewport = () => {
      if (vv) keyboard = Math.max(0, window.innerHeight - vv.height - vv.offsetTop);
    };
    vv?.addEventListener("resize", onViewport);
    vv?.addEventListener("scroll", onViewport);

    return () => {
      clearInterval(heartbeat);
      vv?.removeEventListener("resize", onViewport);
      vv?.removeEventListener("scroll", onViewport);
      // Like Apple Notes, a note left blank doesn't stick around.
      const blank = hasData && !lost && role === "owner" && doc.getText("body").toString().trim() === "";
      sync.close(noteId);
      if (blank) api.discardNote(noteId).catch(() => {});
      offDoc();
      offPresence();
      offPeers();
      v.destroy();
      ephemeral.destroy();
    };
  });

  async function remove() {
    if (!confirm("Move this note to the trash?")) return;
    await api.deleteNote(noteId);
    openNote(null);
  }
</script>

<section class:focused>
  <header>
    <button class="flat back phone-only" onclick={() => openNote(null)}>
      <Icon name="back" /><span>{viewTitle(app.view, app.tree)}</span>
    </button>
    <span class="title">{note?.title || "New Note"}</span>
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
    {#if focused}
      <button class="flat done phone-only" onclick={() => view?.contentDOM.blur()}>Done</button>
    {/if}
  </header>

  {#if canEdit && view}
    <div class="format" style:bottom="{keyboard}px">
      <FormatBar {view} {block} {inline} />
    </div>
  {/if}

  {#if lost}
    <div class="banner">
      {lost === "revoked" ? "You no longer have access to this note." : "This note doesn't exist or was moved to the trash."}
    </div>
  {:else if !loaded}
    <div class="banner dim">{app.status === "online" ? "Loading…" : "Waiting for the server…"}</div>
  {/if}

  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scroll" onclick={focusEnd}>
    <div class="page" class:hidden={!loaded} bind:this={parent}></div>
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

  .back {
    padding: 0 10px 0 6px;
    gap: 4px;
    color: var(--accent);
    font-weight: 400;
    max-width: 40%;
  }

  .back span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .done {
    color: var(--accent);
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

  .format {
    border-bottom: 1px solid var(--border);
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
    cursor: text;
  }

  .page {
    width: 100%;
    max-width: 760px;
    padding: 28px 24px 40vh;
  }

  .hidden {
    visibility: hidden;
  }

  .page :global(.cm-editor) {
    background: transparent;
    color: var(--fg);
    font-size: 1.05rem;
  }

  .page :global(.cm-editor.cm-focused) {
    outline: none;
  }

  .page :global(.cm-scroller) {
    overflow: visible;
    font-family: inherit;
    line-height: 1.55;
  }

  .page :global(.cm-content) {
    caret-color: var(--accent);
    padding: 0;
  }

  .page :global(.cm-line) {
    padding: 0;
  }

  .page :global(.cm-placeholder) {
    color: var(--dim-fg);
  }

  .page :global(.cm-h1) {
    font-size: 1.75em;
    font-weight: 800;
    line-height: 1.25;
    padding-bottom: 0.2em;
  }

  .page :global(.cm-h2) {
    font-size: 1.35em;
    font-weight: 800;
    padding-top: 0.4em;
  }

  .page :global(.cm-h3) {
    font-size: 1.12em;
    font-weight: 700;
    padding-top: 0.3em;
  }

  .page :global(.cm-quote) {
    border-left: 3px solid var(--border);
    padding-left: 12px;
    color: var(--dim-fg);
  }

  .page :global(.cm-bullet) {
    display: inline-block;
    width: 1.1em;
    color: var(--dim-fg);
  }

  .page :global(.cm-checkbox) {
    display: inline-block;
    width: 1.15em;
    height: 1.15em;
    margin-right: 0.45em;
    vertical-align: -0.2em;
    border: 1.5px solid var(--dim-fg);
    border-radius: 50%;
    cursor: pointer;
  }

  .page :global(.cm-checkbox.checked) {
    border-color: var(--accent-bg);
    background: var(--accent-bg)
      url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'%3E%3Cpath d='M4 8.5l2.5 2.5L12 5.5' fill='none' stroke='white' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E")
      center / 80% no-repeat;
  }

  .page :global(.cm-task-done) {
    color: var(--dim-fg);
    text-decoration: line-through;
  }

  /* Phone: the toolbar floats on top of the keyboard while typing, like Apple Notes. */
  @media (max-width: 700px) {
    .format {
      position: fixed;
      left: 0;
      right: 0;
      z-index: 5;
      border-top: 1px solid var(--border);
      border-bottom: none;
      padding-bottom: env(safe-area-inset-bottom);
      background: var(--headerbar-bg);
      display: none;
    }

    section.focused .format {
      display: block;
    }

    .page {
      padding: 20px 18px 50vh;
    }
  }
</style>
