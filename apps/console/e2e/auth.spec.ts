import { expect, test } from '@playwright/test';
import { ADMIN_PASSWORD, BOOTSTRAP_TOKEN } from './env';

// Sign-in after first run. These start signed out on purpose.
test.use({ storageState: { cookies: [], origins: [] } });

test('once a password is set, it signs in repeatedly and the token no longer does', async ({ page }) => {
  // The old single-use token locked the admin out after one sign-out.
  for (let i = 0; i < 2; i++) {
    await page.goto('/login');
    await expect(page.getByLabel('Password')).toBeVisible();
    await expect(page.getByLabel('Bootstrap token')).toHaveCount(0);
    await page.getByLabel('Password').fill(ADMIN_PASSWORD);
    await page.getByRole('button', { name: 'Sign in' }).click();
    await expect(page.getByRole('link', { name: 'Keys' })).toBeVisible();
    await page.getByRole('button', { name: 'Sign out' }).click();
    await expect(page).toHaveURL(/\/login$/);
  }

  const token = await page.request.post('/admin/v1/session', { data: { token: BOOTSTRAP_TOKEN } });
  expect(token.status()).toBe(401);
});

test('a wrong password shows an alert and grants nothing', async ({ page }) => {
  await page.goto('/login');
  await page.getByLabel('Password').fill('definitely not it');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('alert')).toContainText('wrong password');
  await expect(page.getByRole('link', { name: 'Keys' })).toHaveCount(0);
});
