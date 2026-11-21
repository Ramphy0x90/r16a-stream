import { ChangeDetectionStrategy, Component, inject, input, signal } from '@angular/core';
import { takeUntilDestroyed, toObservable } from '@angular/core/rxjs-interop';
import { Router } from '@angular/router';
import { TranslocoPipe, TranslocoService } from '@jsverse/transloco';
import { combineLatest, switchMap } from 'rxjs';

import { AdminMediaService, type AdminJob } from '../core/media/admin-media.service';
import { InViewDirective } from '../shared/in-view/in-view.directive';
import { problemKey } from './admin-title-new.page';

export const JOB_STATUSES = [
  'queued',
  'running',
  'cancelling',
  'succeeded',
  'failed',
  'cancelled',
] as const;
export const JOB_KINDS = ['transcode', 'image', 'subtitle', 'encode_gc'] as const;

/** The job queue with its progress, failure reasons and controls (SPEC 7.2). Filters live in the URL. */
@Component({
  selector: 'app-admin-jobs-page',
  imports: [TranslocoPipe, InViewDirective],
  templateUrl: './admin-jobs.page.html',
  styleUrl: './admin.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AdminJobsPage {
  private readonly api = inject(AdminMediaService);
  private readonly router = inject(Router);
  private readonly i18n = inject(TranslocoService);

  /** Bound from the query string. Unknown values are ignored. */
  readonly status = input<string>();
  readonly kind = input<string>();

  readonly statuses = JOB_STATUSES;
  readonly kinds = JOB_KINDS;
  readonly items = signal<AdminJob[]>([]);
  readonly next = signal<string | null>(null);
  readonly error = signal<string | null>(null);
  readonly loading = signal(false);

  private filter(value: string | undefined, allowed: readonly string[]): string | null {
    return value && allowed.includes(value) ? value : null;
  }

  readonly statusFilter = () => this.filter(this.status(), JOB_STATUSES);
  readonly kindFilter = () => this.filter(this.kind(), JOB_KINDS);

  constructor() {
    combineLatest([toObservable(this.status), toObservable(this.kind)])
      .pipe(
        switchMap(() => {
          this.loading.set(true);
          return this.api.jobs(this.statusFilter(), this.kindFilter());
        }),
        takeUntilDestroyed(),
      )
      .subscribe({
        next: (page) => {
          this.items.set(page.items);
          this.next.set(page.nextCursor ?? null);
          this.loading.set(false);
        },
        error: (e: unknown) => {
          this.error.set(problemKey(e));
          this.loading.set(false);
        },
      });
  }

  setFilter(name: 'status' | 'kind', event: Event): void {
    const value = (event.target as HTMLSelectElement).value || null;
    void this.router.navigate([], { queryParams: { [name]: value }, queryParamsHandling: 'merge' });
  }

  loadMore(): void {
    const cursor = this.next();
    if (!cursor || this.loading()) return;
    this.loading.set(true);
    this.api.jobs(this.statusFilter(), this.kindFilter(), cursor).subscribe({
      next: (page) => {
        this.items.update((list) => [...list, ...page.items]);
        this.next.set(page.nextCursor ?? null);
        this.loading.set(false);
      },
      error: (e: unknown) => {
        this.error.set(problemKey(e));
        this.loading.set(false);
      },
    });
  }

  date(iso: string | null | undefined): string {
    return iso
      ? new Intl.DateTimeFormat(this.i18n.getActiveLang(), {
          dateStyle: 'short',
          timeStyle: 'short',
        }).format(new Date(iso))
      : '';
  }

  percent(v: number | null | undefined): number {
    return Math.round((v ?? 0) * 100);
  }

  private replace(job: AdminJob): void {
    this.api.jobs(this.statusFilter(), this.kindFilter()).subscribe((page) => {
      const fresh = page.items.find((j) => j.id === job.id);
      this.items.update((list) => list.map((j) => (j.id === job.id ? (fresh ?? j) : j)));
    });
  }

  retry(job: AdminJob): void {
    this.error.set(null);
    this.api.retryJob(job.id).subscribe({
      next: () => this.replace(job),
      error: (e: unknown) => this.error.set(problemKey(e)),
    });
  }

  cancel(job: AdminJob): void {
    this.error.set(null);
    this.api.cancelJob(job.id).subscribe({
      next: () => this.replace(job),
      error: (e: unknown) => this.error.set(problemKey(e)),
    });
  }
}
