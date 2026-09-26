// Typed client for the VKDG admin API.
// All functions run server-side only (src/lib/server/).
// VKDG_ADMIN_URL env var — defaults to http://127.0.0.1:9090.

const BASE = process.env.VKDG_ADMIN_URL ?? 'http://127.0.0.1:9090';

export interface SystemInfo {
  version: string;
  status: 'ok' | 'degraded';
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
  status: 'healthy' | 'degraded' | 'circuit_open' | 'cooldown';
  model_count: number;
  active_requests: number;
}

export interface AdminError {
  code: string;
  message: string;
  request_id: string;
  details?: unknown;
}

function adminHeaders(cookie?: string): Record<string, string> {
  const h: Record<string, string> = { 'Content-Type': 'application/json' };
  if (cookie) h['Cookie'] = cookie;
  return h;
}

export async function getSystem(): Promise<SystemInfo> {
  const res = await fetch(`${BASE}/admin/v1/system`);
  if (!res.ok) throw new Error(`system fetch failed: ${res.status}`);
  return res.json() as Promise<SystemInfo>;
}

export async function login(
  token: string
): Promise<{ user: SessionUser; setCookie: string | null }> {
  const res = await fetch(`${BASE}/admin/v1/session`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ token }),
  });
  if (!res.ok) {
    const err: AdminError = (await res.json()) as AdminError;
    throw new Error(err.message);
  }
  const user = (await res.json()) as SessionUser;
  const setCookie = res.headers.get('set-cookie');
  return { user, setCookie };
}

export async function logout(cookie: string): Promise<void> {
  await fetch(`${BASE}/admin/v1/session`, {
    method: 'DELETE',
    headers: adminHeaders(cookie),
  });
}

export async function getMe(cookie: string): Promise<SessionUser> {
  const res = await fetch(`${BASE}/admin/v1/session/me`, {
    headers: adminHeaders(cookie),
  });
  if (!res.ok) throw new Error('unauthorized');
  return res.json() as Promise<SessionUser>;
}

export async function listConnections(cookie: string): Promise<ConnectionSummary[]> {
  const res = await fetch(`${BASE}/admin/v1/connections`, {
    headers: adminHeaders(cookie),
  });
  if (!res.ok) throw new Error(`connections fetch failed: ${res.status}`);
  const data = (await res.json()) as { items: ConnectionSummary[] };
  return data.items;
}

export async function getConnection(
  id: string,
  cookie: string
): Promise<ConnectionSummary> {
  const res = await fetch(`${BASE}/admin/v1/connections/${encodeURIComponent(id)}`, {
    headers: adminHeaders(cookie),
  });
  if (!res.ok) throw new Error(`connection fetch failed: ${res.status}`);
  return res.json() as Promise<ConnectionSummary>;
}

export interface ClientKey {
  id: string;
  name: string;
  role: string;
  created_at: string;
  last_used_at: string | null;
  scopes: string[];
}
export interface CreatedKey { key: string; id: string; name: string }
export interface RouteSummary { id: string; match_models: string[]; strategy: string; targets: string[] }
export interface RoutePreview {
  model: string;
  eligible_connections: string[];
  excluded_connections: { id: string; reason: string }[];
}
export interface RequestSummary {
  request_id: string;
  model: string;
  api_type: string;
  status: string;
  connection_id: string | null;
  started_at_ms: number;
  duration_ms: number | null;
}
export interface RequestList { items: RequestSummary[]; has_more: boolean; cursor: string | null }

// Keys
export async function listKeys(cookie: string): Promise<ClientKey[]> {
  const res = await fetch(`${BASE}/admin/v1/keys`, { headers: adminHeaders(cookie) });
  if (!res.ok) throw new Error(`keys fetch failed: ${res.status}`);
  const d = (await res.json()) as { items: ClientKey[] };
  return d.items;
}

export async function createKey(cookie: string, name: string, role: string): Promise<CreatedKey> {
  const res = await fetch(`${BASE}/admin/v1/keys`, {
    method: 'POST',
    headers: adminHeaders(cookie),
    body: JSON.stringify({ name, role }),
  });
  if (!res.ok) throw new Error(`create key failed: ${res.status}`);
  return res.json() as Promise<CreatedKey>;
}

export async function revokeKey(cookie: string, id: string): Promise<void> {
  await fetch(`${BASE}/admin/v1/keys/${encodeURIComponent(id)}`, {
    method: 'DELETE',
    headers: adminHeaders(cookie),
  });
}

// Routes
export async function listRoutes(cookie: string): Promise<RouteSummary[]> {
  const res = await fetch(`${BASE}/admin/v1/routes`, { headers: adminHeaders(cookie) });
  if (!res.ok) throw new Error(`routes fetch failed: ${res.status}`);
  const d = (await res.json()) as { items: RouteSummary[] };
  return d.items;
}

export async function previewRoute(cookie: string, model: string): Promise<RoutePreview> {
  const res = await fetch(
    `${BASE}/admin/v1/routes/preview?model=${encodeURIComponent(model)}`,
    { headers: adminHeaders(cookie) },
  );
  if (!res.ok) throw new Error(`preview failed: ${res.status}`);
  return res.json() as Promise<RoutePreview>;
}

// Requests
export async function listRequests(cookie: string, limit = 50): Promise<RequestSummary[]> {
  const res = await fetch(`${BASE}/admin/v1/requests?limit=${limit}`, {
    headers: adminHeaders(cookie),
  });
  if (!res.ok) throw new Error(`requests fetch failed: ${res.status}`);
  const d = (await res.json()) as RequestList;
  return d.items;
}

export async function getRequest(cookie: string, id: string): Promise<RequestSummary> {
  const res = await fetch(`${BASE}/admin/v1/requests/${encodeURIComponent(id)}`, {
    headers: adminHeaders(cookie),
  });
  if (!res.ok) throw new Error(`request not found: ${res.status}`);
  return res.json() as Promise<RequestSummary>;
}
