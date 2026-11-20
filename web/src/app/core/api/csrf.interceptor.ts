import { type HttpInterceptorFn } from '@angular/common/http';

/**
 * Marks every API call with the header the backend's CSRF check expects on state-changing
 * cookie requests. Only same-origin `/api` calls get it, never the bucket paths.
 */
export const csrfHeaderInterceptor: HttpInterceptorFn = (req, next) => {
  if (!req.url.startsWith('/api/')) {
    return next(req);
  }
  return next(req.clone({ setHeaders: { 'X-R16a-Request': '1' } }));
};
