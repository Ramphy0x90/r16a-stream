import { HttpErrorResponse } from '@angular/common/http';
import { signal } from '@angular/core';
import { firstValueFrom } from 'rxjs';

import { type AdminMediaService, type CompletedPart } from './admin-media.service';

export type UploadPhase =
  'starting' | 'uploading' | 'paused' | 'completing' | 'done' | 'failed' | 'refused' | 'cancelled';

export interface UploadProgress {
  phase: UploadPhase;
  sentBytes: number;
  totalBytes: number;
  /** Translation key of the failure. */
  errorKey?: string;
  /** From a `507 insufficient_storage` refusal. */
  neededBytes?: number;
  availableBytes?: number;
  jobId?: string;
}

/** Sends one part. The presigned URL must never be logged or stored. */
export type PutPart = (
  url: string,
  body: Blob,
  signal: AbortSignal,
) => Promise<{ ok: boolean; status: number; etag: string | null }>;

/**
 * Uploads a part straight to Garage on the app's own origin. `credentials: 'omit'` keeps the
 * session cookie off the request, Traefik strips it as well (SPEC 10.3).
 */
export const fetchPart: PutPart = async (url, body, signal) => {
  const res = await fetch(url, {
    method: 'PUT',
    body,
    credentials: 'omit',
    cache: 'no-store',
    signal,
  });
  return { ok: res.ok, status: res.status, etag: res.headers.get('ETag') };
};

/** Reads the duration from the file's metadata when the browser can play the container. */
export function readDuration(file: Blob, timeoutMs = 5000): Promise<number | undefined> {
  return new Promise((resolve) => {
    const video = document.createElement('video');
    const url = URL.createObjectURL(file);
    const done = (value: number | undefined) => {
      clearTimeout(timer);
      URL.revokeObjectURL(url);
      video.removeAttribute('src');
      resolve(value);
    };
    const timer = setTimeout(() => done(undefined), timeoutMs);
    video.preload = 'metadata';
    video.onloadedmetadata = () =>
      done(
        Number.isFinite(video.duration) && video.duration > 0
          ? Math.round(video.duration)
          : undefined,
      );
    video.onerror = () => done(undefined);
    video.src = url;
  });
}

export interface UploaderOptions {
  put?: PutPart;
  parallel?: number;
  tries?: number;
  /** Waits between retries and while storage is degraded. Replaced in tests. */
  sleep?: (ms: number) => Promise<void>;
  duration?: (file: Blob) => Promise<number | undefined>;
  /** How long to wait before asking again while storage is degraded (SPEC 11.1). */
  pauseMs?: number;
}

const BATCH = 50;
const sleepFor = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

class Cancelled extends Error {}

/**
 * One source upload (SPEC 7.1). Parts go up four at a time, each retried with backoff. A
 * `503 storage_degraded` pauses the upload until storage writes again. Resuming asks the API
 * which parts Garage already holds and sends only the rest.
 */
export class MultipartUpload {
  readonly progress = signal<UploadProgress>({ phase: 'starting', sentBytes: 0, totalBytes: 0 });
  private readonly abort = new AbortController();
  private readonly put: PutPart;
  private readonly parallel: number;
  private readonly tries: number;
  private readonly sleep: (ms: number) => Promise<void>;
  private readonly duration: (file: Blob) => Promise<number | undefined>;
  private readonly pauseMs: number;

  constructor(
    private readonly api: AdminMediaService,
    private readonly file: File,
    options: UploaderOptions = {},
  ) {
    this.put = options.put ?? fetchPart;
    this.parallel = options.parallel ?? 4;
    this.tries = options.tries ?? 4;
    this.sleep = options.sleep ?? sleepFor;
    this.duration = options.duration ?? readDuration;
    this.pauseMs = options.pauseMs ?? 30_000;
  }

  /** Stops sending. The upload stays open on the server and can be resumed or aborted. */
  cancel(): void {
    this.abort.abort();
  }

  private set(p: Partial<UploadProgress>): void {
    this.progress.update((old) => ({ ...old, ...p }));
  }

  private check(): void {
    if (this.abort.signal.aborted) throw new Cancelled();
  }

  /** Calls the API, waiting out `503 storage_degraded` with the phase set to paused. */
  private async call<T>(make: () => Promise<T>, phase: UploadPhase): Promise<T> {
    for (;;) {
      this.check();
      try {
        const out = await make();
        this.set({ phase });
        return out;
      } catch (e) {
        if (e instanceof HttpErrorResponse && e.status === 503) {
          this.set({ phase: 'paused' });
          await this.sleep(this.pauseMs);
          continue;
        }
        throw e;
      }
    }
  }

  /** Uploads to `variantId`, or resumes `resumeId` when given. Resolves when it ended. */
  async run(variantId: string, resumeId?: string): Promise<void> {
    const total = this.file.size;
    this.set({ phase: 'starting', sentBytes: 0, totalBytes: total });
    try {
      const done = new Map<number, string>();
      let uploadId: string;
      let partSize: number;
      let partCount: number;
      if (resumeId) {
        const state = await firstValueFrom(this.api.upload(resumeId));
        if (state.status !== 'initiated' || state.sizeBytes !== total) {
          this.set({ phase: 'failed', errorKey: 'admin.media.resumeMismatch' });
          return;
        }
        uploadId = state.uploadId;
        partSize = state.partSize;
        partCount = state.partCount;
        for (const p of state.parts) {
          const expected =
            p.partNumber === partCount ? total - partSize * (partCount - 1) : partSize;
          if (p.sizeBytes === expected) done.set(p.partNumber, p.etag);
        }
      } else {
        const seconds = await this.duration(this.file);
        const started = await this.call(
          () => firstValueFrom(this.api.startUpload(variantId, this.file.name, total, seconds)),
          'uploading',
        );
        uploadId = started.uploadId;
        partSize = started.partSize;
        partCount = started.partCount;
      }
      const sizeOf = (n: number) =>
        n === partCount ? total - partSize * (partCount - 1) : partSize;
      let sent = [...done.keys()].reduce((sum, n) => sum + sizeOf(n), 0);
      this.set({ phase: 'uploading', sentBytes: sent });

      const missing = Array.from({ length: partCount }, (_, i) => i + 1).filter(
        (n) => !done.has(n),
      );
      for (let i = 0; i < missing.length; i += BATCH) {
        const batch = missing.slice(i, i + BATCH);
        const urls = await this.call(
          () => firstValueFrom(this.api.partUrls(uploadId, batch)),
          'uploading',
        );
        const queue = [...urls.parts];
        const workers = Array.from({ length: Math.min(this.parallel, queue.length) }, async () => {
          for (let next = queue.shift(); next; next = queue.shift()) {
            const n = next.partNumber;
            const blob = this.file.slice((n - 1) * partSize, (n - 1) * partSize + sizeOf(n));
            done.set(n, await this.sendPart(next.url, blob));
            sent += blob.size;
            this.set({ sentBytes: sent });
          }
        });
        await Promise.all(workers);
      }

      const parts: CompletedPart[] = [...done.entries()]
        .sort(([a], [b]) => a - b)
        .map(([partNumber, etag]) => ({ partNumber, etag }));
      this.set({ phase: 'completing' });
      const ref = await this.call(() => firstValueFrom(this.api.complete(uploadId, parts)), 'done');
      this.set({ phase: 'done', jobId: ref.jobId });
    } catch (e) {
      if (e instanceof Cancelled || this.abort.signal.aborted) {
        this.set({ phase: 'cancelled' });
      } else if (e instanceof HttpErrorResponse && e.status === 507) {
        const body = e.error as { neededBytes?: number; availableBytes?: number } | null;
        this.set({
          phase: 'refused',
          errorKey: 'admin.media.noSpace',
          neededBytes: body?.neededBytes,
          availableBytes: body?.availableBytes,
        });
      } else if (e instanceof HttpErrorResponse && e.status === 409) {
        this.set({ phase: 'failed', errorKey: 'admin.media.busy' });
      } else {
        this.set({ phase: 'failed', errorKey: 'admin.media.uploadFailed' });
      }
    }
  }

  /** One part with retries and growing waits. Returns the ETag Garage answered. */
  private async sendPart(url: string, blob: Blob): Promise<string> {
    for (let attempt = 1; ; attempt++) {
      this.check();
      try {
        const res = await this.put(url, blob, this.abort.signal);
        if (res.ok && res.etag) return res.etag;
        // A refused signature or a client error will not get better by retrying.
        if (res.status >= 400 && res.status < 500 && res.status !== 408 && res.status !== 429) {
          throw new Error('part refused');
        }
      } catch (e) {
        if (this.abort.signal.aborted) throw new Cancelled();
        if (e instanceof Error && e.message === 'part refused') throw e;
      }
      if (attempt >= this.tries) throw new Error('part failed');
      await this.sleep(1000 * 2 ** (attempt - 1));
    }
  }
}
