<script lang="ts">
  import { onMount } from "svelte";
  import { EditorState } from "@codemirror/state";
  import { EditorView, keymap } from "@codemirror/view";
  import { defaultKeymap } from "@codemirror/commands";
  import { markdown } from "@codemirror/lang-markdown";
  import { EphemeralStore, LoroDoc, UndoManager } from "loro-crdt";
  import { LoroExtensions } from "loro-codemirror";

  // Local-only for now. The websocket sync from docs/DESIGN.md attaches to this doc and ephemeral store.
  const doc = new LoroDoc();
  const ephemeral = new EphemeralStore();
  const undoManager = new UndoManager(doc, {});
  doc.getText("body").insert(0, "# Welcome to Gnotes\n\nStart typing.\n");
  doc.commit();

  let parent: HTMLDivElement;

  onMount(() => {
    const view = new EditorView({
      parent,
      state: EditorState.create({
        extensions: [
          markdown(),
          EditorView.lineWrapping,
          keymap.of(defaultKeymap),
          LoroExtensions(
            doc,
            { ephemeral, user: { name: "Me", colorClassName: "user-blue" } },
            undoManager,
            (d) => d.getText("body"),
          ),
        ],
      }),
    });
    return () => view.destroy();
  });
</script>

<div class="editor" bind:this={parent}></div>

<style>
  .editor {
    width: 100%;
    max-width: 760px;
    padding: 24px 16px;
  }

  .editor :global(.cm-editor) {
    height: 100%;
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
</style>
