import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
} from '@angular/core';
import { TranslocoPipe, TranslocoService } from '@jsverse/transloco';
import { firstValueFrom } from 'rxjs';

import { type AdminTitle } from '../../core/catalog/admin-catalog.service';
import { AdminMediaService, type AdminVariant } from '../../core/media/admin-media.service';
import { MultipartUpload, fetchPart } from '../../core/media/uploader';
import { problemKey } from '../admin-title-new.page';
import { formatBytes } from './bytes';

/** A group of variants, the movie itself or one episode. */
interface Group {
  episodeId: string | null;
  label: string;
  variants: AdminVariant[];
}

const POLL_MS = 10_000;

/**
 * Languages, uploads, transcodes, subtitles and manual images of one title (SPEC 7.1, 11.1).
 * Admin only, web only.
 */
@Component({
  selector: 'app-media-panel',
  imports: [TranslocoPipe],
  templateUrl: './media-panel.html',
  styleUrl: '../admin.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class MediaPanel {
  private readonly api = inject(AdminMediaService);
  private readonly i18n = inject(TranslocoService);

  readonly title = input.required<AdminTitle>();
  /** Emitted after an image upload, so the page reloads the title. */
  readonly changed = output();

  readonly variants = signal<AdminVariant[]>([]);
  readonly uploads = signal<ReadonlyMap<string, MultipartUpload>>(new Map());
  readonly error = signal<string | null>(null);
  readonly busy = signal(false);
  readonly newLanguage = signal('');
  readonly newEpisode = signal('');
  readonly imageStatus = signal<string | null>(null);

  readonly episodes = computed(() =>
    this.title().seasons.flatMap((s) =>
      s.episodes.map((e) => ({
        id: e.id,
        label: `S${s.number} E${e.number}${e.name ? ` · ${e.name}` : ''}`,
      })),
    ),
  );

  readonly groups = computed<Group[]>(() => {
    const list = this.variants();
    if (this.title().kind === 'movie') {
      return [{ episodeId: null, label: '', variants: list }];
    }
    return this.episodes()
      .map((e) => ({
        episodeId: e.id,
        label: e.label,
        variants: list.filter((v) => v.episodeId === e.id),
      }))
      .filter((g) => g.variants.length > 0);
  });

  private readonly active = computed(() =>
    this.variants().some(
      (v) => v.job && ['queued', 'running', 'cancelling'].includes(v.job.status),
    ),
  );

  constructor() {
    effect(() => {
      this.title();
      this.load();
    });
    const timer = setInterval(() => {
      if (this.active()) this.load();
    }, POLL_MS);
    inject(DestroyRef).onDestroy(() => {
      clearInterval(timer);
      for (const u of this.uploads().values()) u.cancel();
    });
  }

  bytes(n: number | null | undefined): string {
    return formatBytes(n, this.i18n.getActiveLang());
  }

  percent(v: number | null | undefined): number {
    return Math.round((v ?? 0) * 100);
  }

  load(): void {
    this.api.titleStorage(this.title().id).subscribe({
      next: (s) => this.variants.set(s.variants),
      error: (e: unknown) => this.error.set(problemKey(e)),
    });
  }

  private run(call: Promise<unknown>): void {
    this.busy.set(true);
    this.error.set(null);
    call
      .then(() => this.load())
      .catch((e: unknown) => this.error.set(problemKey(e)))
      .finally(() => this.busy.set(false));
  }

  value(event: Event): string {
    return (event.target as HTMLInputElement | HTMLSelectElement).value;
  }

  addVariant(): void {
    const language = this.newLanguage().trim();
    if (!language) return;
    const episode =
      this.title().kind === 'show' ? this.newEpisode() || this.episodes()[0]?.id : undefined;
    if (this.title().kind === 'show' && !episode) return;
    this.newLanguage.set('');
    this.run(firstValueFrom(this.api.createVariant(this.title().id, language, episode)));
  }

  removeVariant(v: AdminVariant): void {
    if (!globalThis.confirm(this.i18n.translate('admin.confirmDelete'))) return;
    this.run(firstValueFrom(this.api.deleteVariant(v.id)));
  }

  retranscode(v: AdminVariant): void {
    this.run(firstValueFrom(this.api.retranscode(v.id)));
  }

  cancelJob(v: AdminVariant): void {
    if (v.job) this.run(firstValueFrom(this.api.cancelJob(v.job.id)));
  }

  /** Starts a new upload, or resumes the open one with the same file picked again. */
  upload(v: AdminVariant, event: Event): void {
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    const up = new MultipartUpload(this.api, file);
    this.uploads.update((m) => new Map(m).set(v.id, up));
    void up.run(v.id, v.openUploadId ?? undefined).then(() => this.load());
  }

  stopUpload(v: AdminVariant): void {
    this.uploads().get(v.id)?.cancel();
  }

  abortUpload(v: AdminVariant): void {
    if (!v.openUploadId) return;
    this.uploads().get(v.id)?.cancel();
    this.uploads.update((m) => {
      const next = new Map(m);
      next.delete(v.id);
      return next;
    });
    this.run(firstValueFrom(this.api.abort(v.openUploadId)));
  }

  /** Adds a WebVTT track. Language and kind come from the selects next to the file input. */
  subtitle(v: AdminVariant, language: string, kind: string, event: Event): void {
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file || !language.trim()) return;
    this.run(firstValueFrom(this.api.addSubtitle(v.id, file, language.trim(), kind)));
  }

  /** Uploads a manual poster, backdrop or episode still as one part, then completes it. */
  image(kind: 'poster' | 'backdrop' | 'still', event: Event, episodeId?: string): void {
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    this.imageStatus.set('admin.media.imageUploading');
    const task = async () => {
      const started = await firstValueFrom(
        this.api.startImage({
          kind,
          sizeBytes: file.size,
          contentType: file.type,
          ...(kind === 'still' ? { episodeId } : { titleId: this.title().id }),
        }),
      );
      const res = await fetchPart(started.partUrl, file, new AbortController().signal);
      if (!res.ok || !res.etag) throw new Error('image part failed');
      await firstValueFrom(
        this.api.complete(started.uploadId, [{ partNumber: 1, etag: res.etag }]),
      );
    };
    task()
      .then(() => {
        this.imageStatus.set('admin.media.imageQueued');
        this.changed.emit();
      })
      .catch((e: unknown) => {
        this.imageStatus.set(null);
        this.error.set(problemKey(e));
      });
  }
}
