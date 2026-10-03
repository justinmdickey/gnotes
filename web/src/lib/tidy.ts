// Tidy Up's result goes into the note as the smallest set of edits that turns the old text into
// the new, so collaborators' cursors and edits in untouched lines are left alone, and it merges
// like any other typing.

export interface TextChange {
  from: number;
  to: number;
  insert: string;
}

/** Lines with their line breaks, so joining them gives the text back. */
const pieces = (text: string) => text.match(/[^\n]*\n|[^\n]+$/g) ?? [];

/** Above this many line pairs, the changed middle is replaced in one piece instead of diffed. */
const MAX_PAIRS = 4_000_000;

/**
 * Edits from `a` to `b`, in `a`'s positions, sorted and not overlapping (a CodeMirror ChangeSpec
 * list). Unchanged lines are matched by a longest common subsequence; each changed run of lines
 * then loses the characters it starts and ends with in common.
 */
export function changesBetween(a: string, b: string): TextChange[] {
  const A = pieces(a);
  const B = pieces(b);
  let start = 0;
  while (start < A.length && start < B.length && A[start] === B[start]) start++;
  let end = 0;
  while (end < A.length - start && end < B.length - start && A[A.length - 1 - end] === B[B.length - 1 - end]) end++;
  const am = A.slice(start, A.length - end);
  const bm = B.slice(start, B.length - end);
  const n = am.length;
  const m = bm.length;

  // Pairs of matching lines in the middle, in order.
  const pairs: [number, number][] = [];
  if (n * m <= MAX_PAIRS) {
    const w = m + 1;
    const lcs = new Uint32Array((n + 1) * w);
    for (let i = n - 1; i >= 0; i--)
      for (let j = m - 1; j >= 0; j--)
        lcs[i * w + j] = am[i] === bm[j] ? lcs[(i + 1) * w + j + 1] + 1 : Math.max(lcs[(i + 1) * w + j], lcs[i * w + j + 1]);
    for (let i = 0, j = 0; i < n && j < m; ) {
      if (am[i] === bm[j]) pairs.push([i++, j++]);
      else if (lcs[(i + 1) * w + j] >= lcs[i * w + j + 1]) i++;
      else j++;
    }
  }
  pairs.push([n, m]);

  let pos = A.slice(0, start).reduce((sum, p) => sum + p.length, 0);
  const changes: TextChange[] = [];
  let i = 0;
  let j = 0;
  for (const [pi, pj] of pairs) {
    const old = am.slice(i, pi).join("");
    const now = bm.slice(j, pj).join("");
    if (old !== now) {
      let head = 0;
      while (head < old.length && head < now.length && old[head] === now[head]) head++;
      let tail = 0;
      while (tail < old.length - head && tail < now.length - head && old[old.length - 1 - tail] === now[now.length - 1 - tail]) tail++;
      // Never split an emoji's surrogate pair.
      if (head > 0 && /[\uD800-\uDBFF]/.test(old[head - 1])) head--;
      if (tail > 0 && /[\uDC00-\uDFFF]/.test(old[old.length - tail])) tail--;
      changes.push({ from: pos + head, to: pos + old.length - tail, insert: now.slice(head, now.length - tail) });
    }
    pos += old.length + (pi < n ? am[pi].length : 0);
    i = pi + 1;
    j = pj + 1;
  }
  return changes;
}

/** `text` with `changes` applied, for checking. */
export function applyChanges(text: string, changes: TextChange[]): string {
  let out = "";
  let at = 0;
  for (const c of changes) {
    out += text.slice(at, c.from) + c.insert;
    at = c.to;
  }
  return out + text.slice(at);
}
