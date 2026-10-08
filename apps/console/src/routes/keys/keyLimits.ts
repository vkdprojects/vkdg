import type { ClientKey } from '$lib/api.js';
import { m } from '$lib/paraglide/messages.js';
import { formatRelativeTime } from '$lib/format.js';

export const splitList = (v: string) => v.split(/[\n,]/).map((x) => x.trim()).filter(Boolean);

/** Local `YYYY-MM-DD` for a date input, matching how create sets end-of-day. */
export function toDateInput(iso: string | null | undefined): string {
  if (!iso) return '';
  const d = new Date(iso);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

/** Key lifecycle statuses (a different domain from connection/request statuses). */
export const keyStatusLabels: Record<ClientKey['status'], () => string> = {
  active: m.key_status_active,
  disabled: m.key_status_disabled,
  expired: m.key_status_expired,
  revoked: m.key_status_revoked,
};

export function humanizeDate(iso: string): string {
  const delta = new Date(iso).getTime() - Date.now();
  const abs = Math.abs(delta);
  if (abs < 60 * 60 * 1000) return formatRelativeTime(Math.round(delta / (60 * 1000)), 'minute');
  if (abs < 24 * 60 * 60 * 1000) return formatRelativeTime(Math.round(delta / (60 * 60 * 1000)), 'hour');
  if (abs < 30 * 24 * 60 * 60 * 1000) return formatRelativeTime(Math.round(delta / (24 * 60 * 60 * 1000)), 'day');
  if (abs < 365 * 24 * 60 * 60 * 1000) return formatRelativeTime(Math.round(delta / (30 * 24 * 60 * 60 * 1000)), 'month');
  return formatRelativeTime(Math.round(delta / (365 * 24 * 60 * 60 * 1000)), 'year');
}
