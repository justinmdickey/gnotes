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
  const lists: [Block, "checklist" | "bullets" | "numbers" | "quote", string][] = [
    ["check", "checklist", "Checklist"],
    ["bullet", "bullets", "Bulleted list"],
    ["number", "numbers", "Numbered list"],
    ["quote", "quote", "Quote"],
  ];

  // Keep focus in the editor so the phone keyboard stays up.
  const keep = (e: Event) => e.preventDefault();
  /** Phone only: the text styles sit behind an "Aa" button, like Apple Notes. */
  let stylesOpen = $state(false);
</script>

<div class="bar" class:styles-open={stylesOpen} role="toolbar" aria-label="Formatting" tabindex="-1" onmousedown={keep} onpointerdown={keep}>
  <button class="flat icon aa" class:active={stylesOpen} aria-label="Text styles" aria-expanded={stylesOpen} onclick={() => (stylesOpen = !stylesOpen)}>
    <Icon name="textformat" />
  </button>
  <div class="group styles">
    {#each styles as [b, label] (b)}
      <button class="flat chip style-{b}" class:active={block === b} onclick={() => setBlock(view, b)}>{label}</button>
    {/each}
  </div>
  <div class="sep styles-sep"></div>
  <div class="group marks">
    {#each marks as [m, glyph, label] (m)}
      <button class="flat icon mark-{m}" class:active={inline.has(m)} title={label} aria-label={label} aria-pressed={inline.has(m)} onclick={() => toggleInline(view, m)}>
        {glyph}
      </button>
    {/each}
    <button class="flat icon" class:active={inline.has("code")} title="Code" aria-label="Code" aria-pressed={inline.has("code")} onclick={() => toggleInline(view, "code")}>
      <Icon name="code" />
    </button>
  </div>
  <div class="sep marks-sep"></div>
  <div class="group lists">
    {#each lists as [b, icon, label] (b)}
      <button class="flat icon" class:active={block === b} title={label} aria-label={label} aria-pressed={block === b} onclick={() => setBlock(view, b)}>
        <Icon name={icon} />
      </button>
    {/each}
  </div>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 6px;
    overflow-x: auto;
    scrollbar-width: none;
    background: var(--headerbar-bg);
  }

  .group {
    display: flex;
    gap: 2px;
    flex: none;
  }

  .sep {
    flex: none;
    width: 1px;
    height: 20px;
    margin: 0 4px;
    background: var(--border);
  }

  .chip {
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

  /* Phone: Aa, lists, then marks; styles replace the row while open. */
  @media (max-width: 700px) {
    .aa {
      display: inline-flex;
      order: 0;
    }

    .styles,
    .styles-sep {
      display: none;
    }

    .lists {
      order: 1;
    }

    .marks-sep {
      order: 2;
    }

    .marks {
      order: 3;
    }

    .styles-open .styles {
      display: flex;
      order: 1;
    }

    .styles-open .lists,
    .styles-open .marks,
    .styles-open .marks-sep {
      display: none;
    }
  }

  .active {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
</style>
