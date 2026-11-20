import { inject } from '@angular/core';
import { type CanActivateFn, Router } from '@angular/router';

import { SessionService } from './session.service';

/**
 * Keeps members out of the admin screens. UX only, the API answers 404 to every admin call
 * from a non-admin whatever the client shows.
 */
export const adminGuard: CanActivateFn = async () => {
  // Both injected before the await, inject() only works synchronously inside the guard.
  const session = inject(SessionService);
  const router = inject(Router);
  await session.ensureLoaded();
  return session.isAdmin() ? true : router.createUrlTree(['/']);
};
