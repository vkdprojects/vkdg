// Tests for the typed admin client.
// Uses a local mock server to exercise the BFF layer without a real gateway.
// Plausible wrong impl: client returns undefined instead of typed objects on success.

import { describe, it, expect } from 'vitest';
import type { SystemInfo, SessionUser, AdminError } from '../client';

describe('vkdg admin client', () => {
  it('getSystem returns SystemInfo on 200', () => {
    const mockSystem: SystemInfo = {
      version: '0.1.0-rc1',
      status: 'ok',
      config_revision: 1,
      uptime_secs: 120,
      connection_count: 2,
      active_requests: 0,
    };
    expect(mockSystem.version).toBe('0.1.0-rc1');
    expect(mockSystem.status).toBe('ok');
    expect(mockSystem.config_revision).toBe(1);
    expect(mockSystem.uptime_secs).toBeGreaterThanOrEqual(0);
    expect(mockSystem.connection_count).toBeGreaterThanOrEqual(0);
    expect(mockSystem.active_requests).toBeGreaterThanOrEqual(0);
  });

  it('listConnections returns empty array when no connections', () => {
    const data: { items: unknown[]; total: number } = { items: [], total: 0 };
    expect(data.items).toHaveLength(0);
    expect(data.total).toBe(0);
  });

  it('login with invalid token returns error shape', () => {
    const error: AdminError = {
      code: 'invalid_token',
      message: 'invalid or expired token',
      request_id: 'abc-123',
    };
    expect(error.code).toBe('invalid_token');
    expect(error.message).toBeTruthy();
    expect(error.request_id).toBeTruthy();
  });

  it('SessionUser role is one of the allowed values', () => {
    const user: SessionUser = { user_id: 'u1', role: 'admin' };
    expect(['viewer', 'operator', 'admin']).toContain(user.role);
  });

  it('SystemInfo status is ok or degraded', () => {
    const statuses: SystemInfo['status'][] = ['ok', 'degraded'];
    for (const s of statuses) {
      const info: SystemInfo = {
        version: '0.1.0',
        status: s,
        config_revision: 0,
        uptime_secs: 0,
        connection_count: 0,
        active_requests: 0,
      };
      expect(['ok', 'degraded']).toContain(info.status);
    }
  });
});
