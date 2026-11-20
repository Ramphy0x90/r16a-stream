import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { TranslocoPipe } from '@jsverse/transloco';
import { catchError, map, of, startWith } from 'rxjs';

import { StatusService } from '../core/api/status.service';

type ApiState = 'checking' | 'reachable' | 'unreachable';

/** Phase 1 placeholder for the home grid. Proves web, API and proxy share one origin. */
@Component({
  selector: 'app-home-page',
  imports: [TranslocoPipe],
  templateUrl: './home.page.html',
  styleUrl: './home.page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class HomePage {
  readonly api = toSignal(
    inject(StatusService)
      .status()
      .pipe(
        map((): ApiState => 'reachable'),
        catchError(() => of<ApiState>('unreachable')),
        startWith<ApiState>('checking'),
      ),
    { requireSync: true },
  );
}
