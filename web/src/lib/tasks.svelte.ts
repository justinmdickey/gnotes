import { api, ApiError, type TaskItem } from "./api";
import { cache, tickOffline, unreachable } from "./offline";
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

/** Notes with a copy on this device, whose tasks can be ticked offline; refreshed as Tasks opens offline. */
export const onDevice = $state({ ids: new Set<string>() });

export async function refreshOnDevice() {
  onDevice.ids = new Set(await cache.noteIds());
}

/**
 * Ticks or unticks a task in its note. Shown at once; put back if it can't be done. With no server,
 * it changes the note's copy on this device instead, which goes up on reconnecting.
 */
export async function setDone(task: TaskItem, done: boolean): Promise<boolean> {
  const item = tasks.items?.find((t) => t.note === task.note && t.line === task.line);
  if (item) item.done = done;
  const offline = async () => {
    if (await tickOffline(task, done)) {
      if (tasks.items) void cache.saveTasks($state.snapshot(tasks.items));
      return true;
    }
    if (item) item.done = !done;
    toast("That note isn't on this device, so it can't be changed offline");
    return false;
  };
  if (app.status !== "online") return offline();
  try {
    await api.toggleTask(task, done);
    return true;
  } catch (err) {
    if (unreachable(err)) return offline();
    if (item) item.done = !done;
    if (err instanceof ApiError && err.status === 409) {
      toast("That task changed in its note");
      void refreshTasks();
    } else {
      toast(err instanceof ApiError ? err.message : "Couldn't reach the server");
    }
    return false;
  }
}
