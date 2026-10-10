export const REVEAL_DURATION = 650;
export type RevealState = { key: string; context: string; current: ReadonlySet<string>; entering: ReadonlyMap<string, number>; retiring: ReadonlyMap<string, number> };
export function revealState(key: string, context: string, ids: readonly string[]): RevealState {
  return { key, context, current: new Set(ids), entering: new Map(), retiring: new Map() };
}
/** Retiring stars remain in the renderer while Positions returns them to their hub. */
export function retargetReveal(previous: RevealState, key: string, context: string, ids: readonly string[], eligible: ReadonlySet<string>, now: number, reduced: boolean): RevealState {
  if (reduced || previous.context !== context) return revealState(key, context, ids);
  const current = new Set(ids), entering = new Map(previous.entering), retiring = new Map(previous.retiring);
  // Positions retargets the entire layout from its observed coordinates, including
  // stars already returning from an earlier page. Give those the same new deadline.
  if (previous.key !== key) for (const id of retiring.keys()) retiring.set(id, now);
  for (const id of previous.current) if (!current.has(id) && eligible.has(id)) retiring.set(id, now);
  for (const id of current) {
    // An interrupted return reverses from its current physical position without another fade-in.
    if (!previous.current.has(id) && !retiring.has(id)) entering.set(id, now);
    retiring.delete(id);
  }
  for (const [id, start] of retiring) if (!eligible.has(id) || now - start >= REVEAL_DURATION) retiring.delete(id);
  for (const [id, start] of entering) if (!current.has(id) || now - start >= REVEAL_DURATION) entering.delete(id);
  return { key, context, current, entering, retiring };
}
export function revealOpacity(state: RevealState, id: string, now: number): number {
  const returning = state.retiring.get(id), arriving = state.entering.get(id);
  // Keep the moving stars legible; dissolve only as they reach the compact glyph.
  const t = returning !== undefined ? 1 - Math.max(0, Math.min(1, ((now - returning) / REVEAL_DURATION - .72) / .28))
    : arriving !== undefined ? Math.max(0, Math.min(1, (now - arriving) / 160)) : 1;
  return t * t * (3 - 2 * t);
}
