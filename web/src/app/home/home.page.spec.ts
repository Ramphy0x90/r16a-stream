import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideZonelessChangeDetection } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideTransloco } from '@jsverse/transloco';

import { HomePage } from './home.page';

describe('HomePage', () => {
  function setup() {
    TestBed.configureTestingModule({
      imports: [HomePage],
      providers: [
        provideZonelessChangeDetection(),
        provideHttpClient(),
        provideHttpClientTesting(),
        provideTransloco({ config: { availableLangs: ['en'], defaultLang: 'en' } }),
      ],
    });
    const fixture = TestBed.createComponent(HomePage);
    const http = TestBed.inject(HttpTestingController);
    return { fixture, http };
  }

  it('starts in the checking state', () => {
    const { fixture } = setup();
    expect(fixture.componentInstance.api()).toBe('checking');
  });

  it('reports a reachable API', () => {
    const { fixture, http } = setup();
    http.expectOne('/api/v1/status').flush({ service: 'r16a-stream', apiVersion: 1 });
    expect(fixture.componentInstance.api()).toBe('reachable');
  });

  it('reports an unreachable API instead of failing', () => {
    const { fixture, http } = setup();
    http.expectOne('/api/v1/status').flush(null, { status: 502, statusText: 'Bad Gateway' });
    expect(fixture.componentInstance.api()).toBe('unreachable');
  });
});
