import { expect, test, type Page } from '@playwright/test';
import { DATA_URL } from './env';

// Combos, end to end: a combo created in the console must change routing on
// the data plane at once, and deleting it must hand its traffic back. c1 points
// at a closed port, so the call itself fails; the request history row shows
// which route and connection the gateway picked.

type Row = { request_id: string; model: string; connection_id: string | null; decision?: { route_id: string | null } | null };

async function history(page: Page): Promise<Row[]> {
  return (await (await page.request.get('/admin/v1/requests?limit=50')).json()).items;
}

/** The row a call just added: newest row for `model` whose id was not there before. */
async function newRow(page: Page, model: string, before: Set<string>): Promise<Row | undefined> {
  return (await history(page)).find((r) => r.model === model && !before.has(r.request_id));
}

async function listedModels(page: Page, key: string): Promise<string[]> {
  const res = await page.request.get(`${DATA_URL}/v1/models`, { headers: { authorization: `Bearer ${key}` } });
  return (await res.json()).data.map((m: { id: string }) => m.id);
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

  expect(await listedModels(page, key)).toContain('e2e-fast');

  let before = new Set((await history(page)).map((r) => r.request_id));
  await call();
  await expect.poll(async () => (await newRow(page, 'e2e-fast', before))?.decision?.route_id).toBe('combo:e2e-fast');
  expect((await newRow(page, 'e2e-fast', before))?.connection_id).toBe('c1');

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
  expect(await listedModels(page, key)).not.toContain('e2e-fast');

  // With the combo gone no route claims the name: the call is refused, and the
  // row it leaves (failed requests are logged too) was not routed through it.
  before = new Set((await history(page)).map((r) => r.request_id));
  const res = await call();
  expect(res.status()).toBeGreaterThanOrEqual(400);
  await expect.poll(async () => (await newRow(page, 'e2e-fast', before)) !== undefined).toBe(true);
  const after = await newRow(page, 'e2e-fast', before);
  expect(after!.decision?.route_id ?? null).not.toBe('combo:e2e-fast');
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
