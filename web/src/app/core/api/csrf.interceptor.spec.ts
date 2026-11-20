import { HttpClient, provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideZonelessChangeDetection } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { csrfHeaderInterceptor } from './csrf.interceptor';

describe('csrfHeaderInterceptor', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [
        provideZonelessChangeDetection(),
        provideHttpClient(withInterceptors([csrfHeaderInterceptor])),
        provideHttpClientTesting(),
      ],
    });
  });

  it('marks API calls', () => {
    TestBed.inject(HttpClient).get('/api/v1/status').subscribe();
    const req = TestBed.inject(HttpTestingController).expectOne('/api/v1/status');
    expect(req.request.headers.get('X-R16a-Request')).toBe('1');
  });

  it('leaves other requests alone, such as translation files and bucket paths', () => {
    TestBed.inject(HttpClient).get('/i18n/en.json').subscribe();
    const req = TestBed.inject(HttpTestingController).expectOne('/i18n/en.json');
    expect(req.request.headers.has('X-R16a-Request')).toBe(false);
  });
});
