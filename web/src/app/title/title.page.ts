import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { takeUntilDestroyed, toObservable } from '@angular/core/rxjs-interop';
import { TranslocoPipe } from '@jsverse/transloco';
import { HttpErrorResponse } from '@angular/common/http';
import { catchError, map, of, switchMap } from 'rxjs';

import { CatalogService, type Episode, type TitleDetail } from '../core/catalog/catalog.service';
import { Picture } from '../shared/picture/picture';

type Loaded =
  | { kind: 'ok'; title: TitleDetail }
  | { kind: 'missing' }
  | { kind: 'error' }
  | { kind: 'loading' };

/** The title page. Shows load their episodes per season when the season is picked. */
@Component({
  selector: 'app-title-page',
  imports: [TranslocoPipe, Picture],
  templateUrl: './title.page.html',
  styleUrl: './title.page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TitlePage {
  private readonly catalog = inject(CatalogService);

  /** Bound from the route. */
  readonly id = input.required<string>();

  readonly loaded = signal<Loaded>({ kind: 'loading' });
  readonly season = signal<number | null>(null);
  readonly episodes = signal<Episode[] | null>(null);

  readonly people = computed(() => {
    const l = this.loaded();
    if (l.kind !== 'ok') return { leads: [], cast: [] };
    return {
      leads: l.title.credits.filter((c) => c.role !== 'cast'),
      cast: l.title.credits.filter((c) => c.role === 'cast').slice(0, 12),
    };
  });

  constructor() {
    toObservable(this.id)
      .pipe(
        switchMap((id) =>
          this.catalog.title(id).pipe(
            map((title): Loaded => ({ kind: 'ok', title })),
            catchError((e: unknown) =>
              of<Loaded>(
                e instanceof HttpErrorResponse && e.status === 404
                  ? { kind: 'missing' }
                  : { kind: 'error' },
              ),
            ),
          ),
        ),
        takeUntilDestroyed(),
      )
      .subscribe((l) => {
        this.loaded.set(l);
        const first =
          l.kind === 'ok'
            ? (l.title.seasons.find((s) => s.number > 0) ?? l.title.seasons[0])
            : undefined;
        this.episodes.set(null);
        this.season.set(null);
        if (first) this.pickSeason(first.number);
      });
  }

  pickSeason(number: number): void {
    this.season.set(number);
    this.episodes.set(null);
    this.catalog.episodes(this.id(), number).subscribe({
      next: (list) => {
        if (this.season() === number) this.episodes.set(list);
      },
      error: () => this.episodes.set([]),
    });
  }
}
