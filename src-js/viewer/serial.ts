// One lane for every read the viewer's poll makes against the server.
//
// **Why reads queue rather than overlap.** Each element read that builds a body
// costs the server a full assemble, and concurrent entity reads were measured
// taking it from 245 MB to 904 MB resident (see `pollInOrder`). The 2-second
// tick already runs its reads in series behind a re-entry guard; a storey
// switch used to start a second series BESIDE it, so a level change during a
// tick asked the server for every layer twice at once. Both now go through one
// lane.
//
// No DOM and no fetch here, so the ordering is testable on its own.

export class Serial {
  private tail: Promise<unknown> = Promise.resolve();

  /** Run `job` after everything already queued. A job that fails does not
   *  stop the lane: the next one still runs. */
  run<T>(job: () => Promise<T>): Promise<T> {
    const result = this.tail.then(job);
    this.tail = result.catch(() => undefined);
    return result;
  }
}

/**
 * A job that is queued at most once at a time.
 *
 * Asked again while its run is still WAITING in the lane, it shares that run:
 * the job reads the page's scope when it starts, so one queued run already
 * answers every request made before it began. Asked while it is RUNNING, it
 * queues another, because the scope may have moved after the running one read
 * it. Five storey switches during one slow tick therefore cost one follow-up
 * read, not five.
 */
export function coalesced<T>(lane: Serial, job: () => Promise<T>): () => Promise<T> {
  let waiting: Promise<T> | null = null;
  return () => {
    if (waiting) return waiting;
    const queued = lane.run(async () => {
      waiting = null;
      return job();
    });
    waiting = queued;
    return queued;
  };
}
