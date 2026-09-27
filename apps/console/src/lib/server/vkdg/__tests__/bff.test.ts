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
  updateKey, regenerateKey, disableKey, enableKey,
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
      {
        id: 'anthropic-default', provider: 'anthropic', status: 'cooldown', model_count: 3,
        active_requests: 2, max_concurrent: 50,
        cooldown_until: '2026-09-27T12:00:30+00:00', failure_count: 3,
      },
    ];
    server.use(http.get(`${BASE}/admin/v1/connections`, () =>
      HttpResponse.json({ items, total: 1 })
    ));
    const result = await listConnections('session=tok');
    expect(result[0].id).toBe('anthropic-default');
    expect(result[0].status).toBe('cooldown');
    expect(result[0].failure_count).toBe(3);
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
      {
        id: 'key-1', name: 'ci-key', tenant_id: 'default', prefix: 'vkdg_1a2b3c4d',
        scopes: ['data_inference'], created_at: '2026-01-01T00:00:00Z',
        last_used_at: null, revoked_at: '2026-01-02T00:00:00Z', expires_at: null,
        allowed_models: [], allowed_ips: [], status: 'revoked',
      },
    ];
    server.use(http.get(`${BASE}/admin/v1/keys`, () =>
      HttpResponse.json({ items, total: 1 })
    ));
    const result = await listKeys('session=tok');
    expect(result[0].prefix).toBe('vkdg_1a2b3c4d');
    expect(result[0].status).toBe('revoked');
    expect(result[0]).not.toHaveProperty('key');
  });
});

describe('createKey', () => {
  // Plausible wrong impl: raw token not returned from CreatedKey response
  it('returns raw key value on 201', async () => {
    const created: CreatedKey = {
      key: 'vkdg_1a2b3c4dfull', id: 'key-uuid', name: 'test-key', tenant_id: 'default',
      prefix: 'vkdg_1a2b3c4d', scopes: ['data_inference'], created_at: '2026-01-01T00:00:00Z',
      last_used_at: null, revoked_at: null, expires_at: '2026-12-31T00:00:00+00:00',
      allowed_models: ['claude-*'], allowed_ips: ['10.0.0.0/8'], status: 'active',
    };
    let sent: unknown;
    server.use(http.post(`${BASE}/admin/v1/keys`, async ({ request }) => {
      sent = await request.json();
      return HttpResponse.json(created, { status: 201 });
    }));
    const result = await createKey('session=tok', 'test-key', ['data_inference']);
    expect(result.key).toBe('vkdg_1a2b3c4dfull');
    // The admin API takes scopes; a stale `role` field would be silently ignored.
    expect(sent).toEqual({ name: 'test-key', scopes: ['data_inference'] });
  });

  // Plausible wrong impl: limits dropped or nested instead of top-level fields the Rust body reads
  it('sends limits as top-level fields', async () => {
    let sent: unknown;
    server.use(http.post(`${BASE}/admin/v1/keys`, async ({ request }) => {
      sent = await request.json();
      return HttpResponse.json({}, { status: 201 });
    }));
    await createKey('session=tok', 'k', ['data_inference'], {
      expires_at: '2026-12-31T00:00:00.000Z', allowed_models: ['claude-*'],
      allowed_ips: ['10.0.0.0/8'], monthly_token_limit: 1000, requests_per_minute: 60,
    });
    expect(sent).toEqual({
      name: 'k', scopes: ['data_inference'], expires_at: '2026-12-31T00:00:00.000Z',
      allowed_models: ['claude-*'], allowed_ips: ['10.0.0.0/8'],
      monthly_token_limit: 1000, requests_per_minute: 60,
    });
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

describe('updateKey', () => {
  // Plausible wrong impl: `null` dropped by a "compact" step, so the limit is never cleared
  it('sends null to clear a limit and leaves absent fields out', async () => {
    let sent: unknown;
    server.use(http.patch(`${BASE}/admin/v1/keys/key-uuid`, async ({ request }) => {
      sent = await request.json();
      return HttpResponse.json({ id: 'key-uuid', name: 'renamed', status: 'active' });
    }));
    const result = await updateKey('session=tok', 'key-uuid', { name: 'renamed', monthly_token_limit: null });
    expect(sent).toEqual({ name: 'renamed', monthly_token_limit: null });
    expect(result.name).toBe('renamed');
  });

  // Plausible wrong impl: backend 400 (bad CIDR) treated as success
  it('throws on 400', async () => {
    server.use(http.patch(`${BASE}/admin/v1/keys/key-uuid`, () =>
      HttpResponse.json({ code: 'invalid_request', message: 'bad ip', request_id: 'x' }, { status: 400 })
    ));
    await expect(updateKey('session=tok', 'key-uuid', { allowed_ips: ['192.168.'] })).rejects.toThrow();
  });
});

describe('regenerateKey', () => {
  // Plausible wrong impl: flattened summary not merged, or raw key missing
  it('returns the new raw key with the flattened summary', async () => {
    server.use(http.post(`${BASE}/admin/v1/keys/key-uuid/regenerate`, () =>
      HttpResponse.json({
        key: 'vkdg_9f8e7d6cnew', id: 'key-uuid', name: 'ci', tenant_id: 'default',
        prefix: 'vkdg_9f8e7d6c', scopes: ['data_inference'], created_at: '2026-01-01T00:00:00+00:00',
        last_used_at: null, revoked_at: null, expires_at: null, allowed_models: [], allowed_ips: [],
        monthly_token_limit: null, requests_per_minute: null, usage_this_month: null, status: 'active',
      })
    ));
    const result = await regenerateKey('session=tok', 'key-uuid');
    expect(result.key).toBe('vkdg_9f8e7d6cnew');
    expect(result.id).toBe('key-uuid');
  });
});

describe('disableKey / enableKey', () => {
  // Plausible wrong impl: both hit the same path, or a 404 is swallowed
  it('hits the matching action path and throws on 404', async () => {
    const hits: string[] = [];
    server.use(
      http.post(`${BASE}/admin/v1/keys/key-uuid/disable`, () => { hits.push('disable'); return new HttpResponse(null, { status: 204 }); }),
      http.post(`${BASE}/admin/v1/keys/key-uuid/enable`, () => { hits.push('enable'); return new HttpResponse(null, { status: 204 }); }),
      http.post(`${BASE}/admin/v1/keys/gone/disable`, () =>
        HttpResponse.json({ code: 'not_found', message: 'not found', request_id: 'x' }, { status: 404 })),
    );
    await disableKey('session=tok', 'key-uuid');
    await enableKey('session=tok', 'key-uuid');
    expect(hits).toEqual(['disable', 'enable']);
    await expect(disableKey('session=tok', 'gone')).rejects.toThrow();
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
    let status: string | null = 'unset';
    server.use(http.get(`${BASE}/admin/v1/requests`, ({ request }) => {
      status = new URL(request.url).searchParams.get('status');
      return HttpResponse.json({
        items: [
          {
            request_id: 'req-1', model: 'claude-3-5-haiku-20241022',
            api_type: 'anthropic', status: 'failed',
            connection_id: 'anthropic-default',
            started_at_ms: 1700000000000, duration_ms: 342, decision: null,
          },
        ],
        has_more: false,
        cursor: null,
      });
    }));
    const result = await listRequests('session=tok', 50, 'failed');
    expect(status).toBe('failed');
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
        request_id: 'req-abc', model: 'claude-opus-4-5', api_type: 'anthropic',
        status: 'completed', connection_id: 'anthropic-default',
        started_at_ms: 1700000001000, duration_ms: 1200,
        decision: {
          route_id: 'default', attempt_count: 2,
          candidates_excluded: [{ id: 'openai-default', reason: 'circuit open' }],
        },
      })
    ));
    const result = await getRequest('session=tok', 'req-abc');
    expect(result.request_id).toBe('req-abc');
    expect(result.decision?.candidates_excluded[0].reason).toBe('circuit open');
  });
});
