import { HttpErrorResponse } from '@angular/common/http';
import { type Observable, of, throwError } from 'rxjs';

import { type AdminMediaService, type CompletedPart } from './admin-media.service';
import { MultipartUpload, type PutPart } from './uploader';

interface Calls {
  started: unknown[][];
  urls: number[][];
  completed: CompletedPart[][];
  puts: string[];
}

/** A fake API with 4 byte parts, so a 10 byte file has three. */
function fakeApi(over: Partial<Record<keyof AdminMediaService, unknown>> = {}) {
  const calls: Calls = { started: [], urls: [], completed: [], puts: [] };
  const api = {
    startUpload: (...args: unknown[]) => {
      calls.started.push(args);
      return of({ uploadId: 'u1', partSize: 4, partCount: 3 });
    },
    partUrls: (_id: string, numbers: number[]) => {
      calls.urls.push(numbers);
      return of({ parts: numbers.map((n) => ({ partNumber: n, url: `https://app/part/${n}` })) });
    },
    complete: (_id: string, parts: CompletedPart[]) => {
      calls.completed.push(parts);
      return of({ jobId: 'job-1' });
    },
    upload: () =>
      of({
        uploadId: 'u1',
        status: 'initiated',
        sizeBytes: 10,
        partSize: 4,
        partCount: 3,
        parts: [{ partNumber: 2, etag: '"e2"', sizeBytes: 4 }],
      }),
    ...over,
  } as unknown as AdminMediaService;
  return { api, calls };
}

const file = () => new File([new Uint8Array(10)], 'film.mkv');
const noWait = { sleep: () => Promise.resolve(), duration: () => Promise.resolve(3600) };

function putOk(calls: Calls): PutPart {
  return (url) => {
    calls.puts.push(url);
    return Promise.resolve({ ok: true, status: 200, etag: `"e${url.split('/').pop()}"` });
  };
}

describe('MultipartUpload', () => {
  it('sends every part and completes them in order with their tags', async () => {
    const { api, calls } = fakeApi();
    const up = new MultipartUpload(api, file(), { ...noWait, put: putOk(calls) });
    await up.run('v1');
    expect(calls.started[0]).toEqual(['v1', 'film.mkv', 10, 3600]);
    expect(calls.puts.length).toBe(3);
    expect(calls.completed[0]).toEqual([
      { partNumber: 1, etag: '"e1"' },
      { partNumber: 2, etag: '"e2"' },
      { partNumber: 3, etag: '"e3"' },
    ]);
    expect(up.progress()).toEqual(
      expect.objectContaining({ phase: 'done', sentBytes: 10, totalBytes: 10, jobId: 'job-1' }),
    );
  });

  it('resumes with only the parts Garage does not hold', async () => {
    const { api, calls } = fakeApi();
    const up = new MultipartUpload(api, file(), { ...noWait, put: putOk(calls) });
    await up.run('v1', 'u1');
    expect(calls.started.length).toBe(0);
    expect(calls.urls).toEqual([[1, 3]]);
    expect(calls.completed[0]?.map((p) => p.etag)).toEqual(['"e1"', '"e2"', '"e3"']);
  });

  it('refuses to resume with a different file', async () => {
    const { api } = fakeApi();
    const up = new MultipartUpload(api, new File([new Uint8Array(11)], 'other.mkv'), noWait);
    await up.run('v1', 'u1');
    expect(up.progress().phase).toBe('failed');
    expect(up.progress().errorKey).toBe('admin.media.resumeMismatch');
  });

  it('retries a failed part and gives up after the last try', async () => {
    const { api, calls } = fakeApi();
    let failures = 2;
    const flaky: PutPart = (url) => {
      calls.puts.push(url);
      if (url.endsWith('/2') && failures-- > 0) return Promise.reject(new TypeError('network'));
      return Promise.resolve({ ok: true, status: 200, etag: '"x"' });
    };
    const up = new MultipartUpload(api, file(), { ...noWait, put: flaky });
    await up.run('v1');
    expect(up.progress().phase).toBe('done');
    expect(calls.puts.filter((u) => u.endsWith('/2')).length).toBe(3);

    const dead: PutPart = () => Promise.resolve({ ok: false, status: 500, etag: null });
    const failing = new MultipartUpload(fakeApi().api, file(), { ...noWait, put: dead, tries: 2 });
    await failing.run('v1');
    expect(failing.progress().phase).toBe('failed');
  });

  it('shows the numbers of a 507 refusal', async () => {
    const refused = new HttpErrorResponse({
      status: 507,
      error: { neededBytes: 9_000, availableBytes: 100 },
    });
    const { api } = fakeApi({ startUpload: (): Observable<never> => throwError(() => refused) });
    const up = new MultipartUpload(api, file(), noWait);
    await up.run('v1');
    expect(up.progress()).toEqual(
      expect.objectContaining({ phase: 'refused', neededBytes: 9_000, availableBytes: 100 }),
    );
  });

  it('waits while storage is degraded and carries on afterwards', async () => {
    let degraded = 2;
    const phases: string[] = [];
    const { api, calls } = fakeApi({
      partUrls: (_id: string, numbers: number[]) =>
        degraded-- > 0
          ? throwError(() => new HttpErrorResponse({ status: 503 }))
          : of({ parts: numbers.map((n) => ({ partNumber: n, url: `https://app/part/${n}` })) }),
    });
    const up = new MultipartUpload(api, file(), {
      ...noWait,
      put: putOk(calls),
      sleep: () => {
        phases.push(up.progress().phase);
        return Promise.resolve();
      },
    });
    await up.run('v1');
    expect(phases).toEqual(['paused', 'paused']);
    expect(up.progress().phase).toBe('done');
  });

  it('stops when cancelled', async () => {
    const { api, calls } = fakeApi();
    const holder: { up?: MultipartUpload } = {};
    const put: PutPart = (url) => {
      holder.up?.cancel();
      return putOk(calls)(url, new Blob(), new AbortController().signal);
    };
    const up = new MultipartUpload(api, file(), { ...noWait, put, parallel: 1 });
    holder.up = up;
    await up.run('v1');
    expect(up.progress().phase).toBe('cancelled');
    expect(calls.completed.length).toBe(0);
  });
});
