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
