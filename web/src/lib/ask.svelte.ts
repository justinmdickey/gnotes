import { api, ApiError, type AskMessage, type AskNote, type SnippetPart } from "./api";

/**
 * Ask: a conversation with your notes. Each question shows the notes found for it as cards, then
 * an answer written from them with numbered links. Follow-ups build on the turns before. The
 * conversation lives here, not in the screen, so it stays while you visit other tabs; it lasts
 * until New Chat or logging out, and only the app keeps it.
 */
export interface Turn {
  question: string;
  /** The notes found; null while searching. */
  notes: AskNote[] | null;
  terms: string[];
  /** The answer so far, as it streams in. */
  answer: string;
  /** Set once the answer is finished: the notes it cites. */
  cited: number[] | null;
  /** The notes didn't say. */
  none: boolean;
  error: string;
}

export const chat = $state({
  /** What's typed in the field: the next question. */
  draft: "",
  turns: [] as Turn[],
  busy: false,
});

/** Stops the answer being written when the conversation ends. */
let stop = new AbortController();

/** Drops the conversation and stops any answer still coming. */
export function endChat() {
  stop.abort();
  stop = new AbortController();
  chat.turns = [];
  chat.busy = false;
  chat.draft = "";
}

/** Starts a new conversation with `question`. */
export function startChat(question: string) {
  endChat();
  void ask(question);
}

/** The earlier turns as the server wants them: question, then the answer, for each one answered. */
function history(): AskMessage[] {
  return chat.turns.flatMap((t): AskMessage[] =>
    t.answer && t.cited ? [{ role: "user", text: t.question }, { role: "assistant", text: t.answer }] : [{ role: "user", text: t.question }],
  );
}

/** Asks the next question of the conversation. */
export async function ask(text: string) {
  const messages: AskMessage[] = [...history(), { role: "user", text }];
  chat.turns.push({ question: text, notes: null, terms: [], answer: "", cited: null, none: false, error: "" });
  const turn = chat.turns[chat.turns.length - 1];
  const signal = stop.signal;
  chat.busy = true;
  try {
    const done = await api.ask(
      messages,
      {
        onSources: (s) => {
          turn.notes = s.notes;
          turn.terms = s.terms;
        },
        onText: (piece) => (turn.answer += piece),
      },
      signal,
    );
    turn.answer = done.answer ?? "";
    turn.none = !done.answer;
    turn.cited = done.cited;
  } catch (err) {
    if (signal.aborted) return;
    turn.error = err instanceof ApiError && err.status ? err.message : "Couldn't reach the server";
  } finally {
    if (!signal.aborted) chat.busy = false;
  }
}

/** The cards to show: every note found, then only the ones the finished answer cites. */
export function cards(turn: Turn): AskNote[] {
  const notes = turn.notes ?? [];
  if (!turn.cited?.length) return notes;
  return turn.cited.flatMap((n) => notes.filter((note) => note.n === n));
}

/** The answer as text and [n] citations, which become links to their notes. Numbers that aren't a found note are dropped. */
export function answerParts(turn: Turn) {
  const out: ({ text: string } | AskNote)[] = [];
  let last = 0;
  for (const m of turn.answer.matchAll(/\s?\[(\d+(?:\s*,\s*\d+)*)\]/g)) {
    out.push({ text: turn.answer.slice(last, m.index) });
    for (const n of m[1].split(",").map(Number)) {
      const note = turn.notes?.find((s) => s.n === n);
      if (note) out.push(note);
    }
    last = m.index + m[0].length;
  }
  out.push({ text: turn.answer.slice(last) });
  return out;
}

/** A title split into words, the ones searched for as hits, like the snippet under it. */
export function titleParts(title: string, terms: string[]): SnippetPart[] {
  return title.split(/(\s+)/).map((word) => {
    const w = word.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "").toLowerCase();
    return { text: word, hit: !!w && terms.some((t) => w.startsWith(t)) };
  });
}
