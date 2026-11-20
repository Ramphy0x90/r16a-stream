import { HttpErrorResponse, provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideZonelessChangeDetection } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { provideTransloco } from '@jsverse/transloco';

import { AdminTitleNewPage, problemKey } from './admin-title-new.page';

describe('AdminTitleNewPage', () => {
  function setup() {
    TestBed.configureTestingModule({
      imports: [AdminTitleNewPage],
      providers: [
        provideZonelessChangeDetection(),
        provideRouter([]),
        provideHttpClient(),
        provideHttpClientTesting(),
        provideTransloco({ config: { availableLangs: ['en'], defaultLang: 'en' } }),
      ],
    });
    const fixture = TestBed.createComponent(AdminTitleNewPage);
    fixture.detectChanges();
    return { fixture, http: TestBed.inject(HttpTestingController), router: TestBed.inject(Router) };
  }

  it('prefills the draft from TMDB and saves it as one request', () => {
    const { fixture, http, router } = setup();
    const nav = vi.spyOn(router, 'navigate').mockResolvedValue(true);
    const form = document.createElement('form');
    form.innerHTML =
      '<input name="ref" value="https://www.themoviedb.org/movie/949"><input name="kind" value="">';
    fixture.componentInstance.lookup({
      preventDefault: () => undefined,
      target: form,
    } as unknown as Event);
    const req = http.expectOne((r) => r.url === '/api/v1/admin/tmdb/lookup');
    expect(req.request.params.get('ref')).toBe('https://www.themoviedb.org/movie/949');
    expect(req.request.params.has('kind')).toBe(false);
    req.flush({
      kind: 'movie',
      name: 'Heat',
      newGenres: [{ tmdbId: 28, name: 'Action' }],
      credits: [],
    });
    expect(fixture.componentInstance.draft().name).toBe('Heat');

    fixture.componentInstance.removeGenre(0);
    fixture.componentInstance.save();
    const post = http.expectOne({ method: 'POST', url: '/api/v1/admin/titles' });
    expect(post.request.body.newGenres).toEqual([]);
    post.flush({ id: 'new-id' });
    expect(nav).toHaveBeenCalledWith(['/admin/titles', 'new-id']);
  });

  it('maps API problems to messages', () => {
    const err = (status: number) => new HttpErrorResponse({ status });
    expect(problemKey(err(409))).toBe('admin.error.conflict');
    expect(problemKey(err(503))).toBe('admin.error.unavailable');
    expect(problemKey(new Error('x'))).toBe('catalog.error');
  });

  it('switching to a show drops the movie runtime', () => {
    const { fixture } = setup();
    const input = (value: string) => ({ target: { value } }) as unknown as Event;
    fixture.componentInstance.set('runtimeMinutes', input('120'));
    fixture.componentInstance.set('kind', input('show'));
    expect(fixture.componentInstance.draft().kind).toBe('show');
    expect(fixture.componentInstance.draft().runtimeMinutes).toBeNull();
  });
});
