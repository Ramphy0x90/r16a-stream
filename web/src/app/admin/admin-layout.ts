import { ChangeDetectionStrategy, Component, DestroyRef, inject, signal } from '@angular/core';
import { RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
import { TranslocoPipe } from '@jsverse/transloco';

import { AdminMediaService, type StorageOverview } from '../core/media/admin-media.service';

/** How often the banner state is refreshed (SPEC 11.1). */
export const STORAGE_POLL_MS = 60_000;

/**
 * Frame of every admin page, with the admin navigation and the storage banners, "storage
 * degraded" while writes fail, "almost full" below the warning threshold, and "capacity
 * unknown" until the metrics work (SPEC 7.10, 11.1).
 */
@Component({
  selector: 'app-admin-layout',
  imports: [RouterOutlet, RouterLink, RouterLinkActive, TranslocoPipe],
  template: `
    <nav class="subnav" [attr.aria-label]="'shell.admin' | transloco">
      <a
        routerLink="/admin"
        routerLinkActive="active"
        [routerLinkActiveOptions]="{ exact: true }"
        >{{ 'admin.titles.heading' | transloco }}</a
      >
      <a routerLink="/admin/jobs" routerLinkActive="active">{{
        'admin.jobs.heading' | transloco
      }}</a>
      <a routerLink="/admin/storage" routerLinkActive="active">{{
        'admin.storage.heading' | transloco
      }}</a>
    </nav>
    @if (storage(); as s) {
      @if (s.degraded) {
        <p class="banner" role="alert">{{ 'admin.banner.degraded' | transloco }}</p>
      }
      @if (s.warning && s.capacityKnown) {
        <p class="banner" role="alert">{{ 'admin.banner.full' | transloco }}</p>
      }
      @if (!s.capacityKnown) {
        <p class="banner" role="status">{{ 'admin.banner.unknown' | transloco }}</p>
      }
    }
    <router-outlet />
  `,
  styleUrl: './admin-layout.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AdminLayout {
  private readonly api = inject(AdminMediaService);
  readonly storage = signal<StorageOverview | null>(null);

  constructor() {
    this.refresh();
    const timer = setInterval(() => this.refresh(), STORAGE_POLL_MS);
    inject(DestroyRef).onDestroy(() => clearInterval(timer));
  }

  private refresh(): void {
    // A failed poll keeps the last state, the page itself shows its own errors.
    this.api.storage().subscribe({ next: (s) => this.storage.set(s), error: () => undefined });
  }
}
