/**
 * A row's actions without opening it: a long-press on touch, a right-click with a mouse.
 * `open` gets the spot to show a menu at, and returns false when the row has no actions,
 * so a right-click there keeps the browser's own menu.
 *
 * A finger that moves (a scroll) or lifts early is an ordinary tap or scroll. Once the press
 * opens the menu, the click that follows is swallowed, so the row doesn't open as well.
 */
export function longPress(node: HTMLElement, open: (x: number, y: number) => boolean) {
  let handler = open;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let start = { x: 0, y: 0 };
  /** The press opened the menu; the click that ends it is not a tap. */
  let fired = false;
  /** The last press was a finger or pen, whose own long-press brings the browser's context menu. */
  let touch = false;

  const cancel = () => clearTimeout(timer);

  function down(e: PointerEvent) {
    fired = false;
    touch = e.pointerType !== "mouse";
    if (!touch || !e.isPrimary) return;
    start = { x: e.clientX, y: e.clientY };
    cancel();
    timer = setTimeout(() => {
      if (!handler(start.x, start.y)) return;
      fired = true;
      navigator.vibrate?.(10);
    }, 500);
  }

  function move(e: PointerEvent) {
    if (Math.hypot(e.clientX - start.x, e.clientY - start.y) > 10) cancel();
  }

  function menu(e: MouseEvent) {
    // A finger's long-press is the timer's; the browser's menu would only cover ours.
    if (touch) return e.preventDefault();
    if (handler(e.clientX, e.clientY)) e.preventDefault();
  }

  /** Swallows the click that may end the press, wherever it lands: the menu's backdrop is over the row by then. */
  function swallow(e: MouseEvent) {
    e.preventDefault();
    e.stopImmediatePropagation();
    window.removeEventListener("click", swallow, true);
  }

  function up() {
    cancel();
    if (!fired) return;
    fired = false;
    window.addEventListener("click", swallow, true);
    // Phones usually send no click after a long-press at all.
    setTimeout(() => window.removeEventListener("click", swallow, true), 400);
  }

  node.addEventListener("pointerdown", down);
  node.addEventListener("pointermove", move);
  node.addEventListener("pointerup", up);
  // Scrolling takes the pointer over, which cancels the press.
  node.addEventListener("pointercancel", cancel);
  node.addEventListener("dragstart", cancel);
  node.addEventListener("contextmenu", menu);
  // No text selection or iOS callout under a held finger.
  node.style.setProperty("-webkit-touch-callout", "none");
  node.style.userSelect = "none";
  return {
    update: (next: typeof open) => (handler = next),
    destroy() {
      cancel();
      node.removeEventListener("pointerdown", down);
      node.removeEventListener("pointermove", move);
      node.removeEventListener("pointerup", up);
      node.removeEventListener("pointercancel", cancel);
      node.removeEventListener("dragstart", cancel);
      node.removeEventListener("contextmenu", menu);
    },
  };
}
