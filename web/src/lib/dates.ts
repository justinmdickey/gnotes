// Due dates on tasks: "@YYYY-MM-DD" in a checkbox line (server/src/tasks.rs reads the same form).
import type { EditorView } from "@codemirror/view";

/** A day as "YYYY-MM-DD" in local time, the form due dates are written in. */
export function isoDay(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

export const today = () => isoDay(new Date());

export function inDays(n: number): string {
  const d = new Date();
  d.setDate(d.getDate() + n);
  return isoDay(d);
}

/** The coming Saturday; on a Saturday, the next one. */
export const weekend = () => inDays((6 - new Date().getDay() + 7) % 7 || 7);

const DAY = 24 * 60 * 60 * 1000;
const dateOf = (s: string) => new Date(`${s}T12:00:00`);

/** "Today", "Tomorrow", "2 days late", "Fri", "Oct 20", and whether it's late or today. */
export function dueLabel(due: string): { text: string; tone: "late" | "today" | "" } {
  const days = Math.round((dateOf(due).getTime() - dateOf(today()).getTime()) / DAY);
  if (Number.isNaN(days)) return { text: due, tone: "" };
  if (days === 0) return { text: "Today", tone: "today" };
  if (days === 1) return { text: "Tomorrow", tone: "" };
  if (days === -1) return { text: "Yesterday", tone: "late" };
  if (days < 0) return { text: `${-days} days late`, tone: "late" };
  return { text: dateOf(due).toLocaleDateString([], days < 7 ? { weekday: "short" } : { month: "short", day: "numeric" }), tone: "" };
}

/** An "@YYYY-MM-DD" standing on its own, as the server finds it. */
export const DUE = /(^|\s)@(\d{4}-\d{2}-\d{2})(?![\p{L}\p{N}])/u;

/** Sets the due date of the line the cursor is on (null takes it off), at the line's end, in one step. */
export function setDue(view: EditorView, date: string | null) {
  const line = view.state.doc.lineAt(view.state.selection.main.head);
  const m = DUE.exec(line.text);
  const changes = [];
  if (m) {
    const from = line.from + m.index;
    changes.push({ from, to: from + m[0].length, insert: "" });
  }
  if (date) changes.push({ from: line.to, insert: `${line.text.endsWith(" ") ? "" : " "}@${date}` });
  if (changes.length) view.dispatch({ changes, userEvent: "input.due" });
  view.focus();
}
