import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { TranslocoPipe } from '@jsverse/transloco';

import { SessionService } from '../core/auth/session.service';

/** Errors the BFF and the guard put in the URL. Anything else shows no message. */
const KNOWN_ERRORS = ['failed', 'denied', 'unavailable', 'setup'] as const;
/** The API's request id, the only form a setup reference can take. */
const REFERENCE = /^[0-9a-f]{32}$/;
type SigninError = (typeof KNOWN_ERRORS)[number];

/**
 * Entry point for signed-out visitors and the landing page of a failed login. The button
 * leaves the app for Authentik, there is no password form here.
 */
@Component({
  selector: 'app-signin-page',
  imports: [TranslocoPipe],
  templateUrl: './signin.page.html',
  styleUrl: './signin.page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SigninPage {
  private readonly session = inject(SessionService);

  /** Bound from the query string. */
  readonly error = input<string | undefined>();
  readonly returnTo = input<string | undefined>();
  /** Bound from `ref`. Names the log line that says which Authentik setting is missing. */
  readonly ref = input<string | undefined>();

  readonly message = computed<SigninError | null>(() => {
    const e = this.error();
    return KNOWN_ERRORS.find((k) => k === e) ?? null;
  });

  /**
   * Shown only for setup errors and only in the request id format, so the page never displays
   * arbitrary text from the URL. The reference itself reveals nothing, an admin looks it up in
   * the API logs.
   */
  readonly reference = computed(() => {
    const r = this.ref();
    return this.message() === 'setup' && r !== undefined && REFERENCE.test(r) ? r : null;
  });

  signIn(): void {
    this.session.signIn(this.returnTo() ?? '/');
  }
}
