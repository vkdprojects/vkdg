// Pure logic behind the usage windows and the all-accounts overview.
// Each test names the wrong implementation it defeats.

import { describe, it, expect } from 'vitest';
import type { Account } from '../api';
import { remainingPercent, resetDelta, usageColumns, usageRows } from '../usage';

function account(over: Partial<Account> & { id: string }): Account {
  return {
    provider: 'claude-code',
    label: over.id,
    expires_at: null,
    has_refresh_token: true,
    status: 'active',
    ...over,
  };
}

describe('remainingPercent', () => {
  // Wrong impl: 100 - used with no clamp shows "-8% left" when upstream overshoots,
  // or a bar wider than its track.
  it('is 100 - used, clamped to 0..100', () => {
    expect(remainingPercent(49)).toBe(51);
    expect(remainingPercent(108)).toBe(0);
    expect(remainingPercent(-3)).toBe(100);
  });
});

describe('resetDelta', () => {
  const now = 1_000_000_000_000; // ms
  const at = (secondsAhead: number) => Math.floor(now / 1000) + secondsAhead;

  // Wrong impl: a missing reset time becomes 0 → "resets now".
  it('is null when the upstream gave no reset time', () => {
    expect(resetDelta(null, now)).toBeNull();
    expect(resetDelta(undefined, now)).toBeNull();
  });

  // Wrong impl: always seconds or always days; unit not chosen by magnitude.
  it('picks minutes, hours or days by magnitude', () => {
    expect(resetDelta(at(25 * 60), now)).toEqual({ n: 25, unit: 'minute' });
    expect(resetDelta(at(3 * 3600), now)).toEqual({ n: 3, unit: 'hour' });
    expect(resetDelta(at(5 * 86400), now)).toEqual({ n: 5, unit: 'day' });
  });

  // Wrong impl: a stale reset time yields a negative "in -4 minutes".
  it('never goes negative once the reset time has passed', () => {
    expect(resetDelta(at(-240), now)).toEqual({ n: 0, unit: 'minute' });
  });
});

describe('usageRows', () => {
  const claudeMain = account({
    id: 'claude-main',
    credits_source: 'reported',
    usage_windows: [
      { kind: 'weekly_sonnet', used_percent: 20, resets_at: 1_790_000_000 },
      { kind: 'five_hour', used_percent: 49, resets_at: 1_789_000_000 },
    ],
  });
  const claudeSecond = account({
    id: 'claude-second',
    credits_source: 'reported',
    usage_windows: [{ kind: 'five_hour', used_percent: 91, resets_at: null }],
  });
  const kiro = account({
    id: 'kiro-1',
    provider: 'kiro',
    credits_source: 'reported',
    credits_used: 300,
    credits_limit: 1000,
    credits_period_end: 1_790_812_800,
  });
  const broken = account({ id: 'codex-down', provider: 'codex', credits_source: 'unavailable' });
  const apiKeyOnly = account({ id: 'plain', provider: 'openai' });

  // Wrong impl: overview shows only the top account, or in list order, hiding the
  // second Claude account that is closest to its limit.
  it('lists every metered account, highest consumption first', () => {
    const rows = usageRows([claudeMain, kiro, claudeSecond]);
    expect(rows.map((r) => r.account.id)).toEqual(['claude-second', 'claude-main', 'kiro-1']);
    expect(rows.map((r) => r.peak)).toEqual([91, 49, 30]);
  });

  // Wrong impl: fabricating a 0% row for accounts we know nothing about, or
  // dropping the unavailable ones so they look healthy.
  it('keeps unavailable accounts, flagged, and skips providers with no usage concept', () => {
    const rows = usageRows([apiKeyOnly, broken, claudeMain]);
    expect(rows.map((r) => r.account.id)).toEqual(['claude-main', 'codex-down']);
    const down = rows[1];
    expect(down.state).toBe('unavailable');
    expect(down.peak).toBeNull();
    expect(down.cells).toEqual({});
  });

  // Wrong impl: credits ratio computed against a missing/zero limit (NaN/Infinity),
  // or credits shown as a window.
  it('turns a credit meter into a percent cell and ignores a zero limit', () => {
    const [row] = usageRows([kiro]);
    expect(row.cells.credits).toEqual({ percent: 30, resetsAt: 1_790_812_800 });
    const [zero] = usageRows([{ ...kiro, credits_limit: 0 }]);
    expect(zero.cells.credits).toBeUndefined();
  });

  // Wrong impl: percent not clamped, so an overshoot distorts sorting and bar width.
  it('clamps percents to 0..100', () => {
    const [row] = usageRows([
      account({ id: 'x', credits_source: 'reported', usage_windows: [{ kind: 'five_hour', used_percent: 130, resets_at: null }] }),
    ]);
    expect(row.cells.five_hour?.percent).toBe(100);
    expect(row.peak).toBe(100);
  });
});

describe('usageColumns', () => {
  // Wrong impl: fixed five columns, so a Claude plan without a general weekly
  // window gets an always-empty "Weekly" column; or columns in arrival order.
  it('shows only the columns some account reports, in a stable order', () => {
    const rows = usageRows([
      account({
        id: 'a',
        credits_source: 'reported',
        usage_windows: [
          { kind: 'weekly_sonnet', used_percent: 10, resets_at: null },
          { kind: 'five_hour', used_percent: 5, resets_at: null },
        ],
      }),
      account({ id: 'k', provider: 'kiro', credits_source: 'reported', credits_used: 1, credits_limit: 10 }),
    ]);
    expect(usageColumns(rows)).toEqual(['five_hour', 'weekly_sonnet', 'credits']);
  });
});
