// Shared UI state and motion: screen size, toasts, confirmations and Svelte transitions.
import { cubicOut, backOut } from "svelte/easing";
import type { TransitionConfig } from "svelte/transition";

const phoneQuery = matchMedia("(max-width: 700px)");
/** Wide enough for the sidebar to stay out, not a drawer. */
const wideQuery = matchMedia("(min-width: 1001px)");
const reducedQuery = matchMedia("(prefers-reduced-motion: reduce)");

export const media = $state({ phone: phoneQuery.matches, wide: wideQuery.matches, reduced: reducedQuery.matches });
phoneQuery.addEventListener("change", (e) => (media.phone = e.matches));
wideQuery.addEventListener("change", (e) => (media.wide = e.matches));
reducedQuery.addEventListener("change", (e) => (media.reduced = e.matches));

/** Installed to the home screen, where there's no browser back swipe of its own. */
export const standalone =
  matchMedia("(display-mode: standalone)").matches || (navigator as { standalone?: boolean }).standalone === true;

const ms = (n: number) => (media.reduced ? 0 : n);

export interface Toast {
  id: number;
  text: string;
  action?: { label: string; run: () => void };
}

export interface Confirm {
  title: string;
  body?: string;
  confirm: string;
  destructive?: boolean;
  resolve: (ok: boolean) => void;
}

export const ui = $state({ toasts: [] as Toast[], confirm: null as Confirm | null });

let nextToast = 1;

/** An AdwToast: a short message at the bottom, optionally with one action like Undo. */
export function toast(text: string, action?: Toast["action"], timeout = action ? 5000 : 2600) {
  const id = nextToast++;
  // One at a time, like libadwaita: a new toast replaces the old one.
  ui.toasts = [{ id, text, action }];
  setTimeout(() => dismissToast(id), timeout);
}

export function dismissToast(id: number) {
  ui.toasts = ui.toasts.filter((t) => t.id !== id);
}

/** A styled replacement for confirm(). */
export function ask(opts: Omit<Confirm, "resolve">): Promise<boolean> {
  ui.confirm?.resolve(false);
  return new Promise((resolve) => {
    ui.confirm = {
      ...opts,
      resolve: (ok) => {
        ui.confirm = null;
        resolve(ok);
      },
    };
  });
}

/** Dialog panel: a bottom sheet on phones, a gentle zoom elsewhere. */
export function sheet(_node: Element): TransitionConfig {
  if (media.phone) {
    return { duration: ms(300), easing: cubicOut, css: (t) => `transform: translateY(${(1 - t) * 100}%)` };
  }
  return {
    duration: ms(200),
    easing: cubicOut,
    css: (t) => `opacity: ${t}; transform: scale(${0.92 + 0.08 * t})`,
  };
}

/** Popover menus grow out of the button that opened them. */
export function pop(_node: Element): TransitionConfig {
  return {
    duration: ms(170),
    easing: backOut,
    css: (t) => `opacity: ${Math.min(1, t * 1.5)}; transform: scale(${0.9 + 0.1 * t}) translateY(${(1 - t) * -6}px)`,
  };
}

/** A full screen that slides in from the right on phones and fades in elsewhere. */
export function page(_node: Element): TransitionConfig {
  if (media.phone) {
    return { duration: ms(340), easing: cubicOut, css: (t) => `transform: translateX(${(1 - t) * 100}%)` };
  }
  return { duration: ms(200), easing: cubicOut, css: (t) => `opacity: ${t}; transform: scale(${0.98 + 0.02 * t})` };
}

export function fadeIn(_node: Element, { duration = 200 } = {}): TransitionConfig {
  return { duration: ms(duration), css: (t) => `opacity: ${t}` };
}

/** Toasts rise from the bottom edge. */
export function rise(_node: Element): TransitionConfig {
  return {
    duration: ms(260),
    easing: backOut,
    css: (t) => `opacity: ${Math.min(1, t * 2)}; transform: translateY(${(1 - t) * 24}px) scale(${0.96 + 0.04 * t})`,
  };
}

/** Small things (avatars, badges) popping in. */
export function bloom(_node: Element): TransitionConfig {
  return { duration: ms(220), easing: backOut, css: (t) => `opacity: ${t}; transform: scale(${0.5 + 0.5 * t})` };
}

/** Adds data-scrolled to the parent while the element is scrolled, so its header can show a divider. */
export function scrollEdge(node: HTMLElement) {
  const target = node.parentElement!;
  const update = () => target.toggleAttribute("data-scrolled", node.scrollTop > 2);
  node.addEventListener("scroll", update, { passive: true });
  update();
  return { destroy: () => node.removeEventListener("scroll", update) };
}

/**
 * Moves an element to the end of <body>, so a popover floats above every pane instead of
 * being clipped or covered by the pane it was opened from.
 */
export function portal(node: HTMLElement) {
  document.body.append(node);
  return { destroy: () => node.remove() };
}
