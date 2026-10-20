/** A canvas press owns its target until release; renderer hover is never a click. */
export class PointerGesture {
  private press: { id: string | null; pointerId: number; x: number; y: number; moved: boolean; released: boolean } | null = null;

  begin(id: string | null, pointerId: number, x: number, y: number) {
    this.press = { id, pointerId, x, y, moved: false, released: false };
  }
  move(pointerId: number, x: number, y: number) {
    const press = this.press;
    if (press?.pointerId === pointerId && !press.released) press.moved ||= Math.hypot(x - press.x, y - press.y) >= 6;
  }
  release(pointerId: number, x: number, y: number) {
    this.move(pointerId, x, y);
    if (this.press?.pointerId === pointerId) this.press.released = true;
  }
  consume() {
    const press = this.press;
    // A drag-end on pointerleave can arrive before release. It cannot activate.
    this.press = null;
    return press?.released && !press.moved ? press.id : null;
  }
  cancel() { this.press = null; }
}
