<script lang="ts">
  import { onMount, untrack } from "svelte";
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
  import Menu from "./lib/Menu.svelte";
  import { app, colorFor, composeNote, goBack, navigate, trashNote, viewTitle } from "./lib/store.svelte";
  import { bloom, media, scrollEdge } from "./lib/ui.svelte";
  import { fly } from "svelte/transition";
  import FormatBar from "./FormatBar.svelte";
  import Recorder from "./Recorder.svelte";
  import { addImages, addRecording } from "./lib/attachments";
  import { toast } from "./lib/ui.svelte";
  import { ApiError } from "./lib/api";
  import ShareDialog from "./ShareDialog.svelte";

  const props: { noteId: string } = $props();
  const noteId = $derived(props.noteId);

  const note = $derived(app.tree.notes.find((n) => n.id === noteId));
  const notebook = $derived(note?.notebook_id ? app.tree.notebooks.find((n) => n.id === note.notebook_id) : undefined);

  /** Wide: show that notebook's list beside this note. Phone: go to the list. */
  function openNotebook() {
    if (!notebook) return;
    navigate({ kind: "notebook", id: notebook.id }, media.phone ? null : noteId);
  }
  let role = $state<Role | null>(null);
  let loaded = $state(false);
  let lost = $state<"revoked" | "not_found" | null>(null);
  let peers = $state<{ name: string; color: string }[]>([]);
  let sharing = $state(false);
  let recording = $state(false);
  let photoInput: HTMLInputElement;
  /** Captured when an add button is pressed: not typing means "add to the end of the note". */
  let addAtEnd = false;
  let adding = $state(0);
  let focused = $state(false);
  let block = $state<Block>("body");
  let inline = $state<Set<Inline>>(new Set());
  /** Height covered by the on-screen keyboard, so the phone toolbar can sit on top of it. */
  let keyboard = $state(0);
  /** The phone format bar's height, so text never ends up underneath it. */
  let formatHeight = $state(0);
  /** Phone, typing: the note's scroll area stops above the keyboard and the format bar. */
  const coveredBottom = $derived(media.phone && focused ? keyboard + formatHeight : 0);
  let view = $state<EditorView>();
  let parent: HTMLDivElement;
  let scroller: HTMLDivElement;
  /** Phone: space kept between the cursor and whatever covers the bottom of the note. */
  const CURSOR_ROOM = 120;

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
    // One editor per note: pin the id so teardown never sees the next note's id.
    const noteId = untrack(() => props.noteId);
    const v = new EditorView({
      parent,
      state: EditorState.create({
        extensions: [
          markdown({ base: markdownLanguage }),
          syntaxHighlighting(markdownStyle),
          livePreview,
          dropImages,
          EditorView.lineWrapping,
          placeholder("Title"),
          keymap.of([...formatKeymap, ...defaultKeymap]),
          // While typing, the text moves up a line at a time to keep this much room below the
          // cursor, so the line being written never touches the format bar or keyboard.
          EditorView.scrollMargins.of(() => ({ bottom: media.phone ? CURSOR_ROOM : 64, top: 24 })),
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

    // Android (interactive-widget=resizes-content) shrinks the layout to fit above the
    // keyboard, so this is 0 there. iOS overlays the keyboard; this is its height.
    const vv = window.visualViewport;
    const onViewport = () => {
      if (!vv) return;
      keyboard = Math.max(0, Math.round(window.innerHeight - vv.height - vv.offsetTop));
      // The keyboard opening shrinks the visible area; bring the cursor back above it.
      if (v.hasFocus) requestAnimationFrame(() => revealCursor(v));
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

  /**
   * Glides the cursor back into comfortable view when the keyboard opens or the visible
   * area changes. On phones that area ends above the format bar and keyboard.
   */
  function revealCursor(v: EditorView) {
    const caret = v.coordsAtPos(v.state.selection.main.head);
    if (!caret || !scroller) return;
    const box = scroller.getBoundingClientRect();
    const room = media.phone ? Math.min(CURSOR_ROOM, box.height * 0.3) : 64;
    const below = caret.bottom - (box.bottom - room);
    const above = box.top + 24 - caret.top;
    const top = below > 0 ? below : above > 0 ? -above : 0;
    if (top) scroller.scrollBy({ top, behavior: media.reduced ? "auto" : "smooth" });
  }

  // After the scroll area shrinks for the keyboard, re-center on the cursor.
  $effect(() => {
    coveredBottom;
    if (view?.hasFocus) requestAnimationFrame(() => view && revealCursor(view));
  });

  /** Adds a checklist item at the end of the note and starts typing in it. */
  function startChecklist() {
    if (!view) return;
    const { doc: text } = view.state;
    const last = text.line(text.lines);
    const insert = last.text.trim() === "" ? "- [ ] " : "\n- [ ] ";
    const from = last.text.trim() === "" ? last.from : text.length;
    view.dispatch({
      changes: { from, to: text.length, insert },
      selection: { anchor: from + insert.length },
      scrollIntoView: true,
    });
    view.focus();
  }

  function failed(err: unknown) {
    toast(err instanceof ApiError ? err.message : "Couldn't add that. Check your connection.");
  }

  async function withBusy(work: () => Promise<void>) {
    adding++;
    try {
      await work();
    } catch (err) {
      failed(err);
    } finally {
      adding--;
    }
  }

  function pickPhoto() {
    addAtEnd = !view?.hasFocus;
    photoInput.click();
  }

  function onPhotos(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const files = [...(input.files ?? [])];
    input.value = "";
    if (view && files.length) void withBusy(() => addImages(view!, noteId, files, addAtEnd));
  }

  function startRecording() {
    addAtEnd = !view?.hasFocus;
    view?.contentDOM.blur();
    recording = true;
  }

  function saveRecording(audio: File) {
    recording = false;
    if (!view) return;
    const v = view;
    void withBusy(async () => {
      await addRecording(v, noteId, audio, app.features.transcription, addAtEnd);
    });
  }

  /** Pasted or dropped images upload straight into the note. */
  const dropImages = EditorView.domEventHandlers({
    paste(e, v) {
      const files = [...(e.clipboardData?.files ?? [])].filter((f) => f.type.startsWith("image/"));
      if (!files.length || !canEdit) return false;
      e.preventDefault();
      void withBusy(() => addImages(v, noteId, files));
      return true;
    },
    drop(e, v) {
      const files = [...(e.dataTransfer?.files ?? [])].filter((f) => f.type.startsWith("image/"));
      if (!files.length || !canEdit) return false;
      e.preventDefault();
      const pos = v.posAtCoords({ x: e.clientX, y: e.clientY });
      if (pos !== null) v.dispatch({ selection: { anchor: pos } });
      void withBusy(() => addImages(v, noteId, files));
      return true;
    },
  });

  const toolbarIn = (node: Element) => fly(node, { y: 60, duration: media.reduced ? 0 : 220 });
</script>

<section class:focused style:padding-bottom="{coveredBottom}px">
  <div class="top">
    <header class="headerbar">
      <button class="flat back phone-only" onclick={goBack}>
        <Icon name="back" /><span>{viewTitle(app.view, app.tree)}</span>
      </button>
      <div class="title"><strong>{note?.title || "New Note"}</strong></div>
      <div class="peers">
        {#each peers as peer (peer.name)}
          <span class="avatar {peer.color}" title="{peer.name} is here" transition:bloom>{peer.name.slice(0, 1).toUpperCase()}</span>
        {/each}
      </div>
      {#if role === "viewer"}<span class="badge">View only</span>{/if}
      {#if focused && media.phone}
        <button class="suggested done" onclick={() => view?.contentDOM.blur()} transition:bloom>Done</button>
      {:else if role === "owner"}
        <button class="flat accent share wide-only" aria-label="Share" title="Share this note" onclick={() => (sharing = true)}>
          <Icon name="people" /><span>Share</span>
        </button>
        <Menu label="Note menu" items={[{ label: "Move to Trash", icon: "trash", destructive: true, onselect: () => trashNote(noteId) }]} />
      {/if}
    </header>

    {#if canEdit && view}
      <div class="format" style:bottom="{keyboard}px" bind:offsetHeight={formatHeight}>
        <FormatBar {view} {block} {inline} onphoto={pickPhoto} onrecord={startRecording} />
      </div>
    {/if}
  </div>

  {#if lost}
    <div class="lost">
      <div class="lost-icon"><Icon name={lost === "revoked" ? "people" : "trash"} size={36} /></div>
      <strong>{lost === "revoked" ? "No Longer Shared" : "Note Not Found"}</strong>
      <p class="dim">{lost === "revoked" ? "You no longer have access to this note." : "This note doesn't exist or was moved to the trash."}</p>
      <button class="pill" onclick={goBack}>Back to Notes</button>
    </div>
  {:else if !loaded}
    <div class="loading dim"><span class="spinner"></span>{app.status === "online" ? "Opening…" : "Waiting for the server…"}</div>
  {/if}

  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scroll" class:gone={lost} onclick={focusEnd} use:scrollEdge bind:this={scroller}>
    <div class="column" class:hidden={!loaded}>
      {#if notebook}
        <button class="flat notebook-chip" title="Open the {notebook.name} notebook" onclick={(e) => (e.stopPropagation(), openNotebook())}>
          <Icon name="folder" size={14} /><span>{notebook.name}</span>
        </button>
      {/if}
      <div class="page" class:with-chip={notebook} bind:this={parent}></div>
    </div>
  </div>

  {#if canEdit && !focused}
    <!-- Phone, not typing: a bottom toolbar with labeled actions. -->
    <div class="toolbar phone-only" transition:toolbarIn>
      <button class="flat tool" onclick={startChecklist}><Icon name="checklist" /><span>Checklist</span></button>
      <button class="flat tool" onclick={pickPhoto}><Icon name="camera" /><span>Photo</span></button>
      <button class="flat tool" onclick={startRecording}><Icon name="mic" /><span>Record</span></button>
      {#if role === "owner"}
        <button class="flat tool" onclick={() => (sharing = true)}><Icon name="people" /><span>Share</span></button>
      {/if}
      <button class="flat tool" onclick={() => composeNote()}><Icon name="compose" /><span>New Note</span></button>
    </div>
  {/if}
</section>

<input bind:this={photoInput} class="file" type="file" accept="image/*" multiple onchange={onPhotos} aria-hidden="true" tabindex="-1" />

{#if adding}
  <div class="adding" transition:bloom><span class="spinner"></span>Adding…</div>
{/if}

{#if recording}
  <Recorder onsave={saveRecording} onclose={() => (recording = false)} />
{/if}

{#if sharing}
  <ShareDialog kind="note" id={noteId} name={note?.title ?? ""} onclose={() => (sharing = false)} />
{/if}

<style>
  section {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--view-bg);
  }

  .top {
    position: relative;
    z-index: 2;
    background: var(--view-bg);
    box-shadow: 0 1px 0 transparent;
    transition: box-shadow var(--fast) ease;
  }

  section:global([data-scrolled]) > .top {
    box-shadow: 0 1px 0 var(--border);
  }

  .back {
    max-width: 40%;
  }

  .back span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .done {
    min-height: 34px !important;
    padding: 0 16px;
    border-radius: 999px;
  }

  .share {
    gap: 6px;
    padding: 0 12px;
  }

  .toolbar {
    display: flex;
    justify-content: space-around;
    padding: 4px 8px calc(4px + env(safe-area-inset-bottom));
    border-top: 1px solid var(--border);
    background: color-mix(in srgb, var(--headerbar-bg) 92%, transparent);
    backdrop-filter: blur(12px);
  }

  .tool {
    flex: 1;
    flex-direction: column;
    gap: 3px;
    min-height: 54px;
    padding: 4px 0;
    color: var(--accent);
    font-size: 0.75rem;
    font-weight: 600;
  }

  .tool :global(svg) {
    width: 22px;
    height: 22px;
  }

  .peers {
    display: flex;
    padding: 0 6px;
  }

  .peers .avatar {
    width: 28px;
    height: 28px;
    font-size: 0.8rem;
    box-shadow: 0 0 0 2px var(--view-bg);
  }

  .peers .avatar + .avatar {
    margin-left: -6px;
  }

  .badge {
    margin: 0 6px;
    padding: 3px 10px;
    border-radius: 999px;
    background: var(--hover);
    color: var(--dim-fg);
    font-size: 0.8rem;
    font-weight: 700;
    white-space: nowrap;
  }

  /* Wide: the formatting toolbar is a soft rounded strip under the headerbar. */
  .format {
    display: flex;
    justify-content: center;
    padding: 0 12px 8px;
  }

  .loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 48px 16px 0;
    animation: rise 300ms var(--ease-out) 150ms both;
  }

  .lost {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 64px 24px 0;
    text-align: center;
    animation: rise 260ms var(--ease-out) both;
  }

  .lost-icon {
    display: grid;
    place-items: center;
    width: 72px;
    height: 72px;
    margin-bottom: 10px;
    border-radius: 50%;
    background: var(--hover);
    color: var(--dim-fg);
  }

  .lost strong {
    font-size: 1.15rem;
  }

  .lost p {
    margin: 0 0 16px;
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    display: flex;
    justify-content: center;
    /* Not stretched: the page grows with its text, so its bottom padding always leaves room. */
    align-items: flex-start;
    cursor: text;
  }

  .scroll.gone {
    display: none;
  }

  .file {
    display: none;
  }

  .adding {
    position: fixed;
    top: calc(64px + env(safe-area-inset-top));
    left: 50%;
    z-index: 20;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 16px;
    translate: -50% 0;
    border-radius: 999px;
    background: var(--popover-bg);
    box-shadow: var(--shadow-md);
    font-weight: 600;
    font-size: 0.9rem;
  }

  .adding .spinner {
    width: 14px;
    height: 14px;
    color: var(--accent);
  }

  /* Photos and voice memos embedded in the text. */
  .page :global(.cm-embed-line) {
    padding: 6px 0 !important;
    font-size: 1rem !important;
    font-weight: 400 !important;
  }

  .page :global(.cm-attachment) {
    display: block;
    width: fit-content;
    max-width: 100%;
    border-radius: 12px;
    cursor: default;
    animation: rise 220ms var(--ease-out) both;
  }

  .page :global(.cm-attachment.loading) {
    width: min(100%, 320px);
    height: 120px;
    background: var(--hover);
    animation: pulse-bg 1.2s ease-in-out infinite;
  }

  @keyframes -global-pulse-bg {
    50% {
      opacity: 0.5;
    }
  }

  .page :global(.cm-attachment.missing) {
    padding: 12px 16px;
    background: var(--hover);
    color: var(--dim-fg);
    font-size: 0.9rem;
  }

  .page :global(.cm-attachment img) {
    display: block;
    max-width: 100%;
    max-height: 70vh;
    border-radius: 12px;
    box-shadow: 0 0 0 1px var(--border);
    cursor: zoom-in;
  }

  .page :global(.cm-audio) {
    display: flex;
    align-items: center;
    gap: 12px;
    width: min(100vw - 48px, 380px);
    padding: 10px 14px 10px 10px;
    border-radius: 14px;
    background: var(--card-bg);
    box-shadow: var(--shadow-sm), 0 0 0 1px var(--border);
    user-select: none;
  }

  .page :global(.cm-audio-play) {
    flex: none;
    width: 40px;
    height: 40px;
    min-height: 40px;
    padding: 0;
    border-radius: 50%;
    background: var(--accent-bg);
    color: #fff;
  }

  /* Play triangle, or pause bars while playing. */
  .page :global(.cm-audio-play::before) {
    content: "";
    width: 0;
    height: 0;
    margin-left: 3px;
    border-style: solid;
    border-width: 7px 0 7px 12px;
    border-color: transparent transparent transparent currentColor;
  }

  .page :global(.cm-audio.playing .cm-audio-play::before) {
    width: 12px;
    height: 14px;
    margin-left: 0;
    border: none;
    border-left: 4px solid currentColor;
    border-right: 4px solid currentColor;
  }

  .page :global(.cm-audio-body) {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .page :global(.cm-audio-title) {
    font-weight: 700;
    font-size: 0.92rem;
    line-height: 1.1;
  }

  .page :global(.cm-audio-bar) {
    height: 5px;
    border-radius: 3px;
    background: var(--active);
    cursor: pointer;
    overflow: hidden;
  }

  .page :global(.cm-audio-bar div) {
    height: 100%;
    width: 0;
    background: var(--accent-bg);
    transition: width 250ms linear;
  }

  .page :global(.cm-audio-time) {
    font-size: 0.85rem;
    font-variant-numeric: tabular-nums;
    color: var(--dim-fg);
  }

  .page :global(.cm-audio-status) {
    display: none;
  }

  .page :global(.transcribing .cm-audio) {
    flex-wrap: wrap;
  }

  .page :global(.transcribing .cm-audio-status) {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding-left: 52px;
    font-size: 0.85rem;
    color: var(--dim-fg);
  }

  .page :global(.transcribing .cm-audio-status::before) {
    content: "";
    width: 12px;
    height: 12px;
    border: 2px solid color-mix(in srgb, currentColor 25%, transparent);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .page :global(.transcribing .cm-audio-status::after) {
    content: "Transcribing…";
  }

  .column {
    width: 100%;
    max-width: 760px;
    transition: opacity 220ms ease;
  }

  .page {
    padding: 28px 24px 40vh;
  }

  .page.with-chip {
    padding-top: 10px;
  }

  /* Which notebook this note lives in; tap to open it. */
  .notebook-chip {
    gap: 6px;
    min-height: 28px;
    margin: 20px 0 0 18px;
    padding: 0 12px 0 10px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 0.85rem;
    font-weight: 700;
    max-width: calc(100% - 36px);
  }

  .notebook-chip span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hidden {
    opacity: 0;
    pointer-events: none;
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

  /* Phone: the formatting bar floats on top of the keyboard while typing. */
  @media (max-width: 700px) {
    header .title {
      visibility: hidden;
    }

    .format {
      position: fixed;
      left: 0;
      right: 0;
      z-index: 5;
      padding: 0;
      border-top: 1px solid var(--border);
      /* No safe-area padding: the bar only shows while typing, sitting on the keyboard. */
      background: var(--headerbar-bg);
      display: none;
    }

    section.focused .format {
      display: block;
      animation: rise 180ms var(--ease-out);
    }

    .page {
      padding: 20px 18px 50vh;
    }

    .page.with-chip {
      padding-top: 8px;
    }

    .notebook-chip {
      margin: 14px 0 0 12px;
    }
  }
</style>
