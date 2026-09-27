import { expect, test as setup } from '@playwright/test';
import { ADMIN_PASSWORD, BOOTSTRAP_TOKEN, SESSION_FILE } from './env';

// First run as an operator would do it: the bootstrap token once, then a
// console password. Every other test reuses the saved session.
setup('first run: bootstrap token, then set the console password', async ({ page }) => {
  await page.goto('/keys');

  // Signed out: bounced to the sign-in form, with no app navigation.
  await expect(page).toHaveURL(/\/login$/);
  await expect(page.getByLabel('Bootstrap token')).toBeVisible();
  await expect(page.getByRole('link', { name: 'Keys' })).toHaveCount(0);

  await page.getByLabel('Bootstrap token').fill(BOOTSTRAP_TOKEN);
  await page.getByRole('button', { name: 'Sign in' }).click();

  await expect(page.getByRole('heading', { name: 'Set a console password' })).toBeVisible();
  await page.getByLabel('New console password').fill(ADMIN_PASSWORD);
  await page.getByLabel('Repeat the password').fill(ADMIN_PASSWORD);
  await page.getByRole('button', { name: 'Save password' }).click();

  // Signed in: the app shell appears without a reload.
  await expect(page.getByRole('link', { name: 'Keys' })).toBeVisible();
  await page.context().storageState({ path: SESSION_FILE });
});
