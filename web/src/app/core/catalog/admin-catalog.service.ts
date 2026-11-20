import { HttpClient, HttpParams } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { type Observable } from 'rxjs';

import { type components } from '../api/schema';

export type TitleInput = components['schemas']['TitleInput'];
export type TitlePatch = components['schemas']['TitlePatch'];
export type SeasonInput = components['schemas']['SeasonInput'];
export type SeasonPatch = components['schemas']['SeasonPatch'];
export type EpisodeInput = components['schemas']['EpisodeInput'];
export type EpisodePatch = components['schemas']['EpisodePatch'];
export type AdminTitle = components['schemas']['AdminTitle'];
export type AdminTitlePage = components['schemas']['AdminTitlePage'];
type Created = components['schemas']['Created'];

/** Catalog administration. The API refuses every call from non-admins with 404. */
@Injectable({ providedIn: 'root' })
export class AdminCatalogService {
  private readonly http = inject(HttpClient);

  list(status: string | null, q: string, cursor?: string | null): Observable<AdminTitlePage> {
    let params = new HttpParams();
    if (status) params = params.set('status', status);
    if (q.trim()) params = params.set('q', q.trim());
    if (cursor) params = params.set('cursor', cursor);
    return this.http.get<AdminTitlePage>('/api/v1/admin/titles', { params });
  }

  get(id: string): Observable<AdminTitle> {
    return this.http.get<AdminTitle>(`/api/v1/admin/titles/${encodeURIComponent(id)}`);
  }

  /** Prefills a draft from a TMDB link, or a numeric id with its kind. Saves nothing. */
  lookup(ref: string, kind?: 'movie' | 'show'): Observable<TitleInput> {
    let params = new HttpParams().set('ref', ref.trim());
    if (kind) params = params.set('kind', kind);
    return this.http.get<TitleInput>('/api/v1/admin/tmdb/lookup', { params });
  }

  create(input: TitleInput): Observable<Created> {
    return this.http.post<Created>('/api/v1/admin/titles', input);
  }

  update(id: string, patch: TitlePatch): Observable<null> {
    return this.http.patch<null>(`/api/v1/admin/titles/${encodeURIComponent(id)}`, patch);
  }

  remove(id: string): Observable<null> {
    return this.http.delete<null>(`/api/v1/admin/titles/${encodeURIComponent(id)}`);
  }

  setPublished(id: string, published: boolean): Observable<null> {
    const action = published ? 'publish' : 'unpublish';
    return this.http.post<null>(`/api/v1/admin/titles/${encodeURIComponent(id)}/${action}`, null);
  }

  addSeason(titleId: string, season: SeasonInput): Observable<Created> {
    return this.http.post<Created>(
      `/api/v1/admin/titles/${encodeURIComponent(titleId)}/seasons`,
      season,
    );
  }

  removeSeason(id: string): Observable<null> {
    return this.http.delete<null>(`/api/v1/admin/seasons/${encodeURIComponent(id)}`);
  }

  addEpisode(seasonId: string, episode: EpisodeInput): Observable<Created> {
    return this.http.post<Created>(
      `/api/v1/admin/seasons/${encodeURIComponent(seasonId)}/episodes`,
      episode,
    );
  }

  removeEpisode(id: string): Observable<null> {
    return this.http.delete<null>(`/api/v1/admin/episodes/${encodeURIComponent(id)}`);
  }
}
