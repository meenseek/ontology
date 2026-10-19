/** Wait for committed classification changes. Timers only recover failed reads. */
export function watchGroupingChanges(read: (signal: AbortSignal) => Promise<void>): () => void {
  let source: EventSource | null = null, controller: AbortController | null = null;
  let retry: number | undefined, again = false, disposed = false;
  const clearRetry = () => { window.clearTimeout(retry); retry = undefined; };
  const refresh = async () => {
    if (disposed || !source || document.hidden) return;
    clearRetry();
    if (controller) { again = true; return; }
    const current = new AbortController(); controller = current;
    try { await read(current.signal); }
    catch (error) {
      const status = error && typeof error === "object" && "status" in error ? Number(error.status) : 0;
      const permanent = status >= 400 && status < 500 && status !== 409 && status !== 429;
      if (permanent) close();
      else if (!current.signal.aborted) retry = window.setTimeout(() => { void refresh(); }, 5000);
    } finally {
      controller = null;
      if (again) { again = false; void refresh(); }
    }
  };
  const close = () => {
    source?.close(); source = null; clearRetry(); again = false; controller?.abort();
  };
  const visibility = () => {
    if (disposed || document.hidden) { close(); return; }
    if (source) return;
    const opened = new EventSource("/api/grouping/events"); source = opened;
    // EventSource reconnects after transport failures; every new server stream
    // sends a catch-up notice, including changes missed while hidden/offline.
    opened.onmessage = () => { if (source === opened) void refresh(); };
  };
  document.addEventListener("visibilitychange", visibility);
  visibility();
  return () => { disposed = true; close(); document.removeEventListener("visibilitychange", visibility); };
}
