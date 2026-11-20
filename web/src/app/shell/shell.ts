import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { RouterLink, RouterOutlet } from '@angular/router';
import { TranslocoPipe } from '@jsverse/transloco';

import { SessionService } from '../core/auth/session.service';

/** Page frame with the header. Navigation grows with the feature phases. */
@Component({
  selector: 'app-shell',
  imports: [RouterOutlet, RouterLink, TranslocoPipe],
  templateUrl: './shell.html',
  styleUrl: './shell.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class Shell {
  private readonly session = inject(SessionService);
  readonly me = this.session.me;

  signOut(): void {
    void this.session.signOut();
  }
}
