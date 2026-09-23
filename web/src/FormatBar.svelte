<script lang="ts">
  import type { EditorView } from "@codemirror/view";
  import { setBlock, toggleInline, type Block, type Inline } from "./lib/format";
  import Icon from "./lib/Icon.svelte";

  let { view, block, inline }: { view: EditorView; block: Block; inline: Set<Inline> } = $props();

  const styles: [Block, string][] = [
    ["title", "Title"],
    ["heading", "Heading"],
    ["subheading", "Subheading"],
    ["body", "Body"],
  ];
  const marks: [Inline, string, string][] = [
    ["bold", "B", "Bold"],
    ["italic", "I", "Italic"],
    ["strike", "S", "Strikethrough"],
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
  Class hints for the phone layout: .main shows in the everyday row, .extra only
  while "Aa" is open. Desktop shows everything except the Aa toggle.
-->
<div class="bar" class:styles-open={stylesOpen} role="toolbar" aria-label="Formatting" tabindex="-1" onmousedown={keep} onpointerdown={keep}>
  <button class="flat icon aa main extra" class:active={stylesOpen} aria-label="Text styles" aria-expanded={stylesOpen} onclick={() => (stylesOpen = !stylesOpen)}>
    <Icon name="textformat" />
  </button>

  {#each styles as [b, label] (b)}
    <button class="flat chip extra style-{b}" class:active={block === b} onclick={() => setBlock(view, b)}>{label}</button>
  {/each}
  <span class="sep"></span>

  {#each marks as [m, glyph, label] (m)}
    <button class="flat icon main mark-{m}" class:active={inline.has(m)} title={label} aria-label={label} aria-pressed={inline.has(m)} onclick={() => toggleInline(view, m)}>
      {glyph}
    </button>
  {/each}
  <button class="flat icon extra" class:active={inline.has("code")} title="Code" aria-label="Code" aria-pressed={inline.has("code")} onclick={() => toggleInline(view, "code")}>
    <Icon name="code" />
  </button>
  <span class="sep"></span>

  {#each lists as [b, icon, label] (b)}
    <button class="flat icon main list" class:active={block === b} title={label} aria-label={label} aria-pressed={block === b} onclick={() => setBlock(view, b)}>
      <Icon name={icon} />
    </button>
  {/each}
  <button class="flat icon extra" class:active={block === "quote"} title="Quote" aria-label="Quote" aria-pressed={block === "quote"} onclick={() => setBlock(view, "quote")}>
    <Icon name="quote" />
  </button>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 6px;
    overflow-x: auto;
    scrollbar-width: none;
    background: var(--headerbar-bg);
  }

  .sep {
    flex: none;
    width: 1px;
    height: 20px;
    margin: 0 6px;
    background: var(--border);
  }

  .chip {
    flex: none;
    padding: 0 10px;
    font-weight: 400;
  }

  .style-title { font-weight: 800; }
  .style-heading { font-weight: 700; }
  .style-subheading { font-weight: 600; }

  .mark-bold { font-weight: 800; }
  .mark-italic { font-style: italic; font-family: serif; font-size: 1.05rem; }
  .mark-strike { text-decoration: line-through; font-weight: 400; }

  .aa {
    display: none;
  }

  .active {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }

  /*
    Phone: one row of big, evenly spaced targets. Everyday row is
    Aa · checklist · bullets · numbers · B · I · S; "Aa" swaps in styles, quote and code.
  */
  @media (max-width: 700px) {
    .bar {
      justify-content: space-between;
      gap: 0;
      padding: 2px 4px;
      overflow-x: hidden;
    }

    .bar > * {
      display: none;
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

    .bar button {
      min-width: 44px;
      min-height: 44px;
      font-size: 1.1rem;
    }

    .bar button :global(svg) {
      width: 21px;
      height: 21px;
    }

    .chip {
      padding: 0 8px;
      font-size: 0.95rem !important;
    }

    /* Lists come right after Aa, marks after them, as in Apple Notes. */
    .aa { order: 0; }
    .list { order: 1; }
    .main[class*="mark-"] { order: 2; }
  }
</style>
