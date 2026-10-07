<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { Compartment, EditorSelection, EditorState } from "@codemirror/state";
  import { EditorView, keymap } from "@codemirror/view";
  import { defaultKeymap } from "@codemirror/commands";
  import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
  import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
  import { tags } from "@lezer/highlight";
  import { EphemeralStore, LoroDoc, UndoManager, type VersionVector } from "loro-crdt";
  import { LoroExtensions, getCursorEphemeralKey, getUserEphemeralKey } from "loro-codemirror";
  import { api, type Role } from "./lib/api";
  import { activeFormats, formatKeymap, type Block, type Inline } from "./lib/format";
  import Icon from "./lib/Icon.svelte";
  import { codeBlocks } from "./lib/codeBlocks";
  import { linkClicks, livePreview, setWikiNotes } from "./lib/livePreview";
  import { slashMenu } from "./lib/slash";
  import { dragHandles } from "./lib/dragHandles";
  import { undoCommands } from "./lib/undo";
  import { blame } from "./lib/blame";
  import { emojiOnColon } from "./lib/emoji";
  import { tables } from "./lib/tables";
  import { kanban } from "./lib/kanban";
  import { sync, type NoteHandler } from "./lib/sync";
  import { cache, dropQueued, editing, flush, isQueued, whenCreated } from "./lib/offline";
  import Fab from "./lib/Fab.svelte";
  import type { MenuItem } from "./lib/Menu.svelte";
  import StatusPage from "./lib/StatusPage.svelte";
  import { app, colorFor, composeNote, goBack, navigate, pathOf, retitle, saveTree, trashNote, twoPane, viewTitle, type View } from "./lib/store.svelte";
  import { bloom, media, scrollEdge } from "./lib/ui.svelte";
  import FormatBar from "./FormatBar.svelte";
  import Recorder from "./Recorder.svelte";
  import { addImages, addRecording, recordingMarker, RecordingSpot } from "./lib/attachments";
  import type { LiveHandlers } from "./lib/live";
  import { toast } from "./lib/ui.svelte";
  import { ApiError } from "./lib/api";
  import MoveDialog from "./MoveDialog.svelte";
  import ShareDialog from "./ShareDialog.svelte";
  import Dialog from "./lib/Dialog.svelte";
  import { changesBetween } from "./lib/tidy";
  import { aiPrompt, openAiPrompt } from "./lib/aiPrompt";

  const props: { noteId: string } = $props();
  const noteId = $derived(props.noteId);

  const note = $derived(app.tree.notes.find((n) => n.id === noteId));
  const notebook = $derived(note?.notebook_id ? app.tree.notebooks.find((n) => n.id === note.notebook_id) : undefined);

  /** Wide: show that notebook's list beside this note. Phone: go to the list. */
  /** Where the note lives, as a chip above it. Loose notes only say so when opened from elsewhere. */
  const place = $derived(
    notebook
      ? { label: pathOf(notebook.id).join(" › "), icon: "folder" as const, view: { kind: "notebook", id: notebook.id } as View }
      : note?.role === "owner" && app.view.kind !== "root"
        ? { label: "Notes", icon: "home" as const, view: { kind: "root" } as View }
        : null,
  );
  function openPlace() {
    if (place) navigate(place.view, media.phone ? null : noteId);
  }
  let role = $state<Role | null>(null);
  let loaded = $state(false);
  let lost = $state<"revoked" | "not_found" | null>(null);
  let peers = $state<{ name: string; color: string }[]>([]);
  let sharing = $state(false);
  let moving = $state(false);
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
  /** Phone: the title line has scrolled away, so the headerbar names the note instead. */
  let compact = $state(false);
  function onScroll() {
    const title = parent?.querySelector(".cm-line");
    compact = !!title && title.getBoundingClientRect().bottom < scroller.getBoundingClientRect().top;
  }

  /** Opened from the copy saved on this device, before (or without) the server answering. */
  let fromCache = $state(false);
  /** Until the server says, a saved copy edits with the role the saved tree gives. */
  const effectiveRole = $derived(role ?? (fromCache ? (note?.role ?? null) : null));
  const canEdit = $derived(loaded && !lost && effectiveRole !== null && effectiveRole !== "viewer");
  $effect(() => {
    void canEdit;
    untrack(refreshEditable);
  });

  const doc = new LoroDoc();
  // Times on each change, so who-wrote-what can say when.
  doc.setRecordTimestamp(true);
  /** Others can edit this note, so lines they wrote get a bar in their color. */
  const shared = $derived(!!note && (note.shared || note.role !== "owner"));
  $effect(() => {
    void shared;
    untrack(() => view?.dispatch({}));
  });
  // [[links]] resolve against the notes the reader can see; the editor keeps its list in step with the tree.
  function pushWikiNotes() {
    if (!view) return;
    const map = new Map<string, string>();
    for (const n of app.tree.notes) {
      const t = n.title.trim().toLowerCase();
      if (t && !map.has(t)) map.set(t, n.id);
    }
    view.dispatch({ effects: setWikiNotes.of(map) });
  }
  $effect(() => {
    void app.tree.notes;
    untrack(pushWikiNotes);
  });
  const ephemeral = new EphemeralStore(30_000);
  const undoManager = new UndoManager(doc, {});
  /** Whether there's anything to undo or redo, for the format bar's buttons. */
  let history = $state({ undo: false, redo: false });
  const refreshHistory = () => (history = { undo: undoManager.canUndo(), redo: undoManager.canRedo() });
  const steps = undoCommands(undoManager, refreshHistory);
  const editable = new Compartment();
  let hasData = false;

  // Offline copies (lib/offline.ts): the doc is saved on the device a moment after each change. A note
  // is "dirty" while it may have edits the server lacks: typed with no connection, or sent just before
  // one dropped. A dirty note left closed is pushed up on the next reconnect.
  const ownId = untrack(() => props.noteId);
  let editedSinceJoin = false;
  let dirty = false;
  function markDirty() {
    if (dirty) return;
    dirty = true;
    void cache.markDirty(ownId);
  }
  $effect(() => {
    if (app.status === "offline") untrack(() => editedSinceJoin && markDirty());
  });
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  function save() {
    clearTimeout(saveTimer);
    saveTimer = undefined;
    if (!hasData || lost) return;
    void cache.saveNote(ownId, doc);
    // The server retitles notes as they change; offline, the lists follow here instead.
    if (app.status !== "online") retitle(ownId, doc.getText("body").toString());
  }

  /** Content is here, from the server or the device: the note shows and, if allowed, takes typing. */
  function ready() {
    if (loaded) return;
    loaded = true;
    refreshEditable();
    if (app.freshNote === ownId && canEdit) {
      // Made to hold a memo or photos (the tab bar's long-press menu): start on that instead of typing.
      const add = app.freshAdd;
      app.freshNote = app.freshAdd = null;
      if (add === "memo") startRecording();
      else if (add && view) void withBusy(() => addImages(view!, ownId, add, true));
      else view?.focus();
    }
  }

  const markdownStyle = HighlightStyle.define([
    { tag: [tags.heading1, tags.heading2, tags.heading3, tags.heading4, tags.heading5, tags.heading6], fontWeight: "inherit" },
    { tag: tags.strong, fontWeight: "700" },
    { tag: tags.emphasis, fontStyle: "italic" },
    { tag: tags.strikethrough, textDecoration: "line-through" },
    { tag: [tags.link, tags.url], color: "var(--accent)" },
    { tag: tags.monospace, fontFamily: "'Adwaita Mono', 'Source Code Pro', monospace", fontSize: "0.92em" },
    { tag: [tags.processingInstruction, tags.contentSeparator, tags.meta], color: "var(--dim-fg)" },
  ]);

  /** A note's look, read-only: for its summary and Tidy Up's preview. */
  const readOnlyLook = (label: string) => [
    markdown({ base: markdownLanguage }),
    syntaxHighlighting(markdownStyle),
    livePreview,
    codeBlocks,
    tables,
    kanban,
    EditorView.lineWrapping,
    EditorView.editable.of(false),
    EditorState.readOnly.of(true),
    EditorView.contentAttributes.of({ "aria-label": label }),
  ];

  // Summary, from the note's actions: an AI summary of the whole note, made the first time it's asked
  // for (and again on request), shown read-only with the note's own styling in a sheet over the note.
  let summaryOpen = $state(false);
  let summary = $state<{ text: string; stale: boolean; at: number } | null>(null);
  let summarizing = $state(false);
  let summaryError = $state("");
  let summaryParent = $state<HTMLDivElement>();

  function keep(s: { summary: string | null; stale?: boolean; created_at?: number }) {
    summary = s.summary ? { text: s.summary, stale: !!s.stale, at: s.created_at ?? Date.now() } : null;
  }

  async function makeSummary() {
    summarizing = true;
    summaryError = "";
    try {
      keep(await api.summarize(noteId));
    } catch (err) {
      summaryError = err instanceof ApiError ? err.message : "Couldn't reach the server";
    }
    summarizing = false;
  }

  // Fetched up front, so the action can say whether it shows a summary or makes one.
  $effect(() => {
    if (!app.features.summaries) return;
    const id = noteId;
    untrack(() => api.summary(id).then((s) => { if (!summarizing && s.summary) keep(s); }, () => {}));
  });

  /** Opening it is the trigger: it shows the saved summary, or makes one when there's none. */
  async function openSummary() {
    summaryOpen = true;
    view?.contentDOM.blur();
    if (summarizing) return;
    try {
      const saved = await api.summary(noteId);
      if (saved.summary) keep(saved);
      else await makeSummary();
    } catch (err) {
      summaryError = err instanceof ApiError ? err.message : "Couldn't reach the server";
    }
  }

  $effect(() => {
    const text = summary?.text;
    const el = summaryParent;
    if (!text || !el) return;
    // Under the note's title, so the first line gets the title style and the summary reads as body text.
    const doc = `${untrack(() => note?.title) || "Summary"}\n${text}`;
    const shown = untrack(() => new EditorView({ parent: el, state: EditorState.create({ doc, extensions: readOnlyLook("Summary") }) }));
    return () => shown.destroy();
  });

  // Tidy Up and Edit with AI: the summary chat model rewrites the note. Tidy Up improves its structure
  // and formatting, and the server makes sure nothing was lost; Edit with AI changes it as you ask. The
  // result is shown rendered, and Apply writes it in as the smallest edits that get there, so it merges
  // like typing and one Undo takes it back.
  const canAI = $derived(canEdit && app.features.summaries);
  type AiEdit = { kind: "tidy" | "edit"; asking: boolean; busy: boolean; error: string; instruction: string; original: string; text: string };
  let aiEdit = $state<AiEdit | null>(null);
  /** What Edit with AI is asked to do, as typed. */
  let instruction = $state("");
  let aiParent = $state<HTMLDivElement>();
  /** Which request is current, so an answer for a closed dialog is dropped. */
  let aiRun = 0;
  const SUGGESTIONS = ["Make it all bullets", "Add action items at the end", "Fix spelling and grammar"];

  async function runAiEdit(kind: AiEdit["kind"], asked = "") {
    if (!view) return;
    const run = ++aiRun;
    const original = view.state.doc.toString();
    aiEdit = { kind, asking: false, busy: true, error: "", instruction: asked, original, text: "" };
    view.contentDOM.blur();
    try {
      const res = kind === "tidy" ? await api.tidy(noteId, original) : await api.editWithAI(noteId, asked, original);
      if (run !== aiRun || !aiEdit) return;
      if (res.text === res.original) {
        aiEdit = null;
        toast(kind === "tidy" ? "This note is already tidy" : "The AI left the note as it was");
      } else aiEdit = { ...aiEdit, busy: false, original: res.original, text: res.text };
    } catch (err) {
      if (run === aiRun && aiEdit) aiEdit = { ...aiEdit, busy: false, error: err instanceof ApiError ? err.message : "Couldn't reach the server" };
    }
  }

  const startTidy = () => runAiEdit("tidy");

  /** Edit with AI starts by asking what to change. */
  function startEdit() {
    aiRun++;
    view?.contentDOM.blur();
    instruction = "";
    aiEdit = { kind: "edit", asking: true, busy: false, error: "", instruction: "", original: "", text: "" };
  }

  function askEdit(asked: string) {
    if (asked.trim()) void runAiEdit("edit", asked.trim());
  }

  const tryAgain = (t: AiEdit) => void runAiEdit(t.kind, t.instruction);

  function closeAiEdit() {
    aiRun++;
    aiEdit = null;
  }

  function applyAiEdit() {
    const t = aiEdit;
    closeAiEdit();
    if (!view || !t?.text) return;
    if (view.state.doc.toString() !== t.original) return toast(`The note changed meanwhile. Try ${t.kind === "tidy" ? "Tidy Up" : "Edit with AI"} again.`);
    // Its own undo step, never merged with an edit just before it, like the slash command's.
    undoManager.setMergeInterval(0);
    view.dispatch({ changes: changesBetween(t.original, t.text), userEvent: `input.${t.kind}` });
    undoManager.setMergeInterval(1000);
    toast(t.kind === "tidy" ? "Note tidied up" : "Note edited", { label: "Undo", run: steps.undo });
  }

  // A preview that's ready puts focus on Apply, the suggested choice, rather than leaving it on Cancel.
  let applyButton = $state<HTMLButtonElement>();
  $effect(() => {
    if (aiEdit?.text && !aiEdit.busy) applyButton?.focus();
  });

  $effect(() => {
    const text = aiEdit?.text;
    const el = aiParent;
    if (!text || !el) return;
    const label = untrack(() => (aiEdit?.kind === "tidy" ? "Tidied note" : "Edited note"));
    const preview = untrack(() => new EditorView({ parent: el, state: EditorState.create({ doc: text, extensions: readOnlyLook(label) }) }));
    return () => preview.destroy();
  });

  // The AI Prompt block (lib/aiPrompt.ts): writes at a spot in the note, with the note as context.
  const promptActions = {
    async write(prompt: string, text: string, at: number) {
      try {
        return (await api.writeWithAI(noteId, prompt, text, at)).text;
      } catch (err) {
        toast(err instanceof ApiError ? err.message : "Couldn't reach the server");
        throw err;
      }
    },
    commit(v: EditorView, spec: Parameters<EditorView["dispatch"]>[0]) {
      // Its own undo step, like Tidy Up's.
      undoManager.setMergeInterval(0);
      v.dispatch(spec);
      undoManager.setMergeInterval(1000);
    },
  };

  /**
   * The note's actions, in the floating button: adding a photo or memo (reachable without the keyboard
   * up), then AI, then Move to and Trash.
   */
  const actions = $derived<MenuItem[]>([
    ...(canEdit
      ? [
          { label: "Photo", icon: "camera" as const, onselect: pickPhoto },
          { label: "Voice Memo", icon: "mic" as const, onselect: startRecording },
        ]
      : []),
    ...(app.features.summaries && loaded && !lost ? [{ label: summary ? "Summary" : "Summarize", icon: "note" as const, onselect: () => void openSummary() }] : []),
    ...(canAI
      ? [
          { label: "Edit with AI", icon: "sparkle" as const, onselect: startEdit },
          { label: "Tidy Up", icon: "broom" as const, onselect: () => void startTidy() },
        ]
      : []),
    ...(role && role !== "viewer" ? [{ label: "Move to…", icon: "move" as const, onselect: () => (moving = true) }] : []),
    ...(role === "owner" ? [{ label: "Move to Trash", icon: "trash" as const, destructive: true, onselect: () => trashNote(noteId) }] : []),
  ]);

  const madeAt = (ms: number) => new Date(ms).toLocaleString([], { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });

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

  /**
   * Tapping blank space below the text puts the cursor at the end, like a sheet of paper; above it,
   * at the start. Beside the text is part of the editor itself, so it never gets here.
   */
  function focusEnd(e: MouseEvent) {
    if (!view || !canEdit || view.contentDOM.contains(e.target as Node)) return;
    const above = e.clientY < view.contentDOM.getBoundingClientRect().top;
    view.dispatch({ selection: EditorSelection.cursor(above ? 0 : view.state.doc.length), scrollIntoView: true });
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
          linkClicks,
          codeBlocks,
          slashMenu({ photo: () => pickPhoto(), record: () => startRecording(), tidy: () => void startTidy(), prompt: openAiPrompt, canAI: () => canAI }),
          aiPrompt(promptActions),
          dragHandles,
          blame(doc, noteId, () => shared),
          emojiOnColon,
          tables,
          kanban,
          recordingMarker,
          dropImages,
          EditorView.lineWrapping,
          steps.keymap,
          keymap.of([...formatKeymap, ...defaultKeymap]),
          // While typing, the text moves up a line at a time to keep this much room below the
          // cursor, so the line being written never touches the format bar or keyboard.
          EditorView.scrollMargins.of(() => ({ bottom: media.phone ? CURSOR_ROOM : 64, top: 24 })),
          editable.of(EditorView.editable.of(false)),
          EditorView.contentAttributes.of({ "aria-label": "Note text", autocapitalize: "sentences", spellcheck: "true" }),
          EditorView.updateListener.of((u) => {
            if (u.focusChanged) syncFocus();
            if (u.docChanged || u.selectionSet) ({ block, inline } = activeFormats(u.state));
            if (u.docChanged) refreshHistory();
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
    pushWikiNotes();
    // Focus moving between the text and a field inside it (the AI Prompt block).
    const onFocusMove = () => setTimeout(syncFocus);
    v.dom.addEventListener("focusin", onFocusMove);
    v.dom.addEventListener("focusout", onFocusMove);

    const offDoc = doc.subscribeLocalUpdates((bytes) => {
      editedSinceJoin = true;
      if (!sync.sendUpdate(noteId, bytes)) markDirty();
    });
    const offSave = doc.subscribe(() => {
      saveTimer ??= setTimeout(save, 800);
    });
    const offPresence = ephemeral.subscribeLocalUpdates((bytes) => sync.sendPresence(noteId, bytes));
    const offPeers = ephemeral.subscribe(updatePeers);

    const handler: NoteHandler = {
      version: () => (hasData ? doc.oplogVersion() : null),
      joined(r: Role, server: VersionVector) {
        setRole(r);
        editedSinceJoin = false;
        // Upload anything typed while disconnected. It stays dirty until a later join shows the server has it.
        const cmp = doc.oplogVersion().compare(server);
        if (hasData && r !== "viewer" && (cmp === undefined || cmp > 0)) {
          sync.sendUpdate(noteId, doc.export({ mode: "update", from: server }));
        } else if (dirty) {
          dirty = false;
          void cache.clearDirty(noteId);
        }
      },
      update(data) {
        doc.import(data);
        hasData = true;
        ready();
      },
      presence: (data) => ephemeral.apply(data),
      role: setRole,
      lost(reason) {
        lost = reason;
        refreshEditable();
        void cache.forget(noteId);
      },
      outOfSync: () => sync.rejoin(noteId),
    };

    // Open from the device's copy first, so the note shows with no network and the join carries its
    // version. A note made offline waits for the server to have it before joining.
    editing.add(noteId);
    let alive = true;
    void (async () => {
      const saved = await cache.note(noteId);
      if (!alive) return;
      if (saved && !hasData) {
        doc.import(saved);
        hasData = fromCache = true;
      } else if (isQueued(noteId)) {
        hasData = fromCache = true;
      }
      if (hasData) ready();
      await whenCreated(noteId);
      if (alive) sync.open(noteId, handler);
    })();

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
      const blank = hasData && !lost && effectiveRole === "owner" && doc.getText("body").toString().trim() === "";
      alive = false;
      clearTimeout(saveTimer);
      if (!blank) save();
      editing.delete(noteId);
      sync.close(noteId, handler);
      if (blank && isQueued(noteId)) {
        void dropQueued(noteId);
        app.tree.notes = app.tree.notes.filter((n) => n.id !== noteId);
        saveTree();
      } else if (blank) {
        void cache.forget(noteId);
        api.discardNote(noteId).catch(() => {});
      } else if (dirty && app.status === "online") {
        // Left before the server confirmed it has everything: check, and push the rest, without the editor.
        void flush();
      }
      offDoc();
      offSave();
      offPresence();
      offPeers();
      v.destroy();
      ephemeral.destroy();
    };
  });

  /** Typing: the cursor in the note, or in its AI Prompt block. */
  function syncFocus() {
    if (!view) return;
    app.typing = focused = view.hasFocus || !!document.activeElement?.closest(".cm-ai-prompt");
  }

  /** Done typing: the keyboard goes away, from the note or a field in it. */
  function blurNote() {
    if (view?.dom.contains(document.activeElement)) (document.activeElement as HTMLElement).blur();
    view?.contentDOM.blur();
  }

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

  // Leaving the note while typing gives the tab bar back.
  $effect(() => () => (app.typing = false));

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

  /** Where the recording in progress goes, picked when it starts and shown with a marker. */
  let spot: RecordingSpot | null = null;
  const liveHandlers: LiveHandlers = {
    ondelta: (text) => spot?.delta(text),
    onfinal: (text) => spot?.final(text),
    onerror: (message) => toast(message),
  };

  function startRecording() {
    if (!view || recording) return;
    // The cursor's line when you were typing, else the end of the note.
    spot = new RecordingSpot(view, !view.hasFocus, app.features.live_transcription ? "Listening…" : "Recording…");
    view.contentDOM.blur();
    recording = true;
  }

  function saveRecording(audio: File) {
    recording = false;
    const s = spot;
    spot = null;
    if (!view || !s) return;
    const v = view;
    void withBusy(() => addRecording(v, noteId, audio, s, app.features.transcription));
  }

  function discardRecording() {
    recording = false;
    spot?.remove();
    spot = null;
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

</script>

<section class:focused style:padding-bottom="{coveredBottom}px">
  <div class="top">
    <header class="headerbar">
      <!-- Phones, and wide screens in a folder where the note covers the folder's page. -->
      {#if media.phone || twoPane()}
        <button class="flat icon circular back-icon" title="Back to {viewTitle(app.view, app.tree)}" aria-label="Back to {viewTitle(app.view, app.tree)}" onclick={goBack}>
          <Icon name="back" />
        </button>
      {/if}
      <div class="title" class:shown={compact} aria-hidden={media.phone && !compact}><strong>{note?.title || "New Note"}</strong></div>
      <div class="peers">
        {#each peers as peer (peer.name)}
          <span class="avatar small {peer.color}" title="{peer.name} is here" transition:bloom>{peer.name.slice(0, 1).toUpperCase()}</span>
        {/each}
      </div>
      {#if role === "viewer"}<span class="badge">View only</span>{/if}
      {#if focused && media.phone}
        <button class="suggested done" onclick={blurNote} transition:bloom>Done</button>
      {:else if role === "owner"}
        <button class="flat share" class:icon={media.phone} aria-label="Share" title="Share this note" onclick={() => (sharing = true)}>
          <Icon name="share" />{#if !media.phone}<span>Share</span>{/if}
        </button>
      {/if}
      {#if twoPane()}
        <!-- The folder's New note button is hidden under the note, so it moves up here. -->
        <button class="suggested icon new" title="New note" aria-label="New note" onclick={() => composeNote()}>
          <Icon name="compose" />
        </button>
      {/if}
    </header>

    {#if canEdit && view}
      <div class="format" style:bottom="{keyboard}px" bind:offsetHeight={formatHeight}>
        <FormatBar {view} {block} {inline} {history} onundo={steps.undo} onredo={steps.redo} onphoto={pickPhoto} onrecord={startRecording} />
      </div>
    {/if}
  </div>

  {#if lost}
    <div class="lost">
      <StatusPage
        icon={lost === "revoked" ? "people" : "trash"}
        title={lost === "revoked" ? "No Longer Shared" : "Note Not Found"}
        description={lost === "revoked" ? "You no longer have access to this note." : "This note doesn't exist or was moved to the trash."}
        tone="neutral"
      >
        <button class="pill" onclick={goBack}>Back to Notes</button>
      </StatusPage>
    </div>
  {:else if !loaded}
    <div class="loading dim"><span class="spinner"></span>{app.status === "online" ? "Opening…" : "Waiting for the server…"}</div>
  {/if}

  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scroll" class:gone={lost} onclick={focusEnd} onscroll={onScroll} use:scrollEdge bind:this={scroller}>
    <div class="column" class:hidden={!loaded}>
      {#if place}
        <!-- Where the note lives. -->
        <div class="note-head">
          <button class="chip-link notebook-chip" title="Open {place.label}" onclick={(e) => (e.stopPropagation(), openPlace())}>
            <Icon name={place.icon} size={14} /><span>{place.label}</span><Icon name="next" size={12} />
          </button>
        </div>
      {/if}
      <div class="page" class:with-chip={place} class:can-edit={canEdit}
        class:can-read={canEdit && app.features.photo_text}
        class:can-transcribe={canEdit && app.features.transcription}
        bind:this={parent}></div>
    </div>
  </div>
  {#if recording}
    <!-- Floats over the note, above the phone's format bar and keyboard, so the transcript stays readable. -->
    <Recorder
      live={app.features.live_transcription ? liveHandlers : null}
      bottom={media.phone ? keyboard + formatHeight + 12 : 24}
      onsave={saveRecording}
      onclose={discardRecording}
    />
  {/if}
  <!-- The note's actions. On phones it steps aside while you type. -->
  {#if actions.length && loaded && !lost && !recording && !(focused && media.phone)}
    <Fab label="Note actions" icon="more" items={actions} />
  {/if}
</section>

<input bind:this={photoInput} class="file" type="file" accept="image/*" multiple onchange={onPhotos} aria-hidden="true" tabindex="-1" />

{#if adding}
  <div class="adding" transition:bloom><span class="spinner"></span>Adding…</div>
{/if}


{#if moving}
  <MoveDialog kind="note" id={noteId} name={note?.title ?? ""} onclose={() => (moving = false)} />
{/if}

{#if aiEdit}
  <Dialog title={aiEdit.kind === "tidy" ? "Tidy Up" : "Edit with AI"} wide onclose={closeAiEdit}>
    {#if aiEdit.asking}
      <form id="ai-edit" class="ai-ask" onsubmit={(e) => (e.preventDefault(), askEdit(instruction))}>
        <!-- svelte-ignore a11y_autofocus -->
        <input bind:value={instruction} placeholder="Describe a change…" aria-label="Change to make" maxlength="1000" autofocus />
        <div class="ai-chips">
          {#each SUGGESTIONS as suggestion (suggestion)}
            <button type="button" class="ai-chip" onclick={() => askEdit(suggestion)}>{suggestion}</button>
          {/each}
        </div>
      </form>
    {:else if aiEdit.busy}
      <div class="tidy-state dim" aria-live="polite"><span class="spinner"></span>{aiEdit.kind === "tidy" ? "Tidying up…" : "Editing…"}</div>
    {:else if aiEdit.error}
      <p class="tidy-state error-text">{aiEdit.error}</p>
    {:else}
      <p class="tidy-hint dim">
        {#if aiEdit.kind === "tidy"}Rearranged and formatted with AI. Everything in the note is still there.{:else}Changed with AI: “{aiEdit.instruction}”{/if}
      </p>
      <div class="page tidy-page" bind:this={aiParent}></div>
    {/if}
    {#snippet actions()}
      <button onclick={closeAiEdit}>Cancel</button>
      {#if aiEdit?.asking}
        <button class="suggested" type="submit" form="ai-edit" disabled={!instruction.trim()}>Edit</button>
      {:else if aiEdit?.error}
        <button onclick={() => aiEdit && tryAgain(aiEdit)}>Try Again</button>
      {:else}
        {#if aiEdit?.text}<button onclick={() => aiEdit && tryAgain(aiEdit)}>Try Again</button>{/if}
        <button bind:this={applyButton} class="suggested" disabled={!aiEdit?.text} onclick={applyAiEdit}>Apply</button>
      {/if}
    {/snippet}
  </Dialog>
{/if}

{#if summaryOpen}
  <Dialog title="Summary" wide onclose={() => (summaryOpen = false)}>
    <div class="summary-state" aria-live="polite">
      {#if summarizing}
        <span class="dim"><span class="spinner"></span> Summarizing…</span>
      {:else if summaryError}
        <span class="error-text">{summaryError}</span>
      {:else if summary?.stale}
        <span class="stale">The note has changed since this summary.</span>
      {:else if summary}
        <span class="dim">Made {madeAt(summary.at)}</span>
      {/if}
    </div>
    {#if summary && !summarizing}
      <div class="page tidy-page summary-page" bind:this={summaryParent}></div>
    {/if}
    {#snippet actions()}
      <button onclick={() => (summaryOpen = false)}>Close</button>
      {#if summaryError}
        <button onclick={() => void makeSummary()}>Try Again</button>
      {:else if summary?.stale}
        <button class="suggested" disabled={summarizing} onclick={() => void makeSummary()}>Update</button>
      {:else}
        <button disabled={summarizing || !summary} onclick={() => void makeSummary()}>Regenerate</button>
      {/if}
    {/snippet}
  </Dialog>
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

  .done {
    min-height: 34px !important;
    padding: 0 16px;
    border-radius: var(--radius-pill);
  }

  .share {
    gap: 6px;
    padding: 0 12px;
  }

  .peers {
    display: flex;
    padding: 0 6px;
  }

  .peers .avatar {
    box-shadow: 0 0 0 2px var(--view-bg);
  }

  .peers .avatar + .avatar {
    margin-left: -6px;
  }

  .badge {
    margin: 0 6px;
    padding: 3px 10px;
    border-radius: var(--radius-pill);
    background: var(--hover);
    color: var(--dim-fg);
    font-size: var(--text-xs);
    font-weight: 700;
    white-space: nowrap;
  }

  /* Wide: the formatting toolbar is a soft rounded strip under the headerbar. */
  .format {
    container: format / inline-size;
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
    top: calc(64px + var(--safe-top));
    left: 50%;
    z-index: 20;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 16px;
    translate: -50% 0;
    border-radius: var(--radius-pill);
    background: var(--popover-bg);
    box-shadow: var(--shadow-md);
    font-weight: 600;
    font-size: var(--text-sm);
  }

  .adding .spinner {
    width: 14px;
    height: 14px;
    color: var(--accent);
  }

  .page :global(.cm-title-hint) {
    color: var(--dim-fg);
    pointer-events: none;
  }

  /* Above a photo or memo on the first line: a tappable row that makes a title line. */
  .page :global(.cm-title-hint.add) {
    display: block;
    width: fit-content;
    margin-bottom: 8px;
    padding: 2px 12px;
    border-radius: var(--radius-pill);
    background: var(--hover);
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
    pointer-events: auto;
  }

  /* Photos and voice memos embedded in the text. */
  /* Where a recording's text is going: a red dot and "Listening…" after the last word. */
  .page :global(.cm-listening) {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: 6px;
    padding: 1px 9px;
    border-radius: var(--radius-pill);
    background: var(--button-bg);
    color: var(--dim-fg);
    font-size: var(--text-xs);
    font-weight: 600;
    vertical-align: 1px;
    user-select: none;
  }

  .page :global(.cm-listening::before) {
    content: "";
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--destructive-bg);
    animation: listening 1.2s ease-in-out infinite;
  }

  @keyframes listening {
    50% {
      opacity: 0.25;
    }
  }

  .page :global(.cm-embed-line) {
    padding: 6px 0 !important;
    font-size: var(--text-md) !important;
    font-weight: 400 !important;
  }

  .page :global(.cm-attachment) {
    display: block;
    width: fit-content;
    max-width: 100%;
    border-radius: var(--radius-md);
    cursor: default;
    animation: rise 220ms var(--ease-out) both;
  }

  .page :global(.cm-attachment.is-image),
  .page :global(.cm-attachment.is-audio) {
    position: relative;
  }

  /* A memo player is as wide as the line allows, up to a card's width. */
  .page :global(.cm-attachment.is-audio) {
    width: min(100%, 380px);
  }

  /* A photo's or memo's tools: a pill in its corner while you point at it, always on touch screens. */
  .page :global(.cm-att-tools) {
    position: absolute;
    top: 8px;
    right: 8px;
    z-index: 1;
    display: flex;
    gap: 2px;
    padding: 3px;
    border-radius: var(--radius-pill);
    background: var(--popover-bg);
    box-shadow: var(--shadow-md);
    opacity: 0;
    transition: opacity var(--fast) ease;
  }

  /* On a memo it sits on the card's top edge, clear of the player's controls. */
  .page :global(.is-audio .cm-att-tools) {
    top: -14px;
  }

  .page :global(.cm-attachment:hover .cm-att-tools),
  .page :global(.cm-att-tools:has(:focus-visible)) {
    opacity: 1;
  }

  @media (hover: none) {
    .page :global(.cm-att-tools) {
      opacity: 1;
    }
  }

  .page :global(.cm-att-tools > *) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-width: 28px;
    min-height: 28px;
    padding: 0 8px;
    border-radius: var(--radius-pill);
    background: transparent;
    color: var(--fg);
    font-size: var(--text-xs);
    font-weight: 700;
    white-space: nowrap;
    text-decoration: none;
  }

  .page :global(.cm-att-tools > *:hover) {
    background: var(--hover);
  }

  .page :global(.cm-att-tools .tool-delete) {
    color: var(--destructive);
  }

  .page :global(.cm-att-tools .tool-read),
  .page :global(.cm-att-tools .tool-transcribe),
  .page :global(.cm-att-tools .tool-delete) {
    display: none;
  }

  .page.can-read :global(.cm-att-tools .tool-read),
  .page.can-transcribe :global(.cm-att-tools .tool-transcribe),
  .page.can-edit :global(.cm-att-tools .tool-delete) {
    display: inline-flex;
  }

  .page :global(.reading .cm-att-tools),
  .page :global(.transcribing .cm-att-tools) {
    display: none;
  }

  /* Playback speed, inside the player next to the time. */
  .page :global(.cm-audio-speed) {
    min-width: 34px;
    min-height: 24px;
    padding: 0 6px;
    border-radius: var(--radius-pill);
    background: var(--button-bg);
    color: var(--dim-fg);
    font-size: var(--text-xs);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  /* A photo whose text is being read: a line under it, where the text will go. */
  .page :global(.cm-reading) {
    display: none;
  }

  .page :global(.reading .cm-reading) {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
    white-space: nowrap;
    color: var(--dim-fg);
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .page :global(.reading .cm-reading::before) {
    content: "";
    width: 10px;
    height: 10px;
    border: 2px solid var(--accent-bg);
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
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
    font-size: var(--text-sm);
  }

  .page :global(.cm-attachment img) {
    display: block;
    max-width: 100%;
    max-height: 70vh;
    border-radius: var(--radius-md);
    box-shadow: 0 0 0 1px var(--border);
    cursor: zoom-in;
  }

  .page :global(.cm-audio) {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 10px 14px 10px 10px;
    border-radius: var(--radius-md);
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

  .page :global(.cm-audio-body) {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .page :global(.cm-audio-title) {
    font-weight: 700;
    font-size: var(--text-sm);
    line-height: 1.1;
  }

  .page :global(.cm-audio-bar) {
    height: 5px;
    border-radius: var(--radius-sm);
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
    font-size: var(--text-sm);
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
    font-size: var(--text-sm);
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

  /*
    The editor spans the whole pane and the text is centered by padding inside it, so a click or
    drag anywhere beside the text is still in the editor: it lands on the nearest line and selects.
  */
  .column {
    --measure: 712px;
    --gutter: 24px;
    width: 100%;
    transition: opacity 220ms ease;
  }

  /* The room under the text is inside the editor, so menus at the cursor count as on screen. */
  .page {
    --room: 40vh;
    padding-top: 28px;
  }

  .page.with-chip {
    padding-top: 10px;
  }

  /* Which notebook this note lives in (tap to open it), lined up with the text. */
  .note-head {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 20px max(var(--gutter), calc((100% - var(--measure)) / 2)) 0 max(18px, calc((100% - var(--measure)) / 2 - 6px));
  }

  .notebook-chip {
    min-width: 0;
    max-width: 100%;
  }

  /* Summarizing…, when it was made, or that the note has moved on since. */
  .summary-state {
    display: flex;
    align-items: center;
    justify-content: center;
    margin: -8px 0 12px;
    font-size: var(--text-sm);
    text-align: center;
  }

  .summary-state > span {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .summary-state .spinner {
    width: 14px;
    height: 14px;
    color: var(--accent);
  }

  .summary-state .stale {
    color: var(--fg);
    font-weight: 600;
  }

  .summary-state .error-text {
    color: var(--destructive);
  }

  .tidy-state {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    margin: 8px 0 16px;
    text-align: center;
  }

  .tidy-hint {
    margin: -8px 0 12px;
    font-size: var(--text-sm);
    text-align: center;
  }

  /* The tidied note, drawn like the note itself on a card of its own. */
  .page.tidy-page {
    --room: 0px;
    --gutter: 0px;
    --measure: 100%;
    padding: 14px 16px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--view-bg);
  }

  .tidy-page :global(.cm-editor) {
    font-size: var(--text-md);
  }

  /* Edit with AI: what to change, and a few ideas to tap. */
  .ai-ask {
    display: flex;
    flex-direction: column;
    gap: 14px;
    margin-bottom: 4px;
  }

  .ai-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .ai-chip {
    min-height: 32px;
    padding: 0 14px;
    border-radius: var(--radius-pill);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  /* The AI Prompt block: a field in the note, under the line it was opened on. */
  .page :global(.cm-ai-prompt) {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 6px 0;
    padding: 0 6px 0 12px;
    border-radius: var(--radius-md);
    background: var(--card-bg);
    box-shadow: var(--shadow-sm), 0 0 0 1px var(--border);
    font-size: var(--text-md);
    white-space: normal;
    cursor: default;
    animation: rise 220ms var(--ease-out) both;
  }

  .page :global(.cm-line.cm-ai-absorbed) {
    display: none;
  }

  .page :global(.cm-ai-prompt:focus-within) {
    box-shadow: var(--shadow-sm), 0 0 0 2px color-mix(in srgb, var(--accent-bg) 60%, transparent);
  }

  .page :global(.cm-ai-prompt > svg) {
    flex: none;
    color: var(--accent);
  }

  /* No width of its own, so a long prompt never widens the note. */
  .page :global(.cm-ai-prompt input) {
    flex: 1;
    width: 0;
    min-height: 44px;
    padding: 0;
    background: transparent;
    box-shadow: none;
  }

  .page :global(.cm-ai-busy) {
    display: none;
    align-items: center;
    gap: 8px;
    padding-right: 8px;
    color: var(--dim-fg);
    font-size: var(--text-sm);
    font-weight: 600;
    white-space: nowrap;
  }

  .page :global(.cm-ai-prompt.busy .cm-ai-busy) {
    display: flex;
  }

  .page :global(.cm-ai-prompt.busy input) {
    color: var(--dim-fg);
  }

  .page :global(.cm-ai-busy .spinner) {
    width: 14px;
    height: 14px;
    color: var(--accent);
  }

  .hidden {
    opacity: 0;
    pointer-events: none;
  }

  .page :global(.cm-editor) {
    background: transparent;
    color: var(--fg);
    font-size: var(--text-lg);
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
    padding: 0 max(var(--gutter), calc((100% - var(--measure)) / 2)) var(--room);
  }

  .page :global(.cm-line) {
    padding: 0;
  }

  /* The blank title line's hint, drawn off to the side of the text so the caret keeps its place. */
  .page :global(.cm-title-empty) {
    position: relative;
  }

  .page :global(.cm-title-empty)::before {
    content: "Title";
    position: absolute;
    inset: 0 auto auto 0;
    color: var(--dim-fg);
    pointer-events: none;
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

  /* A [[link]] to another note reads as its title and a web link as its text, both in the link color; a missing note reads as dimmed. */
  .page :global(.cm-wikilink),
  .page :global(.cm-link) {
    color: var(--accent);
    cursor: pointer;
  }

  .page :global(.cm-wikilink-missing) {
    color: var(--dim-fg);
    text-decoration: underline dotted;
    text-underline-offset: 2px;
    cursor: default;
  }

  /* Bullets and checkboxes (with the space after them) are --marker wide, so wrapped lines hang there. */
  .page :global(.cm-item) {
    --marker: 1.4em;
    padding-left: var(--marker);
    text-indent: calc(-1 * var(--marker));
  }

  .page :global(.cm-item:has(.cm-checkbox)) {
    --marker: 1.75em;
  }

  /* A marker plus its real space (1ch of a monospace face) is exactly --marker wide. */
  .page :global(.cm-marker-space),
  .page :global(.cm-bullet),
  .page :global(.cm-checkbox) {
    font-family: "Adwaita Mono", "Source Code Pro", monospace;
  }

  .page :global(.cm-bullet) {
    display: inline-block;
    width: calc(1.4em - 1ch);
    text-indent: 0;
    color: var(--dim-fg);
  }

  .page :global(.cm-checkbox) {
    display: inline-block;
    width: 1.15em;
    height: 1.15em;
    margin-right: calc(0.6em - 1ch);
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

  /* Who wrote each line: a bar in their color in the margin, with room around it to point at or tap. */
  .page :global(.cm-blamed) {
    position: relative;
  }

  .page :global(.cm-blame) {
    position: absolute;
    top: 0.2em;
    bottom: 0.2em;
    left: -12px;
    width: 3px;
    border-radius: 2px;
    background: var(--user-color);
    text-indent: 0;
    cursor: pointer;
  }

  .page :global(.cm-blame::before) {
    content: "";
    position: absolute;
    inset: -0.2em -6px;
  }

  /* The grip beside a block, in the margin left of its text, and where a dragged block will land. */
  .page :global(.cm-drag-grip) {
    position: absolute;
    z-index: 3;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 28px;
    min-height: 0;
    padding: 0;
    translate: calc(-100% - 4px) -50%;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--dim-fg);
    cursor: grab;
    touch-action: none;
    opacity: 0;
    pointer-events: none;
    transition: opacity var(--fast) ease;
  }

  /* On someone else's line the grip takes their color, like the bar it sits over. */
  .page :global(.cm-drag-grip.authored) {
    background: var(--view-bg);
    color: var(--user-color);
  }

  .page :global(.cm-drag-grip.shown) {
    opacity: 1;
    pointer-events: auto;
  }

  .page :global(.cm-drag-grip:hover),
  .page :global(.cm-drag-active .cm-drag-grip) {
    background: var(--hover);
    color: var(--fg);
  }

  .page :global(.cm-drag-active),
  .page :global(.cm-drag-active .cm-drag-grip) {
    cursor: grabbing;
    user-select: none;
  }

  .page :global(.cm-drag-source) {
    opacity: 0.35;
  }

  .page :global(.cm-drop-marker) {
    position: absolute;
    z-index: 3;
    display: none;
    height: 2px;
    border-radius: 1px;
    background: var(--accent-bg);
    pointer-events: none;
  }

  .page :global(.cm-drop-marker.shown) {
    display: block;
  }

  /* The "/" menu: a popover of styles and things to add, under the cursor. */
  .page :global(.cm-tooltip.cm-slash) {
    overflow: hidden;
    border: none;
    border-radius: var(--radius-md);
    background: var(--popover-bg);
    box-shadow: var(--shadow-lg);
  }

  .page :global(.cm-slash > ul) {
    min-width: 220px;
    max-width: min(360px, 85vw);
    max-height: min(320px, 40vh) !important;
    padding: 6px !important;
    font-family: inherit !important;
  }

  .page :global(.cm-slash completion-section) {
    display: block;
    padding: 8px 12px 4px;
    border: none !important;
    color: var(--dim-fg);
    font-size: var(--text-xs);
    font-weight: 700;
  }

  .page :global(.cm-slash li) {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 38px;
    padding: 0 12px !important;
    border-radius: var(--radius-sm);
    color: var(--fg);
    font-size: var(--text-md);
  }

  .page :global(.cm-slash li svg) {
    flex: none;
    color: var(--dim-fg);
  }

  .page :global(.cm-slash li[aria-selected]) {
    background: var(--active);
    color: var(--fg);
  }

  .page :global(.cm-slash .cm-emoji-glyph) {
    flex: none;
    width: 16px;
    font-size: 1.15em;
    line-height: 1;
    text-align: center;
  }

  /* An emoji's name next to it, then what it is in plain words. */
  .page :global(.cm-slash .cm-completionLabel) {
    flex: none;
  }

  .page :global(.cm-slash .cm-completionDetail) {
    min-width: 0;
    margin-left: auto;
    padding-left: 12px;
    color: var(--dim-fg);
    font-size: var(--text-sm);
    font-style: normal;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .page :global(.cm-slash .cm-completionMatchedText) {
    text-decoration: none;
    font-weight: 700;
  }

  /* Fenced code: a tinted block in a monospace face, its language picker on the first row. */
  .page :global(.cm-line.cm-code) {
    padding: 0 14px;
    background: var(--code-bg);
    font-family: "Adwaita Mono", "Source Code Pro", monospace;
    font-size: 0.85em;
    line-height: 1.6;
  }

  /*
    Code keeps its lines whole: a long one scrolls sideways inside the block, never wrapping mid-token
    or widening the page. A spacer at the block's widest line (--code-cols) lets every line scroll as far.
  */
  .page :global(.cm-line.cm-code-body) {
    position: relative;
    /* Its text never widens the note; it scrolls instead. */
    contain: inline-size;
    white-space: pre;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
  }

  .page :global(.cm-line.cm-code-body::after) {
    content: "";
    position: absolute;
    top: 0;
    left: calc(var(--code-cols) * 1ch);
    width: 28px;
    height: 1px;
  }

  /* Who wrote a line of code: its bar goes inside the block, where the line's scrolling can't clip it. */
  .page :global(.cm-code-body .cm-blame) {
    left: 4px;
  }

  .page :global(.cm-code-first) {
    padding-top: 6px !important;
    border-radius: var(--radius) var(--radius) 0 0;
  }

  .page :global(.cm-code-last) {
    padding-bottom: 4px !important;
    border-radius: 0 0 var(--radius) var(--radius);
  }

  /* The hidden closing fence leaves just a little room under the code. */
  .page :global(.cm-line.cm-code-closed) {
    height: 12px;
    padding: 0 !important;
    overflow: hidden;
  }

  /* The language, a quiet dropdown lined up with the code. */
  .page :global(.cm-code-lang) {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-left: -6px;
    padding: 0 6px;
    border-radius: var(--radius-sm);
    color: var(--dim-fg);
  }

  .page :global(.cm-code-lang:has(select:enabled):hover) {
    background: var(--hover);
  }

  .page :global(.cm-code-lang select) {
    appearance: none;
    field-sizing: content;
    min-height: 24px;
    padding: 0;
    border: none;
    background: transparent;
    color: inherit;
    font-family: system-ui, sans-serif;
    font-size: var(--text-xs);
    font-weight: 700;
    cursor: pointer;
  }

  .page :global(.cm-code-lang select:disabled) {
    opacity: 1;
    cursor: default;
  }

  .page :global(.cm-code .tok-keyword),
  .page :global(.cm-code .tok-operator.tok-keyword) {
    color: var(--code-keyword);
  }

  .page :global(.cm-code .tok-string),
  .page :global(.cm-code .tok-string2) {
    color: var(--code-string);
  }

  .page :global(.cm-code .tok-number),
  .page :global(.cm-code .tok-bool),
  .page :global(.cm-code .tok-atom) {
    color: var(--code-number);
  }

  .page :global(.cm-code .tok-comment),
  .page :global(.cm-code .tok-meta) {
    color: var(--dim-fg);
    font-style: italic;
  }

  .page :global(.cm-code .tok-typeName),
  .page :global(.cm-code .tok-className),
  .page :global(.cm-code .tok-namespace) {
    color: var(--code-type);
  }

  .page :global(.cm-code .tok-definition),
  .page :global(.cm-code .tok-macroName),
  .page :global(.cm-code .tok-propertyName),
  .page :global(.cm-code .tok-tagName) {
    color: var(--code-name);
  }

  .page :global(.cm-task-done) {
    color: var(--dim-fg);
    text-decoration: line-through;
  }

  /* Tables and kanban boards (lib/tables.ts, lib/kanban.ts): drawn from their Markdown, scrolling sideways inside when wide. */
  .page :global(.cm-table-widget),
  .page :global(.cm-kanban-widget) {
    /* Its content never widens the note; it scrolls instead. */
    contain: inline-size;
    padding: 6px 0;
    font-size: var(--text-md);
    line-height: 1.4;
    white-space: normal;
    cursor: default;
  }

  .page :global(.cm-table-scroll) {
    width: fit-content;
    max-width: 100%;
    overflow-x: auto;
    border-radius: var(--radius-md);
    box-shadow: 0 0 0 1px var(--border);
  }

  .page :global(.cm-table) {
    border-collapse: collapse;
  }

  .page :global(.cm-table th),
  .page :global(.cm-table td) {
    padding: 0;
    border: 1px solid var(--border);
    vertical-align: top;
    text-align: left;
  }

  .page :global(.cm-table tr:first-child > *) {
    border-top: none;
  }

  .page :global(.cm-table tr:last-child > *) {
    border-bottom: none;
  }

  .page :global(.cm-table tr > :first-child) {
    border-left: none;
  }

  .page :global(.cm-table tr > :last-child) {
    border-right: none;
  }

  .page :global(.cm-table th) {
    background: var(--hover);
    font-weight: 700;
  }

  .page :global(.cm-table-cell) {
    min-width: 6em;
    max-width: 18em;
    min-height: calc(1.4em + 16px);
    padding: 8px 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    outline: none;
  }

  .page :global(.cm-table-widget.editable .cm-table-cell) {
    cursor: text;
  }

  .page :global(.cm-table-cell:focus) {
    box-shadow: inset 0 0 0 2px var(--accent-bg);
  }

  /* Row and column tools, under the table while a cell is being edited. */
  .page :global(.cm-table-tools) {
    display: none;
    gap: 4px;
    margin-top: 8px;
    overflow-x: auto;
    scrollbar-width: none;
  }

  .page :global(.cm-table-widget:focus-within .cm-table-tools) {
    display: flex;
    animation: rise 160ms var(--ease-out) both;
  }

  .page :global(.cm-table-tools button) {
    flex: none;
    min-height: 32px;
    padding: 0 10px;
    border-radius: var(--radius-pill);
    font-size: var(--text-sm);
  }

  .page :global(.cm-table-tools button[aria-label^="Delete"]) {
    color: var(--destructive);
  }

  .page :global(.cm-table-tools button:disabled) {
    opacity: 0.45;
    cursor: default;
  }

  .page :global(.cm-kanban) {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding-bottom: 6px;
    overflow-x: auto;
    overscroll-behavior-x: contain;
    scroll-snap-type: x proximity;
  }

  .page :global(.cm-kanban-col) {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 240px;
    padding: 6px;
    border-radius: var(--radius-lg);
    background: var(--hover);
    scroll-snap-align: start;
  }

  .page :global(.cm-kanban-head) {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 32px;
    padding: 0 6px;
  }

  .page :global(.cm-kanban-title) {
    flex: 1;
    min-width: 0;
    margin: 0 -4px;
    padding: 2px 4px;
    border-radius: var(--radius-sm);
    font-size: var(--text-sm);
    font-weight: 700;
    overflow-wrap: anywhere;
    outline: none;
  }

  .page :global(.cm-kanban-title:focus) {
    background: var(--view-bg);
    box-shadow: 0 0 0 2px var(--accent-bg);
  }

  .page :global(.cm-kanban-count) {
    color: var(--dim-fg);
    font-size: var(--text-xs);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .page :global(.cm-kanban-cards) {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .page :global(.cm-kanban-card) {
    position: relative;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    border-radius: var(--radius);
    background: var(--card-bg);
    box-shadow: var(--shadow-sm), 0 0 0 1px var(--border);
    user-select: none;
    -webkit-user-select: none;
    -webkit-touch-callout: none;
  }

  .page :global(.cm-kanban-widget.editable .cm-kanban-card) {
    cursor: grab;
  }

  .page :global(.cm-kanban-card.editing) {
    padding-right: 40px;
    box-shadow: 0 0 0 2px var(--accent-bg);
    cursor: text !important;
    user-select: text;
    -webkit-user-select: text;
  }

  .page :global(.cm-kanban-text) {
    flex: 1;
    min-width: 0;
    min-height: 1.4em;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    outline: none;
  }

  .page :global(.cm-kanban-card .cm-checkbox) {
    flex: none;
    margin: 0.12em 0 0;
  }

  .page :global(.cm-kanban-card.done .cm-kanban-text) {
    color: var(--dim-fg);
    text-decoration: line-through;
  }

  .page :global(.cm-kanban-del) {
    position: absolute;
    top: 50%;
    right: 6px;
    display: none;
    width: 28px;
    min-height: 28px;
    padding: 0;
    translate: 0 -50%;
    border-radius: 50%;
    background: var(--popover-bg);
    color: var(--dim-fg);
    box-shadow: var(--shadow-sm);
  }

  .page :global(.cm-kanban-card:hover .cm-kanban-del),
  .page :global(.cm-kanban-card.editing .cm-kanban-del) {
    display: inline-flex;
  }

  .page :global(.cm-kanban-add) {
    justify-content: flex-start;
    min-height: 34px;
    padding: 0 8px;
    background: transparent;
    color: var(--dim-fg);
    font-size: var(--text-sm);
  }

  .page :global(.cm-kanban-add:hover) {
    background: var(--hover);
    color: var(--fg);
  }

  .page :global(.cm-kanban-end) {
    flex: none;
    display: flex;
    gap: 6px;
  }

  .page :global(.cm-kanban-add-col),
  .page :global(.cm-kanban-md) {
    min-height: 44px;
    border-radius: var(--radius-lg);
    background: var(--hover);
    color: var(--dim-fg);
    font-size: var(--text-sm);
  }

  .page :global(.cm-kanban-md) {
    width: 44px;
    padding: 0;
    background: transparent;
  }

  /* Dragging a card: it follows the pointer, and a tinted gap shows where it will land. */
  .page :global(.cm-kanban-card.dragging) {
    display: none;
  }

  .page :global(.cm-kanban-spot) {
    border-radius: var(--radius);
    background: var(--accent-soft);
  }

  .page :global(.cm-kanban-ghost) {
    position: fixed;
    z-index: 30;
    margin: 0;
    background: var(--popover-bg);
    box-shadow: var(--shadow-lg);
    pointer-events: none;
  }

  .page :global(.cm-kanban-widget.dragging),
  .page :global(.cm-kanban-widget.dragging *) {
    cursor: grabbing !important;
  }

  @media (max-width: 700px) {
    .page :global(.cm-kanban-col) {
      width: min(76vw, 260px);
    }
  }

  /* Phone: the formatting bar floats on top of the keyboard while typing. */
  @media (max-width: 700px) {
    /* The title line says what the note is; once it scrolls away, the headerbar does. */
    header .title {
      align-items: center;
      text-align: center;
      opacity: 0;
      transform: translateY(4px);
      transition:
        opacity var(--fast) ease,
        transform var(--fast) ease;
    }

    header .title.shown {
      opacity: 1;
      transform: none;
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

    .column {
      --gutter: 18px;
    }

    .page :global(.cm-drag-grip) {
      width: 18px;
      translate: calc(-100% - 1px) -50%;
    }

    .page {
      --room: 50vh;
      padding-top: 20px;
    }

    .page.with-chip {
      padding-top: 8px;
    }

    .note-head {
      padding: 14px 12px 0 calc(var(--gutter-left) - 6px);
    }

    /* A wider left margin keeps the drag grip clear of the screen edge, where swipes go back. */
    .column {
      --gutter-left: 40px;
    }

    .page:not(.tidy-page) :global(.cm-content) {
      padding-left: var(--gutter-left);
    }
  }

  .new {
    margin-left: 2px;
    min-width: 36px;
    border-radius: 50%;
  }
</style>
