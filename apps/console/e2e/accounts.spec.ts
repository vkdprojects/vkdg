import { expect, test, type Page } from '@playwright/test';

// Connecting accounts through the console, against the test-only `fake-oauth`
// provider (enabled by VKDG_E2E_FAKE_OAUTH=1 in playwright.config.ts). It runs
// the same start → device code → poll → account store path as Kiro, without a
// person approving a code on a third-party site.

async function connectFakeAccount(page: Page): Promise<string> {
  await page.getByRole('button', { name: 'Connect account' }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('Provider').click();
  await page.getByRole('option', { name: 'Fake OAuth (e2e)' }).click();
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
  page.on('dialog', (d) => d.accept());
  await page.goto('/accounts');
  const keep = await connectFakeAccount(page);
  const drop = await connectFakeAccount(page);

  const dropRow = page.getByRole('row').filter({ hasText: drop });
  await dropRow.getByRole('button', { name: /Delete/ }).click();
  const confirm = page.getByRole('alertdialog');
  if (await confirm.isVisible()) await confirm.getByRole('button', { name: /Delete/ }).click();

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
