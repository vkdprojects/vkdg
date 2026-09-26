// Typed client for the VKDG admin API.
// In the embedded SPA, origin = same Rust server (port 9090).
// In dev (bun dev), configure VITE_ADMIN_URL in .env.development.

const BASE = import.meta.env.VITE_ADMIN_URL ?? '';

async function req<T>(method: string, path: string, body?: unknown): Promise<T> {
  const r = await fetch(`${BASE}${path}`, {
    method,
    headers: body ? { 'Content-Type': 'application/json' } : {},
    body: body ? JSON.stringify(body) : undefined,
    credentials: 'include',
  });
  if (!r.ok) {
    const err: unknown = await r.json().catch(() => null);
    const message = err && typeof err === 'object' && 'message' in err && typeof err.message === 'string'
      ? err.message
      : `HTTP ${r.status}`;
    throw new Error(message);
  }
  return r.json() as T;
}

// ── Types ──────────────────────────────────────────────────────────────────
export interface SystemInfo {
  version: string;
  status: string;
  config_revision: number;
  uptime_secs: number;
  connection_count: number;
  active_requests: number;
}

export interface SessionUser {
  user_id: string;
  role: 'viewer' | 'operator' | 'admin';
}

export interface ConnectionSummary {
  id: string;
  provider: string;
  status: string;
  model_count: number;
  active_requests: number;
}

export interface ClientKey {
  id: string;
  name: string;
  role: string;
  created_at: string;
  last_used_at?: string;
  scopes: string[];
}

export interface CreatedKey {
  key: string;
  id: string;
  name: string;
}

export interface RouteSummary {
  id: string;
  match_models: string[];
  strategy: string;
  targets: string[];
}

export interface RoutePreview {
  model: string;
  eligible_connections: string[];
  excluded_connections: { id: string; reason: string }[];
}

export interface RequestSummary {
  request_id: string;
  model: string;
  api_type?: string;
  status: string;
  connection_id?: string;
  started_at_ms: number;
  duration_ms?: number;
}

export interface ComboSummary {
  id: string;
  match_patterns: string[];
  strategy: string;
  targets: string[];
  has_compression: boolean;
  has_cache: boolean;
  has_budget: boolean;
}

// ── Endpoints ──────────────────────────────────────────────────────────────
export const api = {
  system: () =>
    req<SystemInfo>('GET', '/admin/v1/system'),
  me: () =>
    req<SessionUser>('GET', '/admin/v1/session/me'),
  login: (token: string) =>
    req<SessionUser>('POST', '/admin/v1/session', { token }),
  logout: () =>
    req<void>('DELETE', '/admin/v1/session'),
  listConnections: () =>
    req<{ items: ConnectionSummary[]; total: number }>('GET', '/admin/v1/connections'),
  listKeys: () =>
    req<{ items: ClientKey[]; total: number }>('GET', '/admin/v1/keys'),
  createKey: (name: string, role: string) =>
    req<CreatedKey>('POST', '/admin/v1/keys', { name, role }),
  revokeKey: (id: string) =>
    req<void>('DELETE', `/admin/v1/keys/${id}`),
  listRoutes: () =>
    req<{ items: RouteSummary[] }>('GET', '/admin/v1/routes'),
  previewRoute: (model: string) =>
    req<RoutePreview>('GET', `/admin/v1/routes/preview?model=${encodeURIComponent(model)}`),
  listRequests: (limit = 50) =>
    req<{ items: RequestSummary[]; has_more: boolean }>('GET', `/admin/v1/requests?limit=${limit}`),
  getRequest: (id: string) =>
    req<RequestSummary>('GET', `/admin/v1/requests/${id}`),
  listCombos: () =>
    req<{ items: ComboSummary[]; total: number }>('GET', '/admin/v1/combos'),
};
