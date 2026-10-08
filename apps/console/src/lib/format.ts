/**
 * Locale-aware number/date formatting, keyed to the active Paraglide locale
 * instead of the browser/server default. Use these everywhere the console
 * renders a number or date — never call `toLocaleString`/`Intl.*Format`
 * with an implicit locale.
 */
import { getLocale } from './paraglide/runtime.js';

/** Integer/decimal formatting, e.g. account credits, token counts. */
export function formatNumber(value: number): string {
  return new Intl.NumberFormat(getLocale()).format(value);
}

/** Short date+time, e.g. account expiry. */
export function formatDateTime(input: string | number | Date): string {
  return new Intl.DateTimeFormat(getLocale(), {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(input));
}

/** Short date only, e.g. credits period end. */
export function formatDate(input: string | number | Date): string {
  return new Intl.DateTimeFormat(getLocale(), {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  }).format(new Date(input));
}

/** Clock time only (HH:MM:SS), e.g. "updated at" / request timestamps. */
export function formatTime(input: string | number | Date): string {
  return new Intl.DateTimeFormat(getLocale(), {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  }).format(new Date(input));
}

/** Relative time ("in 3 minutes", "2 hours ago"). */
export function formatRelativeTime(seconds: number, unit: Intl.RelativeTimeFormatUnit): string {
  return new Intl.RelativeTimeFormat(getLocale(), { numeric: 'auto' }).format(seconds, unit);
}

/** "in 5 minutes" / "2 hours ago" for an ISO instant, picking the coarsest fitting unit; unparsable → `fallback`. */
export function formatRelativeFrom(iso: string, fallback: string, now = Date.now()): string {
  const t = new Date(iso).getTime();
  if (!Number.isFinite(t)) return fallback;
  const seconds = Math.round((t - now) / 1000);
  if (Math.abs(seconds) < 60) return formatRelativeTime(seconds, 'second');
  const minutes = Math.round(seconds / 60);
  if (Math.abs(minutes) < 60) return formatRelativeTime(minutes, 'minute');
  const hours = Math.round(minutes / 60);
  if (Math.abs(hours) < 24) return formatRelativeTime(hours, 'hour');
  return formatRelativeTime(Math.round(hours / 24), 'day');
}
