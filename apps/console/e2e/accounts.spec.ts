import { expect, test, type Page } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { DATA_DIR, DATA_URL } from './env';

// Connecting accounts through the console, against the test-only `fake-oauth`
// provider (enabled by VKDG_E2E_FAKE_OAUTH=1 in playwright.config.ts). It runs
// the same start → device code → poll → account store path as Kiro, without a
// person approving a code on a third-party site.

async function connectFakeAccount(page: Page): Promise<string> {
  await page.getByRole('button', { name: 'Connect account' }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('Provider').click();
  await page.getByRole('option', { name: 'Fake OAuth (e2e)' }).click();
  // Never press Start on a real provider: a failed selection would begin a
  // live AWS device login from CI.
  await expect(dialog.getByLabel('Provider')).toHaveText('Fake OAuth (e2e)');
  await expect(dialog.getByLabel('AWS region')).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Start login' }).click();

  // The device code is shown, then polling completes the login.
  const code = dialog.getByText(/^FAKE-\d{4}$/);
  await expect(code).toBeVisible();
  const n = Number((await code.textContent())!.slice(-4));
  const label = `e2e-user-${n}@example.test`;
  await expect(page.getByRole('row').filter({ hasText: label })).toContainText('Active');
  await expect(dialog).toBeHidden();
  return label;
}

test('two accounts on the same provider are connected and listed independently', async ({ page }) => {
  await page.goto('/accounts');
  const first = await connectFakeAccount(page);
  const second = await connectFakeAccount(page);
  expect(second).not.toBe(first);

  const api = await (await page.request.get('/admin/v1/accounts')).json();
  const fake = api.items.filter((a: { provider: string }) => a.provider === 'fake-oauth');
  expect(fake.map((a: { label: string }) => a.label).sort()).toEqual([first, second].sort());
  expect(fake.every((a: { status: string }) => a.status === 'active')).toBe(true);
});

test('deleting one account leaves the other connected', async ({ page }) => {
  await page.goto('/accounts');
  const keep = await connectFakeAccount(page);
  const drop = await connectFakeAccount(page);

  const dropRow = page.getByRole('row').filter({ hasText: drop });
  await dropRow.getByRole('button', { name: /Delete/ }).click();
  const confirm = page.getByRole('alertdialog');
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: /Delete/ }).click();

  await expect(dropRow).toHaveCount(0);
  await expect(page.getByRole('row').filter({ hasText: keep })).toContainText('Active');
});

test('the provider list comes from the gateway, not a hardcoded table', async ({ page }) => {
  await page.goto('/accounts');
  await page.getByRole('button', { name: 'Connect account' }).click();
  await page.getByRole('dialog').getByLabel('Provider').click();
  await expect(page.getByRole('option', { name: 'Fake OAuth (e2e)' })).toBeVisible();
  await expect(page.getByRole('option', { name: /Kiro/ })).toBeVisible();
});

test('a revoked account shows Needs login and Reconnect restores it in place', async ({ page }) => {
  // An imported fake account whose access token is already expired and whose
  // refresh token the fake provider rejects as revoked.
  const imported = await page.request.post('/admin/v1/oauth/fake-oauth/import', {
    data: { method: 'import', params: { refresh_token: 'revoked-e2e' } },
  });
  expect(imported.status()).toBe(201);
  const account = (await imported.json()).account;

  // Route a model to it through a config reload, as an operator would.
  const configPath = resolve(DATA_DIR, 'config.yaml');
  const original = readFileSync(configPath, 'utf8');
  writeFileSync(
    configPath,
    original.replace(
      'routes:\n',
      `  - id: fake-acct
    provider: fake-oauth
    auth: { type: account, account: "${account.id}" }
    models: ["fake-model"]
routes:
  - id: r-fake
    match_models: ["fake-model"]
    strategy: round_robin
    targets: [fake-acct]
`,
    ),
  );
  try {
    const key = (await (await page.request.post('/admin/v1/keys', { data: { name: 'revoke-probe' } })).json()).key;
    const auth = { authorization: `Bearer ${key}` };
    await expect
      .poll(async () => JSON.stringify(await (await page.request.get(`${DATA_URL}/v1/models`, { headers: auth })).json()))
      .toContain('fake-model');

    // The request needs a refresh; the refresh is rejected as revoked.
    const call = await page.request.post(`${DATA_URL}/v1/chat/completions`, {
      headers: auth,
      data: { model: 'fake-model', messages: [{ role: 'user', content: 'hi' }] },
    });
    expect(call.status()).toBe(401);

    await page.goto('/accounts');
    const row = page.getByRole('row').filter({ hasText: account.label });
    await expect(row).toContainText('Needs login');
    await expect(row).toContainText('revoked');

    await row.getByRole('button', { name: /Reconnect/ }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog.getByLabel('Provider')).toHaveText('Fake OAuth (e2e)');
    await expect(dialog.getByLabel('AWS region')).toHaveCount(0);
    await dialog.getByRole('button', { name: 'Start login' }).click();
    await expect(dialog).toBeHidden();

    // Same account id, now active: the connection referencing it recovers.
    // The label follows the new login's identity, so assert by id.
    await expect(page.getByText('Needs login')).toHaveCount(0);
    const after = await (await page.request.get('/admin/v1/accounts')).json();
    const same = after.items.filter((a: { id: string }) => a.id === account.id);
    expect(same).toHaveLength(1);
    expect(same[0].status).toBe('active');
    await expect(page.getByRole('row').filter({ hasText: same[0].label })).toContainText('Active');
  } finally {
    writeFileSync(configPath, original);
  }
});
