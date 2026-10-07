// Pure logic shared by the per-account window bars and the all-accounts overview.

import type { Account, UsageWindowKind } from './api.js';

export type UsageColumn = UsageWindowKind | 'credits';

/** Column order in the overview; a column only appears if some account reports it. */
const COLUMN_ORDER: UsageColumn[] = ['five_hour', 'weekly', 'weekly_sonnet', 'weekly_opus', 'credits'];

export interface UsageCell {
  /** Consumed share, clamped to 0..100. */
  percent: number;
  /** Unix seconds; null when the upstream gave no reset time. */
  resetsAt: number | null;
}

export interface UsageRow {
  account: Account;
  /** `unavailable`: the upstream could not be read — shown as such, never as 0%. */
  state: 'reported' | 'unavailable';
  cells: Partial<Record<UsageColumn, UsageCell>>;
  /** Highest consumption across the account's cells; null when there is none. */
  peak: number | null;
}

/** Share of a window still available, clamped so an overshoot never shows a negative. */
export function remainingPercent(usedPercent: number): number {
  return Math.min(100, Math.max(0, 100 - usedPercent));
}

/** Time until a reset as a unit/amount for `Intl.RelativeTimeFormat`; null when unknown. */
export function resetDelta(
  resetsAt: number | null | undefined,
  nowMs: number,
): { n: number; unit: 'minute' | 'hour' | 'day' } | null {
  if (resetsAt == null) return null;
  const minutes = Math.max(0, Math.round((resetsAt * 1000 - nowMs) / 60_000));
  if (minutes < 60) return { n: minutes, unit: 'minute' };
  const hours = Math.round(minutes / 60);
  if (hours < 24) return { n: hours, unit: 'hour' };
  return { n: Math.round(hours / 24), unit: 'day' };
}

const clampPercent = (p: number) => Math.min(100, Math.max(0, p));

/**
 * One row per account that has a usage concept (windows or credits), most
 * consumed first so the account about to hit its limit is on top. Accounts the
 * provider could not read stay in the list, flagged; accounts of providers with
 * no usage concept are omitted.
 */
export function usageRows(accounts: Account[]): UsageRow[] {
  const rows: UsageRow[] = [];
  for (const account of accounts) {
    if (account.credits_source !== 'reported' && account.credits_source !== 'unavailable') continue;
    const cells: UsageRow['cells'] = {};
    for (const w of account.usage_windows ?? []) {
      cells[w.kind] = { percent: clampPercent(w.used_percent), resetsAt: w.resets_at };
    }
    if (account.credits_used != null && account.credits_limit != null && account.credits_limit > 0) {
      cells.credits = {
        percent: clampPercent((account.credits_used * 100) / account.credits_limit),
        resetsAt: account.credits_period_end ?? null,
      };
    }
    const percents = Object.values(cells).map((c) => c.percent);
    rows.push({
      account,
      state: account.credits_source,
      cells,
      peak: percents.length > 0 ? Math.max(...percents) : null,
    });
  }
  return rows.sort((a, b) => (b.peak ?? -1) - (a.peak ?? -1));
}

export function usageColumns(rows: UsageRow[]): UsageColumn[] {
  return COLUMN_ORDER.filter((col) => rows.some((r) => r.cells[col] !== undefined));
}
