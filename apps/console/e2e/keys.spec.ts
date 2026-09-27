import { expect, test, type Page } from '@playwright/test';
import { DATA_URL } from '../playwright.config';

// API keys, end to end: every console action is checked on the data plane,
// because a badge that says "Disabled" proves nothing if /v1 still answers.

async function dataPlane(page: Page, key?: string) {
  const res = await page.request.get(`${DATA_URL}/v1/models`, {
    headers: key ? { authorization: `Bearer ${key}` } : {},
  });
  return { status: res.status(), body: res.ok() ? await res.json() : null };
}

/** The raw key shown once after create or regenerate. */
async function revealedKey(page: Page): Promise<string> {
  const code = page.getByRole('alert').locator('code').filter({ hasText: /^vkdg_/ }).last();
  await expect(code).toBeVisible();
  return (await code.textContent())!.trim();
}

function row(page: Page, name: string) {
  return page.getByRole('row').filter({ hasText: name });
}

/** Click a row action and require the admin call behind it to succeed. */
async function act(page: Page, button: string, path: RegExp) {
  const [response] = await Promise.all([
    page.waitForResponse((r) => path.test(r.url()) && r.request().method() !== 'GET'),
    page.getByRole('button', { name: button }).click(),
  ]);
  expect(response.status(), `${button} → ${response.url()}`).toBeLessThan(300);
}

test('a key created in the console gates the data plane through its whole life', async ({ page }) => {
  page.on('dialog', (d) => d.accept());
  await page.goto('/keys');

  expect((await dataPlane(page)).status).toBe(401);

  await page.getByLabel('Name').fill('e2e-claude');
  await page.getByLabel('Allowed models').fill('claude-*');
  await page.getByLabel('Requests per minute').fill('50');
  await page.getByRole('button', { name: 'Create key' }).click();

  const key = await revealedKey(page);
  expect(key).toMatch(/^vkdg_[0-9a-f]{32}$/);
  await expect(row(page, 'e2e-claude')).toContainText('Active');
  await expect(row(page, 'e2e-claude')).toContainText('claude-*');
  // The list shows only the prefix, never the secret.
  await expect(row(page, 'e2e-claude')).not.toContainText(key);

  // The key works, and only for the models it allows.
  const allowed = await dataPlane(page, key);
  expect(allowed.status).toBe(200);
  expect(allowed.body.data.map((m: { id: string }) => m.id)).toEqual(['claude-sonnet-4.5']);

  await act(page, 'Disable e2e-claude', /\/keys\/[^/]+\/disable$/);
  await expect(row(page, 'e2e-claude')).toContainText('Disabled');
  expect((await dataPlane(page, key)).status).toBe(401);

  await act(page, 'Enable e2e-claude', /\/keys\/[^/]+\/enable$/);
  await expect(row(page, 'e2e-claude')).toContainText('Active');
  expect((await dataPlane(page, key)).status).toBe(200);

  await act(page, 'Regenerate e2e-claude', /\/keys\/[^/]+\/regenerate$/);
  const rotated = await revealedKey(page);
  expect(rotated).not.toBe(key);
  expect((await dataPlane(page, key)).status).toBe(401);
  expect((await dataPlane(page, rotated)).status).toBe(200);

  await act(page, 'Revoke e2e-claude', /\/keys\/[^/]+$/);
  await expect(row(page, 'e2e-claude')).toContainText('Revoked');
  expect((await dataPlane(page, rotated)).status).toBe(401);
});

test('invalid limits are refused in the form, not stored', async ({ page }) => {
  await page.goto('/keys');
  await page.getByLabel('Name').fill('bad-ip');
  await page.getByLabel('Allowed IPs').fill('192.168.');
  await page.getByRole('button', { name: 'Create key' }).click();
  await expect(page.getByRole('alert').filter({ hasText: 'allowed_ips[0]' })).toBeVisible();
  await expect(row(page, 'bad-ip')).toHaveCount(0);
});
