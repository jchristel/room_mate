import { describe, expect, it } from "vitest";

import { Serial, coalesced } from "./serial.js";

/** A job the test finishes by hand, recording when it started. Duplicated
 *  rather than shared, per the house rule on per-module helpers. */
function gate(log: string[], name: string) {
  let open!: () => void;
  const done = new Promise<void>((resolve) => (open = resolve));
  return {
    job: async () => {
      log.push(`start ${name}`);
      await done;
      log.push(`end ${name}`);
    },
    open: () => open(),
  };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("Serial", () => {
  /** The property the fix exists for: a storey switch during a tick must not
   *  start its reads beside the tick's. */
  it("never starts a job while another is running", async () => {
    const lane = new Serial();
    const log: string[] = [];
    const a = gate(log, "tick");
    const b = gate(log, "storeys");
    const ranA = lane.run(a.job);
    const ranB = lane.run(b.job);
    await settle();
    expect(log).toEqual(["start tick"]);
    a.open();
    await ranA;
    await settle();
    expect(log).toEqual(["start tick", "end tick", "start storeys"]);
    b.open();
    await ranB;
  });

  it("keeps running after a job fails", async () => {
    const lane = new Serial();
    const failed = lane.run(async () => {
      throw new Error("boom");
    });
    await expect(failed).rejects.toThrow("boom");
    await expect(lane.run(async () => 7)).resolves.toBe(7);
  });
});

describe("coalesced", () => {
  it("shares one queued run between every request made before it starts", async () => {
    const lane = new Serial();
    const log: string[] = [];
    const tick = gate(log, "tick");
    let runs = 0;
    const layers = coalesced(lane, async () => {
      runs += 1;
    });
    const ranTick = lane.run(tick.job);
    const asks = [layers(), layers(), layers()];
    tick.open();
    await ranTick;
    await Promise.all(asks);
    expect(runs).toBe(1);
  });

  /** The scope may have moved after a running job read it, so a request made
   *  DURING the run is not answered by it. */
  it("queues another run when asked while running", async () => {
    const lane = new Serial();
    const log: string[] = [];
    const first = gate(log, "layers");
    let runs = 0;
    const layers = coalesced(lane, async () => {
      runs += 1;
      if (runs === 1) await first.job();
    });
    const one = layers();
    await settle();
    const two = layers();
    first.open();
    await Promise.all([one, two]);
    expect(runs).toBe(2);
  });
});
