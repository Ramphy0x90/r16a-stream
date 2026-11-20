import { InjectionToken } from '@angular/core';

/** Full-page navigation, replaceable in tests where jsdom cannot leave the page. */
export interface Navigator {
  assign(url: string): void;
}

export const NAVIGATOR = new InjectionToken<Navigator>('NAVIGATOR', {
  providedIn: 'root',
  factory: () => ({ assign: (url: string) => window.location.assign(url) }),
});
