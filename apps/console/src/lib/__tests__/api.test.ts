// Browser admin client (src/lib/api.ts) against a mock HTTP server.
// Covers the account-routing endpoints; each test names the wrong implementation it defeats.

import { describe, it, expect, beforeAll, afterAll, afterEach } from 'vitest';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { api } from '../api';
import type { ConnectionSummary } from '../api';

// vitest.config.ts points VITE_ADMIN_URL here; api.ts reads it once at import time.
const BASE = 'http://admin.test';

const server = setupServer();

beforeAll(() => server.listen({ onUnhandledRequest: 'error' }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());

const summary: ConnectionSummary = {
  id: 'claude-main',
  provider: 'claude-code',
  status: 'healthy',
  model_count: 2,
  models: ['claude-*', 'haiku-*'],
  weight: 3,
  active_requests: 0,
  max_concurrent: 8,
  account_id: 'acct-1',
};

describe('patchConnection', () => {
  // Wrong impl: PUT/POST instead of PATCH, or the patch is not serialised as the JSON body.
  it('PATCHes the encoded id with only the given fields and returns the updated summary', async () => {
    let seen: { method: string; body: unknown; contentType: string | null } | undefined;
    server.use(
      http.patch(`${BASE}/admin/v1/connections/:id`, async ({ request, params }) => {
        seen = {
          method: request.method,
          body: await request.json(),
          contentType: request.headers.get('content-type'),
        };
        expect(params['id']).toBe('team/claude main');
        return HttpResponse.json({ ...summary, weight: 5 });
      }),
    );

    const result = await api.patchConnection('team/claude main', { weight: 5, models: ['claude-*'] });

    expect(seen).toEqual({
      method: 'PATCH',
      body: { weight: 5, models: ['claude-*'] },
      contentType: 'application/json',
    });
    expect(result.weight).toBe(5);
    expect(result.models).toEqual(['claude-*', 'haiku-*']);
  });

  // Wrong impl: swallows the admin error shape and reports a bare "HTTP 422".
  it('surfaces the server validation message', async () => {
    server.use(
      http.patch(`${BASE}/admin/v1/connections/:id`, () =>
        HttpResponse.json(
          { code: 'validation_error', message: 'models must not be empty', request_id: 'r1' },
          { status: 422 },
        ),
      ),
    );

    await expect(api.patchConnection('claude-main', { models: [] })).rejects.toThrow('models must not be empty');
  });
});

describe('deleteConnection', () => {
  // Wrong impl: wrong verb, unencoded id, or the 204 body-less answer parsed as JSON.
  it('deletes the encoded id and resolves on 204', async () => {
    let seen: { method: string; id: unknown } | undefined;
    server.use(
      http.delete(`${BASE}/admin/v1/connections/:id`, ({ request, params }) => {
        seen = { method: request.method, id: params['id'] };
        return new HttpResponse(null, { status: 204 });
      }),
    );

    await expect(api.deleteConnection('claude code/1')).resolves.toBeUndefined();
    expect(seen).toEqual({ method: 'DELETE', id: 'claude code/1' });
  });

  // Wrong impl: a store failure is swallowed and the row disappears from the UI only to come back.
  it('surfaces the server error', async () => {
    server.use(
      http.delete(`${BASE}/admin/v1/connections/:id`, () =>
        HttpResponse.json({ code: 'store_error', message: 'disk full', request_id: 'r3' }, { status: 500 }),
      ),
    );

    await expect(api.deleteConnection('c1')).rejects.toThrow('disk full');
  });
});

describe('syncConnectionModels', () => {
  // Wrong impl: GET/PUT instead of POST, unencoded id, a body sent although the endpoint takes none,
  // or the { models, count } answer dropped.
  it('POSTs without a body to the encoded models/sync path and returns the parsed answer', async () => {
    let seen: { method: string; text: string; id: unknown } | undefined;
    server.use(
      http.post(`${BASE}/admin/v1/connections/:id/models/sync`, async ({ request, params }) => {
        seen = { method: request.method, text: await request.text(), id: params['id'] };
        return HttpResponse.json({ models: ['claude-sonnet-4-6', 'claude-haiku-4-5'], count: 2 });
      }),
    );

    const result = await api.syncConnectionModels('claude code/1');

    expect(seen).toEqual({ method: 'POST', text: '', id: 'claude code/1' });
    expect(result).toEqual({ models: ['claude-sonnet-4-6', 'claude-haiku-4-5'], count: 2 });
  });

  // Wrong impl: swallows the admin error shape and reports a bare "HTTP 502".
  it('surfaces the server message when discovery fails', async () => {
    server.use(
      http.post(`${BASE}/admin/v1/connections/:id/models/sync`, () =>
        HttpResponse.json(
          { code: 'model_sync_failed', message: 'provider returned no models', request_id: 'r4' },
          { status: 502 },
        ),
      ),
    );

    await expect(api.syncConnectionModels('claude-main')).rejects.toThrow('provider returned no models');
  });
});

describe('enableAccount', () => {
  // Wrong impl: wrong verb/path, or a body sent although the endpoint takes none.
  it('POSTs without a body to the account connection endpoint and returns the connection id', async () => {
    let seen: { method: string; text: string } | undefined;
    server.use(
      http.post(`${BASE}/admin/v1/accounts/:id/connection`, async ({ request, params }) => {
        seen = { method: request.method, text: await request.text() };
        expect(params['id']).toBe('acct 1');
        return HttpResponse.json({ connection_id: 'claude-acct-1' }, { status: 201 });
      }),
    );

    const result = await api.enableAccount('acct 1');

    expect(seen).toEqual({ method: 'POST', text: '' });
    expect(result).toEqual({ connection_id: 'claude-acct-1' });
  });

  // Wrong impl: 200 "already existed" treated as failure, or 422 not thrown.
  it('accepts the 200 of an existing connection', async () => {
    server.use(
      http.post(`${BASE}/admin/v1/accounts/:id/connection`, () =>
        HttpResponse.json({ connection_id: 'claude-acct-1' }, { status: 200 }),
      ),
    );

    await expect(api.enableAccount('acct-1')).resolves.toEqual({ connection_id: 'claude-acct-1' });
  });

  it('surfaces the server message when the provider cannot be routed yet', async () => {
    server.use(
      http.post(`${BASE}/admin/v1/accounts/:id/connection`, () =>
        HttpResponse.json(
          { code: 'no_default_models', message: 'antigravity has no default models yet', request_id: 'r2' },
          { status: 422 },
        ),
      ),
    );

    await expect(api.enableAccount('acct-2')).rejects.toThrow('antigravity has no default models yet');
  });
});
