# Console: how the code is organized

Run `bun run lint` before pushing. It runs `lint:tokens`, then `lint:dup`, then `check`.

## Styling

| Need | Where |
|---|---|
| A color, size, radius, spacing, z-index, or duration | `src/styles/tokens.css`. Never write a raw value in a `.svelte` file; `lint:tokens` rejects it. |
| A layout or component class used by 2+ screens | `src/styles/components.css` (`.page`, `.panel`, `.dialog-*`, `.drawer-*`, `.segmented`, `.chip`, `.field*`, …). |
| Element defaults (inputs, tables, headings) | `src/styles/base.css`. |
| Anything else | The component's own `<style>`. |

Theme: tokens recompute per `[data-theme]` subtree, so `data-theme="dark"` on any element pins that area dark. The brand hues come from the logo; to retheme, change the knobs (`--hue-*`, `--l-*`, `--c-*`).

## Logic you should not rewrite

| Need | Use |
|---|---|
| Refresh data while the tab is visible | `poll(load, ms)` from `$lib/live.svelte.ts`. It returns `{ refresh }` for a Refresh button. |
| Countdowns or relative times that tick | `clock(ms, active)` from `$lib/live.svelte.ts`. |
| A connection or request status label | `connectionStatusLabel` / `requestStatusLabel` from `$lib/status.ts`. |
| Cooldown checks and formatting | `isCooling`, `secondsUntil`, `formatCountdown` from `$lib/status.ts`. |
| Latency | `formatDuration` from `$lib/status.ts`. |
| Numbers, dates, relative time | `$lib/format.ts`. |
| The add-connection wizard | `AddConnectionDialog` component and `$lib/connection-wizard.ts`. |
| A delete confirmation | `ConfirmDeleteDialog` component. |
| A provider logo | `ProviderLogo` component. Logos live in `static/providers/<id>.svg`. |

## Files

- Reusable across routes: `src/lib/components/`, exported from `index.ts`.
- Used by one route only: next to that route, e.g. `src/routes/keys/KeyCard.svelte`.
- A page orchestrates: it loads data, holds the state, and composes components. If a page grows past ~350 lines, look for the next natural seam.
- `lint:dup` (jscpd, config in `.jscpd.json`) fails above 1% duplication. When it flags a clone, extract the shared piece; don't tune the threshold.

## Dev against a running gateway

```bash
VKDG_ADMIN_PROXY=https://vkdg.vixpi.host VITE_ADMIN_URL= bun run dev
```

`/admin/*` is proxied same-origin, so the session cookie works. Sign in with that gateway's admin password.
