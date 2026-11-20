import { HttpClient, HttpParams } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { type Observable } from 'rxjs';

import { type components } from '../api/schema';

export type TitleCard = components['schemas']['TitleCard'];
export type TitlePage = components['schemas']['TitlePage'];
export type TitleDetail = components['schemas']['TitleDetail'];
export type Episode = components['schemas']['Episode'];
export type Genre = components['schemas']['Genre'];

/** Browse filters. They mirror the URL query of the home page, so a view can be shared. */
export interface BrowseFilters {
  type?: 'movie' | 'show';
  genre?: string;
  language?: string;
  q?: string;
  sort?: 'recent' | 'az';
}

/** Read access to published titles. */
@Injectable({ providedIn: 'root' })
export class CatalogService {
  private readonly http = inject(HttpClient);

  browse(filters: BrowseFilters, cursor?: string | null): Observable<TitlePage> {
    let params = new HttpParams();
    for (const [key, value] of Object.entries(filters)) {
      if (typeof value === 'string' && value !== '') {
        params = params.set(key, value);
      }
    }
    if (cursor) {
      params = params.set('cursor', cursor);
    }
    return this.http.get<TitlePage>('/api/v1/titles', { params });
  }

  title(id: string): Observable<TitleDetail> {
    return this.http.get<TitleDetail>(`/api/v1/titles/${encodeURIComponent(id)}`);
  }

  episodes(id: string, season: number): Observable<Episode[]> {
    return this.http.get<Episode[]>(
      `/api/v1/titles/${encodeURIComponent(id)}/seasons/${season}/episodes`,
    );
  }

  genres(): Observable<Genre[]> {
    return this.http.get<Genre[]>('/api/v1/genres');
  }
}
