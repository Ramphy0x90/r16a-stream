import { ChangeDetectionStrategy, Component, inject, input, signal } from '@angular/core';
import { takeUntilDestroyed, toObservable } from '@angular/core/rxjs-interop';
import { Router, RouterLink } from '@angular/router';
import { TranslocoPipe, TranslocoService } from '@jsverse/transloco';
import { type Observable, switchMap } from 'rxjs';

import {
  AdminCatalogService,
  type AdminTitle,
  type TitlePatch,
} from '../core/catalog/admin-catalog.service';
import { Picture } from '../shared/picture/picture';
import { problemKey } from './admin-title-new.page';

type Editable = 'name' | 'originalName' | 'overview' | 'year' | 'runtimeMinutes';

/** Edits one title, its publication, and its seasons and episodes. */
@Component({
  selector: 'app-admin-title-edit-page',
  imports: [RouterLink, TranslocoPipe, Picture],
  templateUrl: './admin-title-edit.page.html',
  styleUrl: './admin.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AdminTitleEditPage {
  private readonly admin = inject(AdminCatalogService);
  private readonly router = inject(Router);
  private readonly i18n = inject(TranslocoService);

  /** Bound from the route. */
  readonly id = input.required<string>();

  readonly title = signal<AdminTitle | null>(null);
  /** Changed fields only, sent as one PATCH. */
  readonly patch = signal<TitlePatch>({});
  readonly busy = signal(false);
  readonly error = signal<string | null>(null);
  readonly saved = signal(false);

  constructor() {
    toObservable(this.id)
      .pipe(
        switchMap((id) => this.admin.get(id)),
        takeUntilDestroyed(),
      )
      .subscribe({
        next: (t) => this.title.set(t),
        error: (e: unknown) => this.error.set(problemKey(e)),
      });
  }

  private reload(): void {
    this.admin.get(this.id()).subscribe({
      next: (t) => {
        this.title.set(t);
        this.busy.set(false);
      },
      error: (e: unknown) => {
        this.error.set(problemKey(e));
        this.busy.set(false);
      },
    });
  }

  /** Runs one change, then reloads the title so the page shows what the server stored. */
  private run(call: Observable<unknown>): void {
    this.busy.set(true);
    this.error.set(null);
    this.saved.set(false);
    call.subscribe({
      next: () => {
        this.saved.set(true);
        this.reload();
      },
      error: (e: unknown) => {
        this.error.set(problemKey(e));
        this.busy.set(false);
      },
    });
  }

  set(field: Editable, event: Event): void {
    const raw = (event.target as HTMLInputElement).value;
    const value =
      field === 'year' || field === 'runtimeMinutes'
        ? raw === ''
          ? null
          : Number(raw)
        : raw === '' && field !== 'name'
          ? null
          : raw;
    this.patch.update((p) => ({ ...p, [field]: value }));
  }

  save(): void {
    const p = this.patch();
    if (Object.keys(p).length === 0) return;
    this.patch.set({});
    this.run(this.admin.update(this.id(), p));
  }

  /** Detaches the poster or backdrop from the title. */
  dropImage(field: 'posterPath' | 'backdropPath'): void {
    this.run(this.admin.update(this.id(), { [field]: null }));
  }

  setPublished(published: boolean): void {
    this.run(this.admin.setPublished(this.id(), published));
  }

  private confirmed(): boolean {
    return globalThis.confirm(this.i18n.translate('admin.confirmDelete'));
  }

  /** Deletes the title after a confirmation. */
  confirmRemove(): void {
    if (!this.confirmed()) return;
    this.busy.set(true);
    this.admin.remove(this.id()).subscribe({
      next: () => void this.router.navigateByUrl('/admin'),
      error: (e: unknown) => {
        this.error.set(problemKey(e));
        this.busy.set(false);
      },
    });
  }

  addSeason(event: Event): void {
    event.preventDefault();
    const data = new FormData(event.target as HTMLFormElement);
    const number = Number(data.get('number'));
    const name = (data.get('name') ?? '').toString().trim();
    this.run(this.admin.addSeason(this.id(), { number, name: name || null }));
  }

  removeSeason(id: string): void {
    if (!this.confirmed()) return;
    this.run(this.admin.removeSeason(id));
  }

  addEpisode(seasonId: string, event: Event): void {
    event.preventDefault();
    const data = new FormData(event.target as HTMLFormElement);
    const number = Number(data.get('number'));
    const name = (data.get('name') ?? '').toString().trim();
    this.run(this.admin.addEpisode(seasonId, { number, name: name || null }));
  }

  removeEpisode(id: string): void {
    this.run(this.admin.removeEpisode(id));
  }
}
