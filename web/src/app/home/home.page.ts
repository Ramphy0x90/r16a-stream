import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { takeUntilDestroyed, toObservable, toSignal } from '@angular/core/rxjs-interop';
import { Router } from '@angular/router';
import { TranslocoPipe } from '@jsverse/transloco';
import { catchError, map, of, switchMap } from 'rxjs';

import {
  type BrowseFilters,
  CatalogService,
  type TitleCard as Card,
} from '../core/catalog/catalog.service';
import { InViewDirective } from '../shared/in-view/in-view.directive';
import { TitleCard } from '../shared/title-card/title-card';

type LoadState = 'loading' | 'ready' | 'error';

/**
 * Home grid with filters and search. Every filter lives in the URL query (SPEC 5.1), so a view
 * can be shared and survives a reload. Pages load with keyset cursors as the list scrolls.
 */
@Component({
  selector: 'app-home-page',
  imports: [TranslocoPipe, TitleCard, InViewDirective],
  templateUrl: './home.page.html',
  styleUrl: './home.page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class HomePage {
  private readonly catalog = inject(CatalogService);
  private readonly router = inject(Router);

  /** Bound from the query string. */
  readonly type = input<string | undefined>();
  readonly genre = input<string | undefined>();
  readonly q = input<string | undefined>();
  readonly sort = input<string | undefined>();

  readonly genres = toSignal(this.catalog.genres().pipe(catchError(() => of([]))), {
    initialValue: [],
  });

  readonly filters = computed<BrowseFilters>(() => ({
    type:
      this.type() === 'movie' || this.type() === 'show'
        ? (this.type() as 'movie' | 'show')
        : undefined,
    genre: this.genre() || undefined,
    q: this.q()?.trim() || undefined,
    sort: this.sort() === 'az' ? 'az' : undefined,
  }));

  readonly items = signal<Card[]>([]);
  readonly state = signal<LoadState>('loading');
  private readonly cursor = signal<string | null>(null);
  private loadingMore = false;

  readonly hasMore = computed(() => this.cursor() !== null);

  constructor() {
    toObservable(this.filters)
      .pipe(
        switchMap((f) => {
          this.state.set('loading');
          return this.catalog.browse(f).pipe(
            map((page) => ({ page, ok: true as const })),
            catchError(() => of({ page: null, ok: false as const })),
          );
        }),
        takeUntilDestroyed(),
      )
      .subscribe((r) => {
        this.items.set(r.page?.items ?? []);
        this.cursor.set(r.page?.nextCursor ?? null);
        this.state.set(r.ok ? 'ready' : 'error');
      });
  }

  loadMore(): void {
    const cursor = this.cursor();
    if (!cursor || this.loadingMore || this.state() !== 'ready') {
      return;
    }
    this.loadingMore = true;
    this.catalog.browse(this.filters(), cursor).subscribe({
      next: (page) => {
        this.items.update((list) => [...list, ...page.items]);
        this.cursor.set(page.nextCursor ?? null);
        this.loadingMore = false;
      },
      error: () => {
        this.loadingMore = false;
      },
    });
  }

  /** Writes one filter into the URL. The query binding then reloads the grid. */
  setFilter(key: 'type' | 'genre' | 'q' | 'sort', value: string): void {
    void this.router.navigate([], {
      queryParams: { [key]: value === '' ? null : value },
      queryParamsHandling: 'merge',
    });
  }

  search(event: Event): void {
    event.preventDefault();
    const form = event.target as HTMLFormElement;
    const value = (new FormData(form).get('q') ?? '').toString();
    this.setFilter('q', value.trim());
  }

  valueOf(event: Event): string {
    return (event.target as HTMLSelectElement).value;
  }
}
