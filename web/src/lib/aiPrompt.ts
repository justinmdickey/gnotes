// The AI Prompt block: a prompt field put in the note at the cursor by the "/" menu. It's a widget
// in this editor only, not text in the note, so collaborators never see it. Its place is mapped
// through every change; Enter asks the AI to write there, and what comes back replaces the field as
// ordinary text, so it syncs like typing.
import { type ChangeSpec, StateEffect, StateField, type TransactionSpec } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import { iconSvg } from "./icons";

export interface AiPromptActions {
  /** The Markdown to put at `at` in `text`, written for `prompt`; throws (and says why) on failure. */
  write: (prompt: string, text: string, at: number) => Promise<string>;
  /** Writes the text in, as its own undo step. */
  commit: (view: EditorView, spec: TransactionSpec) => void;
}

interface Prompt {
  /** Which prompt this is, so a new one gets a fresh field. */
  id: number;
  /** A position on the line the field sits under. */
  pos: number;
  busy: boolean;
}

const setPrompt = StateEffect.define<Prompt | null>();
const setBusy = StateEffect.define<boolean>();

let ids = 0;

class PromptWidget extends WidgetType {
  constructor(
    private prompt: Prompt,
    private actions: AiPromptActions,
  ) {
    super();
  }

  eq(other: PromptWidget) {
    return other.prompt.id === this.prompt.id && other.prompt.busy === this.prompt.busy;
  }

  toDOM(view: EditorView) {
    const box = document.createElement("div");
    box.className = "cm-ai-prompt";
    box.append(iconSvg("sparkle"));
    const input = document.createElement("input");
    input.placeholder = "Ask AI to write…";
    input.setAttribute("aria-label", "AI prompt");
    input.enterKeyHint = "send";
    const busy = document.createElement("span");
    busy.className = "cm-ai-busy";
    busy.innerHTML = '<span class="spinner"></span>Writing…';
    box.append(input, busy);
    input.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close(view);
      } else if (e.key === "Enter" && !e.isComposing) {
        e.preventDefault();
        void run(view, input.value, this.actions);
      }
    });
    this.updateDOM(box);
    // Ready to type as soon as it's there.
    requestAnimationFrame(() => input.focus({ preventScroll: true }));
    return box;
  }

  /** The same field going busy or idle keeps its DOM, and so what was typed in it. */
  updateDOM(dom: HTMLElement) {
    if (dom.dataset.prompt && dom.dataset.prompt !== String(this.prompt.id)) return false;
    dom.dataset.prompt = String(this.prompt.id);
    dom.classList.toggle("busy", this.prompt.busy);
    const input = dom.querySelector("input")!;
    input.readOnly = this.prompt.busy;
    dom.setAttribute("aria-busy", String(this.prompt.busy));
    return true;
  }

  // The field handles its own typing, clicks and keys.
  ignoreEvent() {
    return true;
  }
}

const promptField = StateField.define<Prompt | null>({
  create: () => null,
  update(prompt, tr) {
    if (prompt && tr.docChanged) prompt = { ...prompt, pos: tr.changes.mapPos(prompt.pos, -1) };
    for (const e of tr.effects) {
      if (e.is(setPrompt)) prompt = e.value;
      else if (e.is(setBusy) && prompt) prompt = { ...prompt, busy: e.value };
    }
    return prompt;
  },
});

/** Removes the field and puts the cursor back where it was. */
function close(view: EditorView) {
  const prompt = view.state.field(promptField);
  if (!prompt) return;
  view.dispatch({ effects: setPrompt.of(null), selection: { anchor: view.state.doc.lineAt(prompt.pos).to } });
  view.focus();
}

/** A line with nothing on it but maybe a list marker, checkbox or quote mark, like one Enter just continued. */
const BLANK = /^\s*(?:(?:[-*+]|\d+[.)])\s+(?:\[[ xX]\]\s*)?|>\s*)?$/;

/** Where the written text goes: a blank line is replaced, else it goes on a new line under the field's. */
function placeFor(view: EditorView, pos: number): { at: number; to: number; before: string } {
  const line = view.state.doc.lineAt(pos);
  return BLANK.test(line.text) ? { at: line.from, to: line.to, before: "" } : { at: line.to, to: line.to, before: "\n" };
}

async function run(view: EditorView, text: string, actions: AiPromptActions) {
  const prompt = view.state.field(promptField);
  if (!prompt || prompt.busy) return;
  if (!text.trim()) return close(view);
  view.dispatch({ effects: setBusy.of(true) });
  try {
    const { at } = placeFor(view, prompt.pos);
    const written = await actions.write(text.trim(), view.state.doc.toString(), at);
    // Still the same field, and the editor is still open.
    if (!view.dom.isConnected || view.state.field(promptField)?.id !== prompt.id) return;
    const now = placeFor(view, view.state.field(promptField)!.pos);
    const insert = now.before + written;
    const changes: ChangeSpec = { from: now.at, to: now.to, insert };
    actions.commit(view, {
      changes,
      effects: setPrompt.of(null),
      selection: { anchor: now.at + insert.length },
      scrollIntoView: true,
      userEvent: "input.ai",
    });
    view.focus();
  } catch {
    // The caller said why; the prompt stays to try again.
    if (view.dom.isConnected && view.state.field(promptField)?.id === prompt.id) {
      view.dispatch({ effects: setBusy.of(false) });
      view.dom.querySelector<HTMLInputElement>(".cm-ai-prompt input")?.focus();
    }
  }
}

/** Puts a prompt field under the cursor's line, replacing any other. */
export function openAiPrompt(view: EditorView) {
  view.dispatch({ effects: setPrompt.of({ id: ++ids, pos: view.state.selection.main.head, busy: false }) });
}

export function aiPrompt(actions: AiPromptActions) {
  const decorations = EditorView.decorations.compute([promptField], (state): DecorationSet => {
    const prompt = state.field(promptField);
    if (!prompt) return Decoration.none;
    const widget = Decoration.widget({ widget: new PromptWidget(prompt, actions), block: true, side: 1 });
    return Decoration.set([widget.range(state.doc.lineAt(prompt.pos).to)]);
  });
  return [promptField, decorations];
}
