import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { TranslocoPipe, TranslocoService } from '@jsverse/transloco';

import {
  AdminMediaService,
  type StorageOverview,
  type TitleUsagePage,
} from '../core/media/admin-media.service';
import { problemKey } from './admin-title-new.page';
import { formatBytes } from './media/bytes';

/** Garage capacity, reservations, write health and the largest titles (SPEC 7.8, 7.10). */
@Component({
  selector: 'app-admin-storage-page',
  imports: [TranslocoPipe, RouterLink],
  templateUrl: './admin-storage.page.html',
  styleUrl: './admin.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AdminStoragePage {
  private readonly api = inject(AdminMediaService);
  private readonly i18n = inject(TranslocoService);

  readonly overview = signal<StorageOverview | null>(null);
  readonly titles = signal<TitleUsagePage['items']>([]);
  readonly next = signal<string | null>(null);
  readonly error = signal<string | null>(null);

  constructor() {
    this.api.storage().subscribe({
      next: (o) => this.overview.set(o),
      error: (e: unknown) => this.error.set(problemKey(e)),
    });
    this.more();
  }

  more(): void {
    this.api.storageTitles(this.next()).subscribe({
      next: (page) => {
        this.titles.update((list) => [...list, ...page.items]);
        this.next.set(page.nextCursor ?? null);
      },
      error: (e: unknown) => this.error.set(problemKey(e)),
    });
  }

  bytes(n: number | null | undefined): string {
    return formatBytes(n, this.i18n.getActiveLang());
  }
}
