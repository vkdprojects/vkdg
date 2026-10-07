import { m } from '$lib/paraglide/messages.js';
import type { UsageColumn } from './usage.js';

/** Localised column/window labels, keyed by the wire `kind` (or `credits`). */
export const usageLabels: Record<UsageColumn, () => string> = {
  five_hour: m.usage_window_five_hour,
  weekly: m.usage_window_weekly,
  weekly_sonnet: m.usage_window_weekly_sonnet,
  weekly_opus: m.usage_window_weekly_opus,
  credits: m.usage_column_credits,
};
