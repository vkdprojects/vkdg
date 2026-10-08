// Typed client for the VKDG admin API.
// All functions run server-side only (src/lib/server/).
// VKDG_ADMIN_URL env var — defaults to http://127.0.0.1:9090.

import type * as Api from '$lib/api.js';

const BASE = process.env.VKDG_ADMIN_URL ?? 'http://127.0.0.1:9090';

// Wire types live in `$lib/api.ts`; the BFF only re-exposes the subset it
// reads, so a contract change is made once. `Omit` drops fields the BFF's
// callers never populate.
export type SessionUser = Api.SessionUser;
export type KeyScope = Api.KeyScope;
export type KeyLimits = Omit<Api.KeyLimits, 'no_log'>;
export type KeyPatch = Omit<Api.KeyPatch, 'no_log'>;
export type RouteSummary = Api.RouteSummary;
export type RoutePreview = Api.RoutePreview;
export type RequestDecision = Api.RequestDecision;
export type RequestSummary = Pick<
  Api.RequestSummary,
  'request_id' | 'model' | 'api_type' | 'status' | 'connection_id' | 'started_at_ms' | 'duration_ms' | 'decision'
>;
export type SystemInfo = Omit<Api.SystemInfo, 'status'> & { status: 'ok' | 'degraded' };
export type ConnectionSummary = Omit<Api.ConnectionSummary, 'models' | 'weight' | 'account_id'>;
export type ClientKey = Omit<Api.ClientKey, 'no_log' | 'last_used_at' | 'revoked_at' | 'expires_at'> & {
  last_used_at: string | null;
  revoked_at: string | null;
  expires_at: string | null;
};
export type CreatedKey = ClientKey & { key: string };
export type ComboSummary = Pick<
  Api.ComboSummary,
  'id' | 'match_patterns' | 'targets' | 'has_compression' | 'has_cache' | 'has_budget'
> & { strategy: string };

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

// Keys
export async function listKeys(cookie: string): Promise<ClientKey[]> {
  const res = await fetch(`${BASE}/admin/v1/keys`, { headers: adminHeaders(cookie) });
  if (!res.ok) throw new Error(`keys fetch failed: ${res.status}`);
  const d = (await res.json()) as { items: ClientKey[] };
  return d.items;
}

export async function createKey(
  cookie: string,
  name: string,
  scopes: KeyScope[],
  limits: KeyLimits = {},
): Promise<CreatedKey> {
  const res = await fetch(`${BASE}/admin/v1/keys`, {
    method: 'POST',
    headers: adminHeaders(cookie),
    body: JSON.stringify({ name, scopes, ...limits }),
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

export async function updateKey(cookie: string, id: string, patch: KeyPatch): Promise<ClientKey> {
  const res = await fetch(`${BASE}/admin/v1/keys/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    headers: adminHeaders(cookie),
    body: JSON.stringify(patch),
  });
  if (!res.ok) throw new Error(`update key failed: ${res.status}`);
  return res.json() as Promise<ClientKey>;
}

export async function regenerateKey(cookie: string, id: string): Promise<CreatedKey> {
  const res = await fetch(`${BASE}/admin/v1/keys/${encodeURIComponent(id)}/regenerate`, {
    method: 'POST',
    headers: adminHeaders(cookie),
  });
  if (!res.ok) throw new Error(`regenerate key failed: ${res.status}`);
  return res.json() as Promise<CreatedKey>;
}

async function setKeyDisabled(cookie: string, id: string, disabled: boolean): Promise<void> {
  const action = disabled ? 'disable' : 'enable';
  const res = await fetch(`${BASE}/admin/v1/keys/${encodeURIComponent(id)}/${action}`, {
    method: 'POST',
    headers: adminHeaders(cookie),
  });
  if (!res.ok) throw new Error(`${action} key failed: ${res.status}`);
}

export const disableKey = (cookie: string, id: string) => setKeyDisabled(cookie, id, true);
export const enableKey = (cookie: string, id: string) => setKeyDisabled(cookie, id, false);

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
export async function listRequests(
  cookie: string,
  limit = 50,
  status?: 'completed' | 'failed',
): Promise<RequestSummary[]> {
  const q = status ? `&status=${status}` : '';
  const res = await fetch(`${BASE}/admin/v1/requests?limit=${limit}${q}`, {
    headers: adminHeaders(cookie),
  });
  if (!res.ok) throw new Error(`requests fetch failed: ${res.status}`);
  const d = (await res.json()) as { items: RequestSummary[] };
  return d.items;
}

export async function getRequest(cookie: string, id: string): Promise<RequestSummary> {
  const res = await fetch(`${BASE}/admin/v1/requests/${encodeURIComponent(id)}`, {
    headers: adminHeaders(cookie),
  });
  if (!res.ok) throw new Error(`request not found: ${res.status}`);
  return res.json() as Promise<RequestSummary>;
}

// Combos
export async function listCombos(cookie: string): Promise<ComboSummary[]> {
  const res = await fetch(`${BASE}/admin/v1/combos`, { headers: adminHeaders(cookie) });
  if (!res.ok) throw new Error(`combos fetch failed: ${res.status}`);
  const d = (await res.json()) as { items: ComboSummary[] };
  return d.items;
}
