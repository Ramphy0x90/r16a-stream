import { defineConfig } from '@playwright/test';

/**
 * End to end checks run only against a local stack, never production. The base URL must be
 * loopback, so a mistyped variable cannot point the tests, or the CI test users, at a real
 * deployment (SPEC 6.7).
 */
const baseURL = process.env['R16A_E2E_BASE_URL'] ?? 'http://127.0.0.1:8080';
const host = new URL(baseURL).hostname;
if (!['127.0.0.1', 'localhost', '::1', '[::1]'].includes(host)) {
  throw new Error(`R16A_E2E_BASE_URL must be a loopback address, got ${host}`);
}

export default defineConfig({
  testDir: 'e2e',
  fullyParallel: true,
  retries: 0,
  reporter: [['list']],
  use: {
    baseURL,
    browserName: 'chromium',
    headless: true,
  },
});
