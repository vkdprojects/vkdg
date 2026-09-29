import { defineConfig } from '@playwright/test';
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { ADMIN_PORT, ADMIN_URL, BOOTSTRAP_TOKEN, DATA_DIR, DATA_PORT, SESSION_FILE } from './e2e/env';

// End-to-end tests run the real gateway binary with the console embedded, so
// every assertion crosses the same console → admin API → store path a user
// does. Nothing between the browser and Rust is mocked.
//
// The config is loaded by the runner and by every worker, so preparing the
// data directory only in the runner (no TEST_WORKER_INDEX) keeps one fresh
// directory per run. The upstream is an unroutable address: data-plane calls
// get past auth and fail at dispatch, which is all these tests need.

const dataDir = resolve(DATA_DIR);
const configPath = `${dataDir}/config.yaml`;
if (!process.env.TEST_WORKER_INDEX) {
  rmSync(dataDir, { recursive: true, force: true });
  mkdirSync(dataDir, { recursive: true });
  writeFileSync(
    configPath,
    `listen: "127.0.0.1:${DATA_PORT}"
connections:
  - id: c1
    provider: "custom:http://127.0.0.1:1"
    auth: { type: api_key, env_var: E2E_UPSTREAM_KEY }
    models: ["claude-*", "gpt-4o-mini"]
routes:
  - id: r1
    match_models: ["claude-sonnet-4.5", "gpt-4o-mini"]
    strategy: round_robin
    targets: [c1]
`,
  );
}

export default defineConfig({
  testDir: './e2e',
  // One gateway, one bootstrap token: tests share state, so run them in order.
  fullyParallel: false,
  workers: 1,
  // A failed setup should fail once, with its own error, not twice.
  retries: 0,
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    baseURL: ADMIN_URL,
    trace: 'retain-on-failure',
  },
  projects: [
    // Signs in once with the single-use bootstrap token and saves the session.
    { name: 'setup', testMatch: /global\.setup\.ts/ },
    {
      name: 'console',
      testMatch: /.*\.spec\.ts/,
      dependencies: ['setup'],
      use: { storageState: SESSION_FILE },
    },
  ],
  webServer: {
    // Built from the workspace root; `bun run build` must run first so the
    // binary embeds the current console (see `just e2e`). A debug build: the
    // test-only fake OAuth provider does not exist in release builds.
    command: `cargo run -q -p vkdg -- serve --config ${configPath} --listen 127.0.0.1:${DATA_PORT}`,
    cwd: '../..',
    url: `${ADMIN_URL}/login`,
    timeout: 300_000,
    reuseExistingServer: false,
    env: {
      VKDG_ADMIN_ADDR: `127.0.0.1:${ADMIN_PORT}`,
      VKDG_BOOTSTRAP_TOKEN: BOOTSTRAP_TOKEN,
      VKDG_KEYS_DB: `${dataDir}/keys.db`,
      VKDG_ACCOUNTS_DB: `${dataDir}/accounts.db`,
      VKDG_PLUGINS_DIR: `${dataDir}/plugins`,
      VKDG_E2E_FAKE_OAUTH: '1',
      E2E_UPSTREAM_KEY: 'unused',
    },
  },
});

