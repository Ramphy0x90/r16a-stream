import { HttpErrorResponse, type HttpInterceptorFn } from '@angular/common/http';
import { inject } from '@angular/core';
import { Router } from '@angular/router';
import { catchError, throwError } from 'rxjs';

import { SessionService } from './session.service';

/**
 * A 401 from the API means the session expired, was revoked, or lost its access claim. The
 * user is sent to sign in and comes back to the same page afterwards. `/me` is exempt, its
 * 401 is the normal answer for signed-out visitors and the session service handles it.
 */
export const unauthorizedInterceptor: HttpInterceptorFn = (req, next) => {
  const session = inject(SessionService);
  const router = inject(Router);
  return next(req).pipe(
    catchError((e: unknown) => {
      if (
        e instanceof HttpErrorResponse &&
        e.status === 401 &&
        req.url.startsWith('/api/') &&
        req.url !== '/api/v1/me' &&
        req.url !== '/api/v1/auth/logout'
      ) {
        session.sessionLost(router.url);
      }
      return throwError(() => e);
    }),
  );
};
