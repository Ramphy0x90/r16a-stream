import { HttpClient, HttpErrorResponse } from '@angular/common/http';
import { Injectable, computed, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import { firstValueFrom } from 'rxjs';

import { type components } from '../api/schema';
import { NAVIGATOR } from './navigator';

export type Me = components['schemas']['MeResponse'];
type LogoutResponse = components['schemas']['LogoutResponse'];

/**
 * `unknown` until the first `/me` answer, then `signed-in`, `signed-out`, or `unavailable` when
 * the API could not tell, so a server outage is not mistaken for a logout.
 */
export type SessionState = 'unknown' | 'signed-in' | 'signed-out' | 'unavailable';

/**
 * The web session as the browser sees it. The session itself lives in an HttpOnly cookie that
 * scripts cannot read (SPEC 6.3). This service only knows what `/api/v1/me` answers.
 */
@Injectable({ providedIn: 'root' })
export class SessionService {
  private readonly http = inject(HttpClient);
  private readonly router = inject(Router);
  private readonly navigator = inject(NAVIGATOR);

  private readonly current = signal<Me | null>(null);
  private readonly stateSignal = signal<SessionState>('unknown');
  private loading: Promise<SessionState> | null = null;

  readonly me = this.current.asReadonly();
  readonly state = this.stateSignal.asReadonly();
  /** Shows or hides admin navigation only. The API decides admin rights on every call. */
  readonly isAdmin = computed(() => this.current()?.isAdmin ?? false);

  /** Loads the session once and shares the answer between concurrent callers. */
  ensureLoaded(): Promise<SessionState> {
    if (this.stateSignal() !== 'unknown') {
      return Promise.resolve(this.stateSignal());
    }
    this.loading ??= this.load().finally(() => (this.loading = null));
    return this.loading;
  }

  private async load(): Promise<SessionState> {
    try {
      this.current.set(await firstValueFrom(this.http.get<Me>('/api/v1/me')));
      this.stateSignal.set('signed-in');
    } catch (e) {
      const signedOut = e instanceof HttpErrorResponse && e.status === 401;
      this.current.set(null);
      this.stateSignal.set(signedOut ? 'signed-out' : 'unavailable');
    }
    return this.stateSignal();
  }

  /**
   * Full-page navigation to the BFF login, which redirects to Authentik. `returnTo` is checked
   * again by the server, anything but a same-origin path becomes `/`.
   */
  signIn(returnTo: string): void {
    const target = returnTo.startsWith('/') && !returnTo.startsWith('/signin') ? returnTo : '/';
    this.navigator.assign(`/api/v1/auth/login?returnTo=${encodeURIComponent(target)}`);
  }

  /**
   * Ends the session on the server, then visits Authentik's end-session page so the identity
   * provider session ends too. Its logout flow sends the browser back to the start page.
   */
  async signOut(): Promise<void> {
    let endSessionUrl: string | null | undefined = null;
    try {
      endSessionUrl = (
        await firstValueFrom(this.http.post<LogoutResponse>('/api/v1/auth/logout', null))
      ).endSessionUrl;
    } catch {
      // The session may already be gone. The local state is cleared either way.
    }
    this.markSignedOut();
    if (endSessionUrl) {
      this.navigator.assign(endSessionUrl);
    } else {
      await this.router.navigateByUrl('/signin');
    }
  }

  /** Called when any API call answers 401, the session expired or was revoked. */
  sessionLost(returnTo: string): void {
    if (this.stateSignal() === 'signed-out') {
      return;
    }
    this.markSignedOut();
    void this.router.navigate(['/signin'], { queryParams: { returnTo } });
  }

  private markSignedOut(): void {
    this.current.set(null);
    this.stateSignal.set('signed-out');
  }
}
