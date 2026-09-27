import { expect, test, type Page } from '@playwright/test';
import { DATA_URL } from './env';

// Combos, end to end: a combo created in the console must change routing on
// the data plane at once, and deleting it must hand its traffic back. c1 points
// at a closed port, so the call itself fails; the request history row shows
// which route and connection the gateway picked.

async function lastRowFor(page: Page, model: string) {
  const res = await page.request.get('/admin/v1/requests?limit=20');
  const items: { model: string; connection_id: string | null; decision?: { route_id: string | null } | null }[] =
    (await res.json()).items;
  return items.find((r) => r.model === model);
}

test('a combo made in the console routes the next request, and deleting it stops that', async ({ page }) => {
  page.on('dialog', (d) => d.accept());
  const key = (await (await page.request.post('/admin/v1/keys', { data: { name: 'combo-probe' } })).json()).key;
  const call = () =>
    page.request.post(`${DATA_URL}/v1/messages`, {
      headers: { 'x-api-key': key, 'content-type': 'application/json' },
      data: { model: 'e2e-fast', max_tokens: 8, messages: [{ role: 'user', content: 'hi' }] },
    });

  await page.goto('/combos');
  await page.getByRole('button', { name: 'Create combo' }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('Name / ID').fill('e2e-fast');
  await dialog.getByRole('checkbox', { name: /c1/ }).check();
  await dialog.getByLabel('Upstream model').fill('claude-sonnet-4-5');
  const [saved] = await Promise.all([
    page.waitForResponse((r) => r.url().endsWith('/admin/v1/combos') && r.request().method() === 'POST'),
    dialog.getByRole('button', { name: 'Create combo' }).click(),
  ]);
  expect(saved.status()).toBe(201);
  await expect(page.getByRole('row').filter({ hasText: 'e2e-fast' })).toContainText('claude-sonnet-4-5');

  await call();
  await expect
    .poll(async () => (await lastRowFor(page, 'e2e-fast'))?.decision?.route_id)
    .toBe('combo:e2e-fast');
  expect((await lastRowFor(page, 'e2e-fast'))?.connection_id).toBe('c1');

  // Survives a reload of the page: it came from the gateway, not local state.
  await page.reload();
  const row = page.getByRole('row').filter({ hasText: 'e2e-fast' });
  await expect(row).toBeVisible();
  const [deleted] = await Promise.all([
    page.waitForResponse((r) => r.url().includes('/admin/v1/combos/e2e-fast') && r.request().method() === 'DELETE'),
    page.getByRole('button', { name: 'Delete combo e2e-fast' }).click(),
  ]);
  expect(deleted.status()).toBe(204);
  await expect(row).toHaveCount(0);

  // With the combo gone no route claims the name, so nothing is routed for it.
  const before = (await (await page.request.get('/admin/v1/requests?limit=50')).json()).items.length;
  const res = await call();
  expect(res.status()).toBeGreaterThanOrEqual(400);
  const after = await lastRowFor(page, 'e2e-fast');
  expect(after?.decision?.route_id ?? null).not.toBe('combo:e2e-fast');
  expect(before).toBeGreaterThan(0);
});

test('a combo with no model or an unknown target is refused in the form', async ({ page }) => {
  await page.goto('/combos');
  await page.getByRole('button', { name: 'Create combo' }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('Name / ID').fill('e2e-bad');
  await dialog.getByRole('checkbox', { name: /c1/ }).check();
  // Bypass the browser's required check to reach the gateway's own validation.
  await dialog.getByLabel('Upstream model').evaluate((el: HTMLInputElement) => el.removeAttribute('required'));
  await dialog.getByRole('button', { name: 'Create combo' }).click();
  await expect(dialog.getByRole('alert')).toContainText('Upstream model is required');

  const api = await page.request.post('/admin/v1/combos', {
    data: { id: 'e2e-bad', strategy: 'round_robin', targets: ['nope'], model: 'm' },
  });
  expect(api.status()).toBe(400);
  expect(await api.text()).toContain('nope');
  const list = (await (await page.request.get('/admin/v1/combos')).json()).items;
  expect(list.map((c: { id: string }) => c.id)).not.toContain('e2e-bad');
});
