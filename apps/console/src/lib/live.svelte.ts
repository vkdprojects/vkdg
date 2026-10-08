/**
 * Live-data helpers. Every screen that refreshes itself uses these, so the
 * "pause while the tab is hidden" rule and cleanup live in one place.
 *
 * Call from a component's top-level script (they register effects).
 */

/**
 * Run `fn` now and every `ms` while the tab is visible; resumes immediately on
 * return. Overlapping runs are skipped. Returns `refresh()` for manual reloads
 * (e.g. a Refresh button) that share the same in-flight guard.
 */
export function poll(fn: () => Promise<unknown>, ms = 5000) {
  let busy = false;
  const run = async () => {
    if (busy) return;
    busy = true;
    try { await fn(); } finally { busy = false; }
  };

  $effect(() => {
    let timer = 0;
    const sync = () => {
      window.clearInterval(timer);
      if (document.visibilityState !== 'visible') return;
      void run();
      timer = window.setInterval(run, ms);
    };
    sync();
    document.addEventListener('visibilitychange', sync);
    return () => { window.clearInterval(timer); document.removeEventListener('visibilitychange', sync); };
  });

  return { refresh: run };
}

/**
 * A reactive "now" that ticks every `ms` only while `active()` is true, for
 * countdowns and relative times without a timer per row.
 */
export function clock(ms = 1000, active: () => boolean = () => true) {
  let now = $state(Date.now());
  $effect(() => {
    if (!active()) return;
    now = Date.now();
    const t = window.setInterval(() => (now = Date.now()), ms);
    return () => window.clearInterval(t);
  });
  return { get now() { return now; } };
}
