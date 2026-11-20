import { inject } from '@angular/core';
import { type CanActivateFn, Router } from '@angular/router';

import { SessionService } from './session.service';

/**
 * Sends signed-out visitors to the sign-in page and remembers where they wanted to go. A UX
 * convenience, the API refuses every call without a session anyway.
 */
export const authGuard: CanActivateFn = async (_route, state) => {
  const session = inject(SessionService);
  const router = inject(Router);
  const result = await session.ensureLoaded();
  if (result === 'signed-in') {
    return true;
  }
  if (result === 'unavailable') {
    return router.createUrlTree(['/signin'], {
      queryParams: { error: 'unavailable', returnTo: state.url },
    });
  }
  return router.createUrlTree(['/signin'], { queryParams: { returnTo: state.url } });
};
