import { HttpClient, provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideZonelessChangeDetection } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { SessionService } from './session.service';
import { unauthorizedInterceptor } from './unauthorized.interceptor';

describe('unauthorizedInterceptor', () => {
  function setup() {
    TestBed.configureTestingModule({
      providers: [
        provideZonelessChangeDetection(),
        provideRouter([]),
        provideHttpClient(withInterceptors([unauthorizedInterceptor])),
        provideHttpClientTesting(),
      ],
    });
    const lost = vi
      .spyOn(TestBed.inject(SessionService), 'sessionLost')
      .mockImplementation(() => undefined);
    return { http: TestBed.inject(HttpClient), ctl: TestBed.inject(HttpTestingController), lost };
  }

  it('sends the user to sign in when an API call answers 401', () => {
    const { http, ctl, lost } = setup();
    http.get('/api/v1/titles').subscribe({ error: () => undefined });
    ctl.expectOne('/api/v1/titles').flush(null, { status: 401, statusText: 'Unauthorized' });
    expect(lost).toHaveBeenCalledOnce();
  });

  it('leaves the session probe, other statuses and non-API calls alone', () => {
    const { http, ctl, lost } = setup();
    http.get('/api/v1/me').subscribe({ error: () => undefined });
    ctl.expectOne('/api/v1/me').flush(null, { status: 401, statusText: 'Unauthorized' });
    http.get('/api/v1/titles').subscribe({ error: () => undefined });
    ctl.expectOne('/api/v1/titles').flush(null, { status: 404, statusText: 'Not Found' });
    http.get('/i18n/en.json').subscribe({ error: () => undefined });
    ctl.expectOne('/i18n/en.json').flush(null, { status: 401, statusText: 'Unauthorized' });
    expect(lost).not.toHaveBeenCalled();
  });
});
