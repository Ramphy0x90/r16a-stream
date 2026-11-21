import { HttpClient, HttpParams } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { type Observable } from 'rxjs';

import { type components } from '../api/schema';

type S = components['schemas'];
export type AdminVariant = S['AdminVariant'];
export type TitleStorage = S['TitleStorage'];
export type UploadStarted = S['UploadStarted'];
export type UploadState = S['UploadState'];
export type PartUrls = S['PartUrls'];
export type CompletedPart = S['CompletedPart'];
export type JobRef = S['JobRef'];
export type AdminJob = S['AdminJob'];
export type AdminJobPage = S['AdminJobPage'];
export type StorageOverview = S['StorageOverview'];
export type TitleUsagePage = S['TitleUsagePage'];
export type ImageUploadStart = S['ImageUploadStart'];
export type ImageUploadStarted = S['ImageUploadStarted'];
type Created = S['Created'];

const id = encodeURIComponent;

/**
 * Variant, upload, subtitle, job and storage administration. The API refuses every call from
 * non-admins with 404. Part URLs are presigned and never logged or stored.
 */
@Injectable({ providedIn: 'root' })
export class AdminMediaService {
  private readonly http = inject(HttpClient);

  titleStorage(titleId: string): Observable<TitleStorage> {
    return this.http.get<TitleStorage>(`/api/v1/admin/titles/${id(titleId)}/storage`);
  }

  createVariant(titleId: string, language: string, episodeId?: string): Observable<Created> {
    return this.http.post<Created>('/api/v1/admin/variants', {
      titleId,
      language,
      ...(episodeId ? { episodeId } : {}),
    });
  }

  deleteVariant(variantId: string): Observable<null> {
    return this.http.delete<null>(`/api/v1/admin/variants/${id(variantId)}`);
  }

  retranscode(variantId: string): Observable<JobRef> {
    return this.http.post<JobRef>(`/api/v1/admin/variants/${id(variantId)}/retranscode`, null);
  }

  startUpload(
    variantId: string,
    fileName: string,
    sizeBytes: number,
    durationSeconds?: number,
  ): Observable<UploadStarted> {
    return this.http.post<UploadStarted>(`/api/v1/admin/variants/${id(variantId)}/uploads`, {
      fileName,
      sizeBytes,
      ...(durationSeconds ? { durationSeconds } : {}),
    });
  }

  upload(uploadId: string): Observable<UploadState> {
    return this.http.get<UploadState>(`/api/v1/admin/uploads/${id(uploadId)}`);
  }

  partUrls(uploadId: string, partNumbers: number[]): Observable<PartUrls> {
    return this.http.post<PartUrls>(`/api/v1/admin/uploads/${id(uploadId)}/parts`, { partNumbers });
  }

  complete(uploadId: string, parts: CompletedPart[]): Observable<JobRef> {
    return this.http.post<JobRef>(`/api/v1/admin/uploads/${id(uploadId)}/complete`, { parts });
  }

  abort(uploadId: string): Observable<null> {
    return this.http.delete<null>(`/api/v1/admin/uploads/${id(uploadId)}`);
  }

  startImage(input: ImageUploadStart): Observable<ImageUploadStarted> {
    return this.http.post<ImageUploadStarted>('/api/v1/admin/images', input);
  }

  addSubtitle(
    variantId: string,
    file: Blob,
    language: string,
    kind: string,
    label?: string,
  ): Observable<Created> {
    let params = new HttpParams().set('language', language).set('kind', kind);
    if (label?.trim()) params = params.set('label', label.trim());
    return this.http.post<Created>(`/api/v1/admin/variants/${id(variantId)}/subtitles`, file, {
      params,
      headers: { 'Content-Type': 'text/vtt' },
    });
  }

  deleteSubtitle(subtitleId: string): Observable<null> {
    return this.http.delete<null>(`/api/v1/admin/subtitles/${id(subtitleId)}`);
  }

  jobs(
    status: string | null,
    kind: string | null,
    cursor?: string | null,
  ): Observable<AdminJobPage> {
    let params = new HttpParams();
    if (status) params = params.set('status', status);
    if (kind) params = params.set('kind', kind);
    if (cursor) params = params.set('cursor', cursor);
    return this.http.get<AdminJobPage>('/api/v1/admin/jobs', { params });
  }

  retryJob(jobId: string): Observable<null> {
    return this.http.post<null>(`/api/v1/admin/jobs/${id(jobId)}/retry`, null);
  }

  cancelJob(jobId: string): Observable<null> {
    return this.http.post<null>(`/api/v1/admin/jobs/${id(jobId)}/cancel`, null);
  }

  storage(): Observable<StorageOverview> {
    return this.http.get<StorageOverview>('/api/v1/admin/storage');
  }

  storageTitles(cursor?: string | null): Observable<TitleUsagePage> {
    let params = new HttpParams();
    if (cursor) params = params.set('cursor', cursor);
    return this.http.get<TitleUsagePage>('/api/v1/admin/storage/titles', { params });
  }
}
