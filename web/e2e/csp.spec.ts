import { expect, test } from '@playwright/test';

/**
 * The production build served by nginx must load with zero CSP violations. `ng serve` sends no
 * CSP, so this runs against the nginx container (SPEC 11.3). The listener is injected before
 * any page script, through the browser protocol, so the CSP cannot block it.
 */
test('the app loads with zero CSP violations', async ({ page }) => {
  const violations: string[] = [];
  page.on('console', (msg) => {
    if (/content security policy|trusted type/i.test(msg.text())) {
      violations.push(msg.text());
    }
  });
  page.on('pageerror', (err) => violations.push(err.message));
  await page.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', (e) => {
      (window as unknown as { __csp: string[] }).__csp ??= [];
      (window as unknown as { __csp: string[] }).__csp.push(
        `${e.violatedDirective} ${e.blockedURI}`,
      );
    });
  });

  const response = await page.goto('/');
  const csp = response?.headers()['content-security-policy'] ?? '';
  expect(csp).toContain("default-src 'self'");
  expect(csp).toContain("require-trusted-types-for 'script'");
  expect(csp).not.toContain('unsafe-inline');
  expect(csp).not.toContain('unsafe-eval');
  expect(csp).toMatch(/'nonce-[A-Za-z0-9+/=]{16,}'/);

  // Signed out, the guard asks /api/v1/me through the same origin and lands on the sign-in
  // page. No error message there means the API answered 401, so the /api proxy works. An
  // unreachable API would show the unavailable message instead.
  await expect(page).toHaveURL(/\/signin\?returnTo=%2F$/);
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  await expect(page.getByRole('button')).toBeVisible();
  await expect(page.getByRole('alert')).toHaveCount(0);

  const fromPage = await page.evaluate(
    () => (window as unknown as { __csp?: string[] }).__csp ?? [],
  );
  expect([...violations, ...fromPage]).toEqual([]);
});

test('security headers are present on the document', async ({ request }) => {
  const res = await request.get('/');
  const h = res.headers();
  expect(h['x-content-type-options']).toBe('nosniff');
  expect(h['referrer-policy']).toBe('no-referrer');
  expect(h['cross-origin-opener-policy']).toBe('same-origin');
  expect(h['cache-control']).toBe('no-store');
  expect(h['permissions-policy']).toContain('camera=()');
});
