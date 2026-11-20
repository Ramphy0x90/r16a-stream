import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { TranslocoPipe } from '@jsverse/transloco';

import { AdminCatalogService, type AdminTitlePage } from '../core/catalog/admin-catalog.service';

type Row = AdminTitlePage['items'][number];

/** Every title, drafts first among recent changes, with status filter and name search. */
@Component({
  selector: 'app-admin-titles-page',
  imports: [RouterLink, TranslocoPipe],
  templateUrl: './admin-titles.page.html',
  styleUrl: './admin.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AdminTitlesPage {
  private readonly admin = inject(AdminCatalogService);

  readonly rows = signal<Row[]>([]);
  readonly next = signal<string | null>(null);
  readonly failed = signal(false);
  readonly status = signal<string>('');
  readonly query = signal('');

  constructor() {
    this.reload();
  }

  reload(): void {
    this.failed.set(false);
    this.admin.list(this.status() || null, this.query()).subscribe({
      next: (p) => {
        this.rows.set(p.items);
        this.next.set(p.nextCursor ?? null);
      },
      error: () => this.failed.set(true),
    });
  }

  more(): void {
    const c = this.next();
    if (!c) return;
    this.admin.list(this.status() || null, this.query(), c).subscribe({
      next: (p) => {
        this.rows.update((r) => [...r, ...p.items]);
        this.next.set(p.nextCursor ?? null);
      },
      error: () => this.failed.set(true),
    });
  }

  setStatus(event: Event): void {
    this.status.set((event.target as HTMLSelectElement).value);
    this.reload();
  }

  search(event: Event): void {
    event.preventDefault();
    this.query.set((new FormData(event.target as HTMLFormElement).get('q') ?? '').toString());
    this.reload();
  }
}
