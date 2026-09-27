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
  // DELETE endpoints answer 204 with no body.
  if (r.status === 204) return undefined as T;
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

export type ConnectionStatus = 'healthy' | 'degraded' | 'circuit_open' | 'cooldown' | 'unknown';

export interface ConnectionSummary {
  id: string;
  provider: string;
  /** `unknown` when the data plane is not running. */
  status: ConnectionStatus;
  model_count: number;
  active_requests: number;
  max_concurrent: number;
  /** RFC 3339; set while cooling down or while the circuit is open. */
  cooldown_until?: string;
  /** Set only while cooling down. */
  failure_count?: number;
}

export type KeyScope = 'data_inference' | 'data_image';

export interface ClientKey {
  id: string;
  name: string;
  tenant_id: string;
  /** Safe-to-show start of the key, e.g. `vkdg_1a2b3c4d`. */
  prefix: string;
  scopes: KeyScope[];
  created_at: string;
  last_used_at?: string | null;
  revoked_at?: string | null;
  expires_at?: string | null;
  allowed_models: string[];
  allowed_ips: string[];
  monthly_token_limit?: number | null;
  requests_per_minute?: number | null;
  /** Requests from this key are kept out of the request history. */
  no_log: boolean;
  /** Tokens and requests this calendar month (UTC). */
  usage_this_month?: { input_tokens: number; output_tokens: number; requests: number } | null;
  status: 'active' | 'disabled' | 'expired' | 'revoked';
}

/** Optional create-time limits; absent = unrestricted. */
export interface KeyLimits {
  /** RFC 3339 instant. */
  expires_at?: string;
  allowed_models?: string[];
  /** Addresses or CIDR ranges. */
  allowed_ips?: string[];
  monthly_token_limit?: number;
  requests_per_minute?: number;
  no_log?: boolean;
}

/**
 * PATCH body: an absent field is left as it is; `null` clears
 * `expires_at`, `monthly_token_limit` or `requests_per_minute`.
 */
export interface KeyPatch {
  name?: string;
  scopes?: KeyScope[];
  expires_at?: string | null;
  allowed_models?: string[];
  allowed_ips?: string[];
  monthly_token_limit?: number | null;
  requests_per_minute?: number | null;
  no_log?: boolean;
}

/** Returned once by createKey and regenerateKey; `key` is never shown again. */
export interface CreatedKey extends ClientKey {
  key: string;
}

export interface Account {
  id: string;
  provider: string;
  label: string;
  expires_at: string | null;
  has_refresh_token: boolean;
  status: 'active' | 'needs_login';
  revoked_reason?: string;
}

export type OAuthFlow = 'authorization_code_pkce' | 'device_code' | 'import_token';

export interface LoginField {
  id: string;
  label: string;
  required: boolean;
  secret: boolean;
  default: string | null;
}

export interface LoginMethod {
  id: string;
  label: string;
  flow: OAuthFlow;
  fields: LoginField[];
}

export type LoginStart =
  | {
      flow: 'device_code';
      login_id: string;
      user_code: string;
      verification_uri: string;
      verification_uri_complete: string | null;
      interval_secs: number;
      expires_in_secs: number;
    }
  | { flow: 'authorization_code_pkce'; login_id: string; authorize_url: string };

export type LoginPoll =
  | { status: 'pending' | 'slow_down' }
  | { status: 'failed'; message: string }
  | { status: 'done'; account: Account };

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

export interface RequestDecision {
  route_id: string | null;
  attempt_count: number;
  candidates_excluded: { id: string; reason: string }[];
}

export interface RequestSummary {
  request_id: string;
  model: string;
  api_type: string;
  /** `pending` while the body streams, then `completed`, `cancelled` or `failed`. */
  status: string;
  connection_id: string | null;
  started_at_ms: number;
  duration_ms: number | null;
  decision?: RequestDecision | null;
  /** Reported by the response; absent while streaming or when not reported. */
  input_tokens?: number | null;
  output_tokens?: number | null;
  /** Microdollars at the provider's list price; absent when it lists none. */
  cost_microdollars?: number | null;
}

export type RequestStatusFilter = 'all' | 'completed' | 'cancelled' | 'failed';

export interface ComboSummary {
  id: string;
  match_patterns: string[];
  strategy: string;
  targets: string[];
  has_compression: boolean;
  has_cache: boolean;
  has_budget: boolean;
}

export interface PluginSummary {
  name: string;
  version: string;
  kind: string;
  description: string;
  tags: string[];
  models: string[];
  removable: boolean;
}

// ── Endpoints ──────────────────────────────────────────────────────────────
export const api = {
  system: () =>
    req<SystemInfo>('GET', '/admin/v1/system'),
  me: () =>
    req<SessionUser>('GET', '/admin/v1/session/me'),
  /** First run: the bootstrap token. Once a password is set: the password. */
  login: (credential: { token: string } | { password: string }) =>
    req<SessionUser>('POST', '/admin/v1/session', credential),
  setupStatus: () =>
    req<{ password_set: boolean }>('GET', '/admin/v1/setup'),
  setPassword: (password: string) =>
    req<void>('POST', '/admin/v1/setup', { password }),
  logout: () =>
    req<void>('DELETE', '/admin/v1/session'),
  listConnections: () =>
    req<{ items: ConnectionSummary[]; total: number }>('GET', '/admin/v1/connections'),
  listKeys: () =>
    req<{ items: ClientKey[]; total: number }>('GET', '/admin/v1/keys'),
  createKey: (name: string, scopes: KeyScope[], limits: KeyLimits = {}) =>
    req<CreatedKey>('POST', '/admin/v1/keys', { name, scopes, ...limits }),
  updateKey: (id: string, patch: KeyPatch) =>
    req<ClientKey>('PATCH', `/admin/v1/keys/${encodeURIComponent(id)}`, patch),
  regenerateKey: (id: string) =>
    req<CreatedKey>('POST', `/admin/v1/keys/${encodeURIComponent(id)}/regenerate`),
  disableKey: (id: string) =>
    req<void>('POST', `/admin/v1/keys/${encodeURIComponent(id)}/disable`),
  enableKey: (id: string) =>
    req<void>('POST', `/admin/v1/keys/${encodeURIComponent(id)}/enable`),
  revokeKey: (id: string) =>
    req<void>('DELETE', `/admin/v1/keys/${encodeURIComponent(id)}`),
  listAccounts: () =>
    req<{ items: Account[]; total: number }>('GET', '/admin/v1/accounts'),
  deleteAccount: (id: string) =>
    req<void>('DELETE', `/admin/v1/accounts/${encodeURIComponent(id)}`),
  /** Providers on this gateway that support interactive login, plugins included. */
  oauthProviders: () =>
    req<{ items: { id: string; display_name: string }[] }>('GET', '/admin/v1/providers/oauth'),
  loginMethods: (provider: string) =>
    req<{ provider: string; items: LoginMethod[] }>('GET', `/admin/v1/providers/${encodeURIComponent(provider)}/login-methods`),
  /** `accountId` reconnects that account in place (same id) instead of adding one. */
  startLogin: (provider: string, method: string, params: Record<string, string> = {}, accountId?: string) =>
    req<LoginStart>('POST', `/admin/v1/oauth/${encodeURIComponent(provider)}/start`, { method, params, account_id: accountId }),
  pollLogin: (provider: string, login_id: string, code?: string) =>
    req<LoginPoll>('POST', `/admin/v1/oauth/${encodeURIComponent(provider)}/poll`, { login_id, code }),
  importToken: (provider: string, method: string, params: Record<string, string>, accountId?: string) =>
    req<{ status: 'done'; account: Account }>('POST', `/admin/v1/oauth/${encodeURIComponent(provider)}/import`, { method, params, account_id: accountId }),
  listRoutes: () =>
    req<{ items: RouteSummary[] }>('GET', '/admin/v1/routes'),
  previewRoute: (model: string) =>
    req<RoutePreview>('GET', `/admin/v1/routes/preview?model=${encodeURIComponent(model)}`),
  listRequests: (limit = 50, status: RequestStatusFilter = 'all') =>
    req<{ items: RequestSummary[]; has_more: boolean; cursor: string | null }>(
      'GET',
      `/admin/v1/requests?limit=${limit}${status === 'all' ? '' : `&status=${status}`}`,
    ),
  getRequest: (id: string) =>
    req<RequestSummary>('GET', `/admin/v1/requests/${encodeURIComponent(id)}`),
  listCombos: () =>
    req<{ items: ComboSummary[]; total: number }>('GET', '/admin/v1/combos'),
  listPlugins: () =>
    req<{ items: PluginSummary[]; total: number; directory: string }>('GET', '/admin/v1/plugins'),
  installPlugin: (manifest: string, wasmBase64?: string) =>
    req<{ name: string; version: string; directory: string }>('POST', '/admin/v1/plugins', {
      manifest,
      wasm_base64: wasmBase64,
    }),
  removePlugin: (name: string) =>
    req<void>('DELETE', `/admin/v1/plugins/${encodeURIComponent(name)}`),
};
