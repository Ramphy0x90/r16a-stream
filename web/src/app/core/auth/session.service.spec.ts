import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideZonelessChangeDetection } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import { NAVIGATOR } from './navigator';
import { type Me, SessionService } from './session.service';

const ME: Me = {
  id: '00000000-0000-4000-8000-000000000001',
  displayName: 'Ada',
  isAdmin: false,
  uiLanguage: 'en',
  theme: 'dark',
  preferredAudioLanguage: null,
};

describe('SessionService', () => {
  function setup() {
    const assign = vi.fn();
    TestBed.configureTestingModule({
      providers: [
        provideZonelessChangeDetection(),
        provideRouter([]),
        provideHttpClient(),
        provideHttpClientTesting(),
        { provide: NAVIGATOR, useValue: { assign } },
      ],
    });
    return {
      session: TestBed.inject(SessionService),
      http: TestBed.inject(HttpTestingController),
      router: TestBed.inject(Router),
      assign,
    };
  }

  it('loads the user once and shares the answer', async () => {
    const { session, http } = setup();
    const a = session.ensureLoaded();
    const b = session.ensureLoaded();
    http.expectOne('/api/v1/me').flush(ME);
    expect(await a).toBe('signed-in');
    expect(await b).toBe('signed-in');
    expect(session.me()?.displayName).toBe('Ada');
    http.verify();
  });

  it('treats 401 as signed out and other failures as unavailable', async () => {
    const first = setup();
    const p = first.session.ensureLoaded();
    first.http.expectOne('/api/v1/me').flush(null, { status: 401, statusText: 'Unauthorized' });
    expect(await p).toBe('signed-out');

    TestBed.resetTestingModule();
    const second = setup();
    const q = second.session.ensureLoaded();
    second.http.expectOne('/api/v1/me').flush(null, { status: 503, statusText: 'Unavailable' });
    expect(await q).toBe('unavailable');
  });

  it('starts the BFF login with an encoded same-origin return path', () => {
    const { session, assign } = setup();
    session.signIn('/title/1?x=a b');
    expect(assign).toHaveBeenCalledWith('/api/v1/auth/login?returnTo=%2Ftitle%2F1%3Fx%3Da%20b');
    session.signIn('https://evil.example');
    expect(assign).toHaveBeenLastCalledWith('/api/v1/auth/login?returnTo=%2F');
    session.signIn('/signin?error=failed');
    expect(assign).toHaveBeenLastCalledWith('/api/v1/auth/login?returnTo=%2F');
  });

  it('signs out on the server, then visits the end-session page', async () => {
    const { session, http, assign } = setup();
    const done = session.signOut();
    http.expectOne({ method: 'POST', url: '/api/v1/auth/logout' }).flush({
      endSessionUrl: 'https://auth.example/application/o/r16a-stream/end-session/',
    });
    await done;
    expect(session.state()).toBe('signed-out');
    expect(assign).toHaveBeenCalledWith(
      'https://auth.example/application/o/r16a-stream/end-session/',
    );
  });

  it('goes to the sign-in page when Authentik gave no end-session URL', async () => {
    const { session, http, router, assign } = setup();
    const nav = vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true);
    const done = session.signOut();
    http.expectOne('/api/v1/auth/logout').flush({ endSessionUrl: null });
    await done;
    expect(assign).not.toHaveBeenCalled();
    expect(nav).toHaveBeenCalledWith('/signin');
  });
});
