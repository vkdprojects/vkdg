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
