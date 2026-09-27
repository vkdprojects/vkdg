import { expect, test as setup } from '@playwright/test';
import { BOOTSTRAP_TOKEN } from '../playwright.config';

export const SESSION_FILE = 'e2e/.auth/session.json';

// The bootstrap token signs in exactly once per gateway, so the suite signs in
// here and every other test reuses the saved session cookie.
setup('sign in with the bootstrap token', async ({ page }) => {
  await page.goto('/keys');

  // Signed out: bounced to the sign-in form, with no app navigation.
  await expect(page).toHaveURL(/\/login$/);
  await expect(page.getByLabel('Bootstrap token')).toBeVisible();
  await expect(page.getByRole('link', { name: 'Keys' })).toHaveCount(0);

  await page.getByLabel('Bootstrap token').fill(BOOTSTRAP_TOKEN);
  await page.getByRole('button', { name: 'Sign in' }).click();

  // Signed in: the app shell appears without a reload.
  await expect(page.getByRole('link', { name: 'Keys' })).toBeVisible();
  await page.context().storageState({ path: SESSION_FILE });
});
