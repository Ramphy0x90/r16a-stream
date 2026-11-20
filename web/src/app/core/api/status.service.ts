import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { type Observable } from 'rxjs';

import { type components } from './schema';

export type StatusResponse = components['schemas']['StatusResponse'];

/** Reachability of the API through the same origin. */
@Injectable({ providedIn: 'root' })
export class StatusService {
  private readonly http = inject(HttpClient);

  status(): Observable<StatusResponse> {
    return this.http.get<StatusResponse>('/api/v1/status');
  }
}
