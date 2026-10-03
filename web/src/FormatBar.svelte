<script lang="ts">
  import type { EditorView } from "@codemirror/view";
  import { setBlock, toggleInline, type Block, type Inline } from "./lib/format";
  import Icon from "./lib/Icon.svelte";
  import { insertTable } from "./lib/tables";

  let {
    view,
    block,
    inline,
    history,
    onundo,
    onredo,
    onphoto,
    onrecord,
    ondone,
  }: {
    view: EditorView;
    block: Block;
    inline: Set<Inline>;
    /** Whether there's anything to undo or redo. */
    history: { undo: boolean; redo: boolean };
    onundo: () => void;
    onredo: () => void;
    onphoto: () => void;
    onrecord: () => void;
    ondone: () => void;
  } = $props();

  const styles: [Block, string][] = [
    ["title", "Title"],
    ["heading", "Heading"],
    ["subheading", "Subheading"],
    ["body", "Body"],
  ];
  const marks: [Inline, "bold" | "italic" | "strikethrough", string][] = [
    ["bold", "bold", "Bold"],
    ["italic", "italic", "Italic"],
    ["strike", "strikethrough", "Strikethrough"],
  ];
  const lists: [Block, "checklist" | "bullets" | "numbers", string][] = [
    ["check", "checklist", "Checklist"],
    ["bullet", "bullets", "Bulleted list"],
    ["number", "numbers", "Numbered list"],
  ];

  // Keep focus in the editor so the phone keyboard stays up.
  const keep = (e: Event) => e.preventDefault();
  /** Phone only: text styles, quote and code sit behind an "Aa" button, like Apple Notes. */
  let stylesOpen = $state(false);
</script>

<!--
  Class hints for the compact layout: .main shows in the everyday row, .extra only
  while "Aa" is open. With room for everything, all show except the Aa toggle.
  Bold and italic sit behind "Aa" there, as in Apple Notes, to leave room for undo and redo.
-->
<div class="bar" class:styles-open={stylesOpen} role="toolbar" aria-label="Formatting" tabindex="-1" onmousedown={keep} onpointerdown={keep}>
  <button class="flat icon aa main extra" class:active={stylesOpen} aria-label="Text styles" aria-expanded={stylesOpen} onclick={() => (stylesOpen = !stylesOpen)}>
    <Icon name="textformat" />
  </button>

  <button class="flat icon main history" title="Undo" aria-label="Undo" disabled={!history.undo} onclick={onundo}><Icon name="undo" /></button>
  <button class="flat icon main history" title="Redo" aria-label="Redo" disabled={!history.redo} onclick={onredo}><Icon name="redo" /></button>
  <span class="sep"></span>

  {#each styles as [b, label] (b)}
    <button class="flat chip extra style-{b}" class:active={block === b} onclick={() => setBlock(view, b)}>{label}</button>
  {/each}
  <span class="sep"></span>

  {#each marks as [m, icon, label] (m)}
    <button class="flat icon extra mark-{m}" class:active={inline.has(m)} title={label} aria-label={label} aria-pressed={inline.has(m)} onclick={() => toggleInline(view, m)}>
      <Icon name={icon} />
    </button>
  {/each}
  <button class="flat icon extra" class:active={inline.has("code")} title="Code" aria-label="Code" aria-pressed={inline.has("code")} onclick={() => toggleInline(view, "code")}>
    <Icon name="code" />
  </button>
  <span class="sep"></span>

  {#each lists as [b, icon, label] (b)}
    <button class="flat icon list" class:main={b !== "number"} class:extra={b === "number"} class:active={block === b} title={label} aria-label={label} aria-pressed={block === b} onclick={() => setBlock(view, b)}>
      <Icon name={icon} />
    </button>
  {/each}
  <button class="flat icon extra" class:active={block === "quote"} title="Quote" aria-label="Quote" aria-pressed={block === "quote"} onclick={() => setBlock(view, "quote")}>
    <Icon name="quote" />
  </button>
  <button class="flat icon extra" title="Table" aria-label="Insert table" onclick={() => insertTable(view)}><Icon name="table" /></button>
  <span class="sep"></span>

  <button class="flat icon main attach" title="Add photo" aria-label="Add photo" onclick={onphoto}><Icon name="camera" /></button>
  <button class="flat icon main attach" title="Record voice memo" aria-label="Record voice memo" onclick={onrecord}><Icon name="mic" /></button>
  <!-- Phone: finish typing from down here instead of reaching for Done at the top. -->
  <button class="flat icon main extra done" title="Hide keyboard" aria-label="Hide keyboard" onclick={ondone}><Icon name="keyboardhide" /></button>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 2px;
    max-width: 100%;
    padding: 4px;
    overflow-x: auto;
    scrollbar-width: none;
    border-radius: var(--radius-md);
    background: var(--hover);
  }

  .bar button {
    min-height: 32px;
  }

  .bar button.icon {
    min-width: 32px;
  }

  .bar button:active:not(:disabled) {
    transform: scale(0.92);
  }

  .sep {
    flex: none;
    width: 1px;
    height: 20px;
    margin: 0 4px;
    background: var(--border);
  }

  .chip {
    flex: none;
    padding: 0 8px;
    font-weight: 400;
  }

  .style-title { font-weight: 800; }
  .style-heading { font-weight: 700; }
  .style-subheading { font-weight: 600; }


  .aa,
  .done {
    display: none;
  }

  .bar .active {
    background: var(--view-bg);
    color: var(--accent);
    box-shadow: 0 1px 2px rgb(0 0 6 / 15%);
  }

  /*
    Not enough room for everything (phones, or a narrow editor): one row of everyday
    buttons, Aa · checklist · bullets · B · I · photo · memo, and "Aa" swaps in the
    text styles, numbers, strikethrough, code and quote.
  */
  @container format (max-width: 720px) {
    .bar {
      justify-content: space-between;
      gap: 0;
      overflow-x: hidden;
    }

    .bar > * {
      display: none;
      animation: rise 160ms var(--ease-out) both;
    }

    /* The styles row can be wider than a small phone; let it scroll instead of clipping. */
    .bar.styles-open {
      justify-content: flex-start;
      gap: 2px;
      overflow-x: auto;
    }

    .bar:not(.styles-open) > .main,
    .bar.styles-open > .extra {
      display: inline-flex;
    }

    /* Lists come right after Aa, then undo and redo, as in Apple Notes. */
    .aa { order: 0; }
    .list { order: 1; }
    .history { order: 2; }
    .attach { order: 3; color: var(--accent); }
    .done { order: 4; }
  }

  /* Only phones have an on-screen keyboard to hide. */
  @media (min-width: 701px) {
    .bar > .done {
      display: none !important;
    }
  }

  /* Phone: a full-width bar on the keyboard with big, evenly spaced targets. */
  @media (max-width: 700px) {
    .done {
      color: var(--dim-fg);
    }

    .bar {
      padding: 2px 4px;
      border-radius: 0;
      background: var(--headerbar-bg);
    }

    .bar .active {
      background: var(--accent-soft);
      box-shadow: none;
    }

    .bar button,
    .bar button.icon {
      min-width: 44px;
      min-height: 44px;
      font-size: var(--text-lg);
    }

    .bar button :global(svg) {
      width: var(--icon-touch);
      height: var(--icon-touch);
    }

    .chip {
      padding: 0 8px;
      font-size: var(--text-sm) !important;
    }
  }
</style>
