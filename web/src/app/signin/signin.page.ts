import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { TranslocoPipe } from '@jsverse/transloco';

import { SessionService } from '../core/auth/session.service';

/** Errors the BFF and the guard put in the URL. Anything else shows no message. */
const KNOWN_ERRORS = ['failed', 'denied', 'unavailable'] as const;
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

  readonly message = computed<SigninError | null>(() => {
    const e = this.error();
    return KNOWN_ERRORS.find((k) => k === e) ?? null;
  });

  signIn(): void {
    this.session.signIn(this.returnTo() ?? '/');
  }
}
