/**
 * Status vocabulary shared by every screen: one label per status, one place to
 * add a new one. Unknown values fall back instead of throwing so a newer
 * gateway never breaks an older console.
 */
import { m } from '$lib/paraglide/messages.js';
import type { ConnectionStatus } from '$lib/api.js';

const connection: Record<ConnectionStatus, () => string> = {
  healthy: m.connection_status_healthy,
  degraded: m.connection_status_degraded,
  circuit_open: m.connection_status_circuit_open,
  cooldown: m.connection_status_cooldown,
  unknown: m.connection_status_unknown,
};

const request: Record<string, () => string> = {
  completed: m.request_status_completed,
  failed: m.request_status_failed,
  partial: m.request_status_partial,
  cancelled: m.request_status_cancelled,
  pending: m.request_status_pending,
};

export function connectionStatusLabel(status: string): string {
  return (connection[status as ConnectionStatus] ?? m.connection_status_unknown)();
}

export function requestStatusLabel(status: string): string {
  return request[status]?.() ?? status;
}

/** Connection is not taking traffic right now. */
export function isCooling(c: { status: ConnectionStatus; cooldown_until?: string }): boolean {
  return c.status === 'cooldown' || c.status === 'circuit_open' || !!c.cooldown_until;
}

/** Seconds until an ISO instant, clamped at 0. Pass `now` to tick reactively. */
export function secondsUntil(iso: string, now = Date.now()): number {
  return Math.max(0, Math.round((new Date(iso).getTime() - now) / 1000));
}

/** "1h 02m", "4m 05s", "12s": compact countdown. */
export function formatCountdown(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const mm = Math.floor((seconds % 3600) / 60);
  const ss = seconds % 60;
  if (h) return `${h}h ${String(mm).padStart(2, '0')}m`;
  if (mm) return `${mm}m ${String(ss).padStart(2, '0')}s`;
  return `${ss}s`;
}

/** Request latency: "840ms", "1.84s"; `null` (still streaming) → pending label. */
export function formatDuration(ms: number | null | undefined): string {
  if (ms == null) return m.common_pending();
  return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(2)}s`;
}
