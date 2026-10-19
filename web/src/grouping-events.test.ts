import assert from "node:assert/strict";
import { test } from "node:test";
import { watchGroupingChanges } from "./grouping-events";

class Events {
  static opened: Events[] = [];
  onmessage: (() => void) | null = null;
  closed = false;
  constructor(readonly url: string) { Events.opened.push(this); }
  close() { this.closed = true; }
  notice() { this.onmessage?.(); }
}
class Visibility extends EventTarget {
  hidden = false;
  set(hidden: boolean) { this.hidden = hidden; this.dispatchEvent(new Event("visibilitychange")); }
}
async function fixture(run: (value: {
  doc: Visibility; reads: { signal: AbortSignal; resolve: () => void; reject: (status?: number) => void }[];
  tick: (ms: number) => void; stop: () => void;
}) => Promise<void>) {
  const keys = ["window", "document", "EventSource"] as const;
  const originals = keys.map(key => Object.getOwnPropertyDescriptor(globalThis, key));
  const doc = new Visibility(), reads: { signal: AbortSignal; resolve: () => void; reject: (status?: number) => void }[] = [];
  let now = 0, id = 0;
  const timers = new Map<number, { at: number; fn: () => void }>();
  Events.opened = [];
  Object.assign(globalThis, { document: doc, EventSource: Events, window: {
    setTimeout: (fn: () => void, ms: number) => { timers.set(++id, { at: now + ms, fn }); return id; },
    clearTimeout: (key: number) => timers.delete(key),
  } });
  const stop = watchGroupingChanges(signal => new Promise<void>((resolve, reject) => reads.push({ signal, resolve, reject: status => reject(Object.assign(new Error("synthetic read failure"), { status })) })));
  const tick = (ms: number) => {
    now += ms;
    for (const [key, timer] of timers) if (timer.at <= now) { timers.delete(key); timer.fn(); }
  };
  try { await run({ doc, reads, tick, stop }); }
  finally {
    stop();
    keys.forEach((key, index) => { const previous = originals[index]; if (previous) Object.defineProperty(globalThis, key, previous); else Reflect.deleteProperty(globalThis, key); });
  }
}
const settle = async () => { await Promise.resolve(); await Promise.resolve(); };

test("healthy idle streams never read on a timer; bursts coalesce without overlapping reads", async () => fixture(async ({ reads, tick }) => {
  assert.equal(Events.opened[0].url, "/api/grouping/events");
  tick(60_000); assert.equal(reads.length, 0);
  Events.opened[0].notice();
  for (let i = 0; i < 100; i++) Events.opened[0].notice();
  assert.equal(reads.length, 1);
  reads[0].resolve(); await settle(); assert.equal(reads.length, 2);
  reads[1].resolve(); await settle(); tick(60_000);
  assert.equal(reads.length, 2, "successful reads never schedule another read");
}));

test("only failed reads retry; success and cleanup clear recovery timers", async () => fixture(async ({ reads, tick, stop }) => {
  Events.opened[0].notice(); reads[0].reject(); await settle();
  tick(4999); assert.equal(reads.length, 1);
  tick(1); assert.equal(reads.length, 2);
  reads[1].resolve(); await settle(); tick(60_000); assert.equal(reads.length, 2);
  Events.opened[0].notice(); reads[2].reject(); await settle();
  stop(); tick(60_000); assert.equal(reads.length, 3);
}));

test("hiding aborts reads, closes the stream and ignores old notices; return catches up", async () => fixture(async ({ doc, reads, tick, stop }) => {
  const old = Events.opened[0]; old.notice();
  doc.set(true); assert(old.closed); assert(reads[0].signal.aborted);
  old.notice(); tick(60_000); assert.equal(reads.length, 1);
  reads[0].reject(); await settle();
  doc.set(false); assert.equal(Events.opened.length, 2);
  old.notice(); assert.equal(reads.length, 1);
  Events.opened[1].notice(); assert.equal(reads.length, 2);
  stop(); assert(Events.opened[1].closed); assert(reads[1].signal.aborted);
  reads[1].reject(); await settle(); doc.set(false); tick(60_000);
  assert.equal(Events.opened.length, 2); assert.equal(reads.length, 2);
}));

test("a new notice supersedes a failed-read retry", async () => fixture(async ({ reads, tick }) => {
  Events.opened[0].notice(); reads[0].reject(); await settle();
  tick(1000); Events.opened[0].notice(); assert.equal(reads.length, 2);
  reads[1].resolve(); await settle(); tick(60_000); assert.equal(reads.length, 2);
}));

for (const status of [403, 404, 410]) {
  test(`HTTP ${status} closes the subscription without retrying a permanent read failure`, async () => fixture(async ({ reads, tick }) => {
    const source = Events.opened[0]; source.notice(); reads[0].reject(status); await settle();
    assert(source.closed); tick(60_000); source.notice(); await settle();
    assert.equal(reads.length, 1);
  }));
}
