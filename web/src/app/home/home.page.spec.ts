import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideZonelessChangeDetection } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { provideTransloco } from '@jsverse/transloco';

import { HomePage } from './home.page';

const card = (n: number) => ({
  id: `id-${n}`,
  kind: 'movie',
  name: `Movie ${n}`,
  year: 2000 + n,
  poster: null,
});

describe('HomePage', () => {
  function setup(inputs: Record<string, string> = {}) {
    TestBed.configureTestingModule({
      imports: [HomePage],
      providers: [
        provideZonelessChangeDetection(),
        provideRouter([]),
        provideHttpClient(),
        provideHttpClientTesting(),
        provideTransloco({ config: { availableLangs: ['en'], defaultLang: 'en' } }),
      ],
    });
    const fixture = TestBed.createComponent(HomePage);
    for (const [k, v] of Object.entries(inputs)) fixture.componentRef.setInput(k, v);
    fixture.detectChanges();
    const http = TestBed.inject(HttpTestingController);
    http.expectOne('/api/v1/genres').flush([]);
    return { fixture, http };
  }

  it('loads the first page with the filters from the URL', () => {
    const { fixture, http } = setup({ type: 'show', q: ' wire ', sort: 'az' });
    const req = http.expectOne((r) => r.url === '/api/v1/titles');
    expect(req.request.params.get('type')).toBe('show');
    expect(req.request.params.get('q')).toBe('wire');
    expect(req.request.params.get('sort')).toBe('az');
    expect(req.request.params.has('genre')).toBe(false);
    req.flush({ items: [card(1)], nextCursor: 'c1' });
    expect(fixture.componentInstance.items().length).toBe(1);
    expect(fixture.componentInstance.hasMore()).toBe(true);
  });

  it('appends the next page from the cursor', () => {
    const { fixture, http } = setup();
    http.expectOne((r) => r.url === '/api/v1/titles').flush({ items: [card(1)], nextCursor: 'c1' });
    fixture.componentInstance.loadMore();
    const next = http.expectOne(
      (r) => r.url === '/api/v1/titles' && r.params.get('cursor') === 'c1',
    );
    next.flush({ items: [card(2)], nextCursor: null });
    expect(fixture.componentInstance.items().map((t) => t.name)).toEqual(['Movie 1', 'Movie 2']);
    expect(fixture.componentInstance.hasMore()).toBe(false);
  });

  it('shows an error instead of failing', () => {
    const { fixture, http } = setup();
    http.expectOne((r) => r.url === '/api/v1/titles').flush(null, { status: 500, statusText: 'x' });
    expect(fixture.componentInstance.state()).toBe('error');
  });

  it('ignores unknown filter values from the URL', () => {
    const { fixture, http } = setup({ type: 'episode', sort: 'random' });
    const req = http.expectOne((r) => r.url === '/api/v1/titles');
    expect(req.request.params.has('type')).toBe(false);
    expect(req.request.params.has('sort')).toBe(false);
    req.flush({ items: [], nextCursor: null });
    expect(fixture.componentInstance.items()).toEqual([]);
  });
});
