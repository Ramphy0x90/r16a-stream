import { provideZonelessChangeDetection } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import {
  type ActivatedRouteSnapshot,
  type RouterStateSnapshot,
  UrlTree,
  provideRouter,
} from '@angular/router';

import { authGuard } from './auth.guard';
import { type SessionState, SessionService } from './session.service';

describe('authGuard', () => {
  async function run(state: SessionState): Promise<boolean | UrlTree> {
    TestBed.configureTestingModule({
      providers: [
        provideZonelessChangeDetection(),
        provideRouter([]),
        { provide: SessionService, useValue: { ensureLoaded: () => Promise.resolve(state) } },
      ],
    });
    const result = await TestBed.runInInjectionContext(() =>
      authGuard({} as ActivatedRouteSnapshot, { url: '/title/7' } as RouterStateSnapshot),
    );
    return result as boolean | UrlTree;
  }

  it('lets signed-in users through', async () => {
    expect(await run('signed-in')).toBe(true);
  });

  it('sends signed-out users to sign in and remembers the page', async () => {
    const tree = await run('signed-out');
    expect(tree.toString()).toBe('/signin?returnTo=%2Ftitle%2F7');
  });

  it('tells the user when the server could not be asked', async () => {
    const tree = await run('unavailable');
    expect(tree.toString()).toBe('/signin?error=unavailable&returnTo=%2Ftitle%2F7');
  });
});
