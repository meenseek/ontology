import test from "node:test";
import assert from "node:assert/strict";
import { PointerGesture } from "./pointer-gesture";

test("one press activates its original target once, regardless of delayed callbacks", () => {
  const gesture = new PointerGesture();
  gesture.begin("collapsed", 1, 50, 80);
  gesture.release(1, 50, 80);
  assert.equal(gesture.consume(), "collapsed");
  assert.equal(gesture.consume(), null);
  // A second real press is independent, with no time-based exclusion window.
  gesture.begin("other", 2, 50, 80);
  gesture.release(2, 50, 80);
  assert.equal(gesture.consume(), "other");
});
test("small jitter is a tap; a drag returning to its origin is not", () => {
  const gesture = new PointerGesture();
  gesture.begin("star", 1, 0, 0); gesture.move(1, 3, 3); gesture.release(1, 0, 0);
  assert.equal(gesture.consume(), "star");
  gesture.begin("star", 1, 0, 0); gesture.move(1, 6, 0); gesture.release(1, 0, 0);
  assert.equal(gesture.consume(), null);
});
test("cancel, leave before release, multi-touch cancellation and background never activate", () => {
  const gesture = new PointerGesture();
  gesture.begin("star", 1, 0, 0); gesture.cancel(); gesture.release(1, 0, 0);
  assert.equal(gesture.consume(), null);
  gesture.begin("star", 1, 0, 0); assert.equal(gesture.consume(), null);
  gesture.release(1, 0, 0); assert.equal(gesture.consume(), null);
  gesture.begin("star", 1, 0, 0); gesture.release(2, 0, 0);
  assert.equal(gesture.consume(), null);
  gesture.begin(null, 1, 0, 0); gesture.release(1, 0, 0);
  assert.equal(gesture.consume(), null);
});
