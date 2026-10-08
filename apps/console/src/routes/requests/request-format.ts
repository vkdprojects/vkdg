import type { RequestSummary } from '$lib/api.js';
import { formatNumber } from '$lib/format.js';
import { m } from '$lib/paraglide/messages.js';

/** Localized usage; each absent metric stays distinct from a reported numeric zero. */
export function fmtTokens(r: RequestSummary): string {
  return m.request_tokens_value({
    input: r.input_tokens == null ? m.request_metric_unavailable() : formatNumber(r.input_tokens),
    output: r.output_tokens == null ? m.request_metric_unavailable() : formatNumber(r.output_tokens),
  });
}

/** USD from microdollars; an explicit localized empty value when no price was reported. */
export function fmtCost(micro: number | null | undefined): string {
  if (micro == null) return m.request_metric_unavailable();
  const usd = micro / 1_000_000;
  return `$${usd < 0.01 ? usd.toFixed(6) : usd.toFixed(4)}`;
}
