import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideZonelessChangeDetection } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideTransloco } from '@jsverse/transloco';

import { TitlePage } from './title.page';

const detail = {
  id: 't1',
  kind: 'show',
  name: 'The Wire',
  originalName: null,
  overview: null,
  year: 2002,
  runtimeMinutes: null,
  genres: [],
  credits: [
    { personName: 'David Simon', character: null, role: 'creator' },
    { personName: 'Dominic West', character: 'McNulty', role: 'cast' },
  ],
  seasons: [
    { number: 0, name: null, year: null, episodeCount: 1 },
    { number: 1, name: null, year: 2002, episodeCount: 13 },
  ],
  languages: [],
  poster: null,
  backdrop: null,
};

describe('TitlePage', () => {
  function setup() {
    TestBed.configureTestingModule({
      imports: [TitlePage],
      providers: [
        provideZonelessChangeDetection(),
        provideHttpClient(),
        provideHttpClientTesting(),
        provideTransloco({ config: { availableLangs: ['en'], defaultLang: 'en' } }),
      ],
    });
    const fixture = TestBed.createComponent(TitlePage);
    fixture.componentRef.setInput('id', 't1');
    fixture.detectChanges();
    return { fixture, http: TestBed.inject(HttpTestingController) };
  }

  it('opens on the first regular season, not the specials', () => {
    const { fixture, http } = setup();
    http.expectOne('/api/v1/titles/t1').flush(detail);
    http.expectOne('/api/v1/titles/t1/seasons/1/episodes').flush([]);
    expect(fixture.componentInstance.season()).toBe(1);
    expect(fixture.componentInstance.people().leads.map((c) => c.personName)).toEqual([
      'David Simon',
    ]);
  });

  it('tells a missing title apart from an error', () => {
    const { fixture, http } = setup();
    http.expectOne('/api/v1/titles/t1').flush(null, { status: 404, statusText: 'Not Found' });
    expect(fixture.componentInstance.loaded().kind).toBe('missing');
  });
});
