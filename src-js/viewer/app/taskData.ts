// The room tasks behind the "Room tasks" colouring.
//
// **Read only while a zone colours by them**, and read again every few seconds, because a Revit
// session reporting is the event the colouring exists to show. Counted rather than owned by one
// zone: two zones can both colour by tasks, and the first to stop must not stop the other.

import { fetchJson } from "./api.js";
import { getState, setRoomTasks } from "./store.js";
import { indexTasks, type TaskRow } from "../taskStatus.js";

const EVERY_MS = 8000;

let watchers = 0;
let timer: ReturnType<typeof setInterval> | null = null;
let busy = false;

async function read(): Promise<void> {
  const project = getState().scope.projectId;
  if (!project || busy) return;
  busy = true;
  try {
    const tasks = await fetchJson<TaskRow[]>(`/projects/${encodeURIComponent(project)}/tasks`);
    setRoomTasks(indexTasks(tasks));
  } catch {
    // A failed read keeps what is drawn: stale colours beat a plan that flashes back to grey.
  } finally {
    busy = false;
  }
}

/** Start reading; returns the stop function, for an effect's cleanup. */
export function watchRoomTasks(): () => void {
  watchers += 1;
  if (watchers === 1) {
    void read();
    timer = setInterval(() => void read(), EVERY_MS);
  } else if (getState().roomTasks === null) void read();
  return () => {
    watchers -= 1;
    if (watchers === 0 && timer !== null) {
      clearInterval(timer);
      timer = null;
      setRoomTasks(null);
    }
  };
}
