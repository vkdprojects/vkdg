import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/__tests__/**/*.test.ts'],
    // Origin for the browser admin client (src/lib/api.ts) under test.
    env: { VITE_ADMIN_URL: 'http://admin.test' },
  },
});
