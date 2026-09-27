// Shared e2e constants. No side effects: imported by the config, the setup and
// the specs, each of which Playwright may load in several processes.

export const ADMIN_PORT = 19290;
export const DATA_PORT = 18290;
export const ADMIN_URL = `http://127.0.0.1:${ADMIN_PORT}`;
export const DATA_URL = `http://127.0.0.1:${DATA_PORT}`;
export const BOOTSTRAP_TOKEN = 'e2e-bootstrap-token';
export const SESSION_FILE = 'e2e/.auth/session.json';
/** Per-run data directory. Not under test-results: Playwright empties that
 *  directory at start, after the config has written the gateway's config. */
export const DATA_DIR = 'e2e/.data';
