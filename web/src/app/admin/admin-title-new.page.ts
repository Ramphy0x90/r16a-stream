import { HttpErrorResponse } from '@angular/common/http';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import { TranslocoPipe } from '@jsverse/transloco';

import { AdminCatalogService, type TitleInput } from '../core/catalog/admin-catalog.service';

/** Maps an API problem to the translation key shown to the admin. */
export function problemKey(e: unknown): string {
  if (e instanceof HttpErrorResponse) {
    if (e.status === 404) return 'admin.error.notFound';
    if (e.status === 409) return 'admin.error.conflict';
    if (e.status === 400) return 'admin.error.invalid';
    if (e.status === 429) return 'admin.error.tooMany';
    if (e.status === 503) return 'admin.error.unavailable';
  }
  return 'catalog.error';
}

const EMPTY: TitleInput = { kind: 'movie', name: '' };

/**
 * Creates a title. A TMDB lookup prefills the draft, the admin edits it, and the whole draft is
 * saved in one request, seasons and episodes included. Images follow in Phase 3b.
 */
@Component({
  selector: 'app-admin-title-new-page',
  imports: [TranslocoPipe],
  templateUrl: './admin-title-new.page.html',
  styleUrl: './admin.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AdminTitleNewPage {
  private readonly admin = inject(AdminCatalogService);
  private readonly router = inject(Router);

  readonly draft = signal<TitleInput>({ ...EMPTY });
  readonly busy = signal(false);
  readonly error = signal<string | null>(null);

  readonly episodeCount = computed(() =>
    (this.draft().seasons ?? []).reduce((n, s) => n + (s.episodes?.length ?? 0), 0),
  );

  lookup(event: Event): void {
    event.preventDefault();
    const data = new FormData(event.target as HTMLFormElement);
    const ref = (data.get('ref') ?? '').toString().trim();
    const kind = (data.get('kind') ?? '').toString();
    if (!ref) return;
    this.busy.set(true);
    this.error.set(null);
    this.admin
      .lookup(ref, kind === 'show' ? 'show' : kind === 'movie' ? 'movie' : undefined)
      .subscribe({
        next: (d) => {
          this.draft.set(d);
          this.busy.set(false);
        },
        error: (e: unknown) => {
          this.error.set(problemKey(e));
          this.busy.set(false);
        },
      });
  }

  /** Updates one text or number field of the draft. Empty optional fields become null. */
  set(
    field: 'name' | 'originalName' | 'overview' | 'year' | 'runtimeMinutes' | 'kind',
    event: Event,
  ): void {
    const raw = (event.target as HTMLInputElement).value;
    this.draft.update((d) => {
      switch (field) {
        case 'year':
        case 'runtimeMinutes':
          return { ...d, [field]: raw === '' ? null : Number(raw) };
        case 'kind':
          return {
            ...d,
            kind: raw === 'show' ? 'show' : 'movie',
            runtimeMinutes: raw === 'show' ? null : d.runtimeMinutes,
          };
        case 'name':
          return { ...d, name: raw };
        default:
          return { ...d, [field]: raw === '' ? null : raw };
      }
    });
  }

  /** Keeps a TMDB image out of the save. */
  dropImage(field: 'posterPath' | 'backdropPath'): void {
    this.draft.update((d) => ({ ...d, [field]: null }));
  }

  removeGenre(index: number): void {
    this.draft.update((d) => ({
      ...d,
      newGenres: (d.newGenres ?? []).filter((_, i) => i !== index),
    }));
  }

  removeCredit(index: number): void {
    this.draft.update((d) => ({ ...d, credits: (d.credits ?? []).filter((_, i) => i !== index) }));
  }

  save(): void {
    this.busy.set(true);
    this.error.set(null);
    this.admin.create(this.draft()).subscribe({
      next: (c) => void this.router.navigate(['/admin/titles', c.id]),
      error: (e: unknown) => {
        this.error.set(problemKey(e));
        this.busy.set(false);
      },
    });
  }
}
