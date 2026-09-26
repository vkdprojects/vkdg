// BFF contract tests: exercises the typed admin client against a mock HTTP server.
// Each test documents the plausible wrong implementation it defeats.
//
// Day 0 doc (section 8) requires: action/load BFF tests with a fake server covering
// status, body, cookie, revision, and backend loss.

import { describe, it, expect, beforeAll, afterAll, afterEach } from 'vitest';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import {
  getSystem, login, logout, getMe, listConnections, listKeys, createKey, revokeKey,
  listRoutes, previewRoute, listRequests, getRequest,
  type SystemInfo, type SessionUser, type ConnectionSummary, type ClientKey,
  type CreatedKey, type RouteSummary,
} from '../client';

// Mock server bound at the default VKDG_ADMIN_URL
const BASE = 'http://127.0.0.1:9090';

const server = setupServer();

beforeAll(() => server.listen({ onUnhandledRequest: 'error' }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());

// ── /admin/v1/system ─────────────────────────────────────────────────────────

describe('getSystem', () => {
  // Plausible wrong impl: getSystem returns undefined on success (missing return)
  it('returns SystemInfo on 200', async () => {
    const mock: SystemInfo = {
      version: '0.1.0-rc1', status: 'ok', config_revision: 3,
      uptime_secs: 120, connection_count: 2, active_requests: 1,
    };
    server.use(http.get(`${BASE}/admin/v1/system`, () => HttpResponse.json(mock)));
    const result = await getSystem();
    expect(result.version).toBe('0.1.0-rc1');
    expect(result.status).toBe('ok');
    expect(result.connection_count).toBe(2);
  });

  // Plausible wrong impl: non-2xx not detected, returns garbled SystemInfo
  it('throws on backend loss (503)', async () => {
    server.use(http.get(`${BASE}/admin/v1/system`, () =>
      HttpResponse.json({}, { status: 503 })
    ));
    await expect(getSystem()).rejects.toThrow();
  });
});

// ── /admin/v1/session ─────────────────────────────────────────────────────────

describe('login', () => {
  // Plausible wrong impl: login succeeds but setCookie is null (header not read)
  it('returns user and Set-Cookie on 200', async () => {
    const user: SessionUser = { user_id: 'admin', role: 'admin' };
    server.use(http.post(`${BASE}/admin/v1/session`, () =>
      HttpResponse.json(user, {
        headers: { 'set-cookie': 'vkdg_session=token123; HttpOnly; SameSite=Lax; Path=/' },
      })
    ));
    const result = await login('bootstrap-token');
    expect(result.user.role).toBe('admin');
    expect(result.setCookie).toContain('vkdg_session=token123');
  });

  // Plausible wrong impl: 401 silently returns empty user instead of throwing
  it('throws with message on 401', async () => {
    server.use(http.post(`${BASE}/admin/v1/session`, () =>
      HttpResponse.json(
        { code: 'invalid_token', message: 'bad token', request_id: 'x' },
        { status: 401 }
      )
    ));
    await expect(login('wrong')).rejects.toThrow('bad token');
  });
});

describe('logout', () => {
  // Plausible wrong impl: logout throws on 204 (no body) instead of resolving void
  it('resolves void on 204', async () => {
    server.use(http.delete(`${BASE}/admin/v1/session`, () =>
      new HttpResponse(null, { status: 204 })
    ));
    await expect(logout('session=tok')).resolves.toBeUndefined();
  });
});

describe('getMe', () => {
  // Plausible wrong impl: throws on valid session instead of returning user
  it('returns SessionUser on 200', async () => {
    const user: SessionUser = { user_id: 'op1', role: 'operator' };
    server.use(http.get(`${BASE}/admin/v1/session/me`, () => HttpResponse.json(user)));
    const result = await getMe('session=tok');
    expect(result.user_id).toBe('op1');
    expect(result.role).toBe('operator');
  });

  // Plausible wrong impl: 401 returns null instead of throwing
  it('throws on 401', async () => {
    server.use(http.get(`${BASE}/admin/v1/session/me`, () =>
      HttpResponse.json({}, { status: 401 })
    ));
    await expect(getMe('session=bad')).rejects.toThrow();
  });
});

// ── /admin/v1/connections ────────────────────────────────────────────────────

describe('listConnections', () => {
  // Plausible wrong impl: returns undefined instead of empty array
  it('returns empty array when no connections', async () => {
    server.use(http.get(`${BASE}/admin/v1/connections`, () =>
      HttpResponse.json({ items: [], total: 0 })
    ));
    const result = await listConnections('session=tok');
    expect(result).toHaveLength(0);
  });

  // Plausible wrong impl: items not extracted from {items, total} wrapper
  it('returns connection list', async () => {
    const items: ConnectionSummary[] = [
      { id: 'anthropic-default', provider: 'anthropic', status: 'healthy', model_count: 3, active_requests: 0 },
    ];
    server.use(http.get(`${BASE}/admin/v1/connections`, () =>
      HttpResponse.json({ items, total: 1 })
    ));
    const result = await listConnections('session=tok');
    expect(result[0].id).toBe('anthropic-default');
    expect(result[0].status).toBe('healthy');
  });

  // Plausible wrong impl: backend loss returns [] instead of throwing
  it('throws on 503', async () => {
    server.use(http.get(`${BASE}/admin/v1/connections`, () =>
      HttpResponse.json({}, { status: 503 })
    ));
    await expect(listConnections('session=tok')).rejects.toThrow();
  });
});

// ── /admin/v1/keys ────────────────────────────────────────────────────────────

describe('listKeys', () => {
  // Plausible wrong impl: items wrapper not unwrapped
  it('returns key list', async () => {
    const items: ClientKey[] = [
      { id: 'key-1', name: 'ci-key', role: 'viewer', created_at: '2026-01-01T00:00:00Z', last_used_at: null, scopes: [] },
    ];
    server.use(http.get(`${BASE}/admin/v1/keys`, () =>
      HttpResponse.json({ items })
    ));
    const result = await listKeys('session=tok');
    expect(result[0].name).toBe('ci-key');
    expect(result[0].role).toBe('viewer');
  });
});

describe('createKey', () => {
  // Plausible wrong impl: raw token not returned from CreatedKey response
  it('returns raw key value on 201', async () => {
    const created: CreatedKey = { key: 'sk-raw-token-abc123', id: 'key-uuid', name: 'test-key' };
    server.use(http.post(`${BASE}/admin/v1/keys`, () =>
      HttpResponse.json(created, { status: 201 })
    ));
    const result = await createKey('session=tok', 'test-key', 'viewer');
    expect(result.key).toBe('sk-raw-token-abc123');
    expect(result.key.length).toBeGreaterThan(0);
  });
});

describe('revokeKey', () => {
  // Plausible wrong impl: throws on successful 204 response
  it('resolves void on 204', async () => {
    server.use(http.delete(`${BASE}/admin/v1/keys/key-uuid`, () =>
      new HttpResponse(null, { status: 204 })
    ));
    await expect(revokeKey('session=tok', 'key-uuid')).resolves.toBeUndefined();
  });
});

// ── /admin/v1/routes ─────────────────────────────────────────────────────────

describe('listRoutes', () => {
  // Plausible wrong impl: items wrapper not unwrapped
  it('returns route list', async () => {
    const items: RouteSummary[] = [
      { id: 'default', match_models: ['claude-*'], strategy: 'priority', targets: ['anthropic-default'] },
    ];
    server.use(http.get(`${BASE}/admin/v1/routes`, () =>
      HttpResponse.json({ items })
    ));
    const result = await listRoutes('session=tok');
    expect(result[0].id).toBe('default');
    expect(result[0].strategy).toBe('priority');
  });
});

describe('previewRoute', () => {
  // Plausible wrong impl: model not URL-encoded, spaces break the URL
  it('returns eligible connections for a model', async () => {
    server.use(http.get(`${BASE}/admin/v1/routes/preview`, ({ request }) => {
      const url = new URL(request.url);
      expect(url.searchParams.get('model')).toBe('claude-3-5-haiku-20241022');
      return HttpResponse.json({
        model: 'claude-3-5-haiku-20241022',
        eligible_connections: ['anthropic-default'],
        excluded_connections: [],
      });
    }));
    const result = await previewRoute('session=tok', 'claude-3-5-haiku-20241022');
    expect(result.eligible_connections).toHaveLength(1);
    expect(result.eligible_connections[0]).toBe('anthropic-default');
  });

  // Plausible wrong impl: backend unavailable returns undefined instead of throwing
  it('throws on 401 unauthorized', async () => {
    server.use(http.get(`${BASE}/admin/v1/routes/preview`, () =>
      HttpResponse.json({ code: 'unauthorized', message: 'no session', request_id: 'x' }, { status: 401 })
    ));
    await expect(previewRoute('', 'claude-3-5-haiku')).rejects.toThrow();
  });
});

// ── /admin/v1/requests ───────────────────────────────────────────────────────

describe('listRequests', () => {
  // Plausible wrong impl: items wrapper not unwrapped, or has_more ignored
  it('returns request list', async () => {
    server.use(http.get(`${BASE}/admin/v1/requests`, () =>
      HttpResponse.json({
        items: [
          {
            request_id: 'req-1', model: 'claude-3-5-haiku-20241022',
            api_type: 'messages', status: 'success',
            connection_id: 'anthropic-default',
            started_at_ms: 1700000000000, duration_ms: 342,
          },
        ],
        has_more: false,
        cursor: null,
      })
    ));
    const result = await listRequests('session=tok');
    expect(result[0].request_id).toBe('req-1');
    expect(result[0].duration_ms).toBe(342);
  });
});

describe('getRequest', () => {
  // Plausible wrong impl: 404 not detected, returns garbled object
  it('throws on 404', async () => {
    server.use(http.get(`${BASE}/admin/v1/requests/missing-id`, () =>
      HttpResponse.json({ code: 'not_found', message: 'not found', request_id: 'x' }, { status: 404 })
    ));
    await expect(getRequest('session=tok', 'missing-id')).rejects.toThrow();
  });

  it('returns RequestSummary on 200', async () => {
    server.use(http.get(`${BASE}/admin/v1/requests/req-abc`, () =>
      HttpResponse.json({
        request_id: 'req-abc', model: 'claude-opus-4-5', api_type: 'messages',
        status: 'success', connection_id: 'anthropic-default',
        started_at_ms: 1700000001000, duration_ms: 1200,
      })
    ));
    const result = await getRequest('session=tok', 'req-abc');
    expect(result.request_id).toBe('req-abc');
    expect(result.model).toBe('claude-opus-4-5');
  });
});
