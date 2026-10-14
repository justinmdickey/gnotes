import { api, ApiError, type TaskItem } from "./api";
import { cache, unreachable } from "./offline";
import { app } from "./store.svelte";
import { toast } from "./ui.svelte";

/** Every task in the notes you can see, from the server's index; null until the first load. */
export const tasks = $state({ items: null as TaskItem[] | null });

let refreshing: Promise<void> | null = null;
let stale = false;

/**
 * Refetches the list while it's on screen; otherwise it waits until Tasks opens again. Every edit to a
 * task line anywhere says tasks changed, so bursts are coalesced like the tree's.
 */
export function refreshTasks(): Promise<void> {
  if (app.view.kind !== "tasks") {
    stale = true;
    return Promise.resolve();
  }
  if (refreshing) {
    stale = true;
    return refreshing;
  }
  stale = false;
  refreshing = api
    .tasks()
    .then((items) => {
      tasks.items = items;
      void cache.saveTasks($state.snapshot(items));
    })
    .catch(async (e) => {
      // Offline, the list from last time shows instead.
      if (unreachable(e)) tasks.items ??= (await cache.tasks()) ?? [];
      else console.warn("tasks refresh failed", e);
    })
    .finally(() => {
      refreshing = null;
      if (stale) void refreshTasks();
    });
  return refreshing;
}

/** Opening Tasks: what was saved shows at once, then the server's list. */
export function openTasks() {
  if (!tasks.items) void cache.tasks().then((saved) => (tasks.items ??= saved ?? null));
  void refreshTasks();
}

export function forgetTasks() {
  tasks.items = null;
}

/** Ticks or unticks a task in its note. Shown at once; put back if the server says no. */
export async function setDone(task: TaskItem, done: boolean): Promise<boolean> {
  const item = tasks.items?.find((t) => t.note === task.note && t.line === task.line);
  if (item) item.done = done;
  try {
    await api.toggleTask(task, done);
    return true;
  } catch (err) {
    if (item) item.done = !done;
    if (err instanceof ApiError && err.status === 409) {
      toast("That task changed in its note");
      void refreshTasks();
    } else {
      toast(err instanceof ApiError ? err.message : "Tasks can't be checked off offline yet");
    }
    return false;
  }
}
