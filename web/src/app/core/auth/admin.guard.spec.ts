import { provideZonelessChangeDetection, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import {
  type ActivatedRouteSnapshot,
  type RouterStateSnapshot,
  UrlTree,
  provideRouter,
} from '@angular/router';

import { adminGuard } from './admin.guard';
import { SessionService } from './session.service';

describe('adminGuard', () => {
  async function run(isAdmin: boolean): Promise<boolean | UrlTree> {
    TestBed.configureTestingModule({
      providers: [
        provideZonelessChangeDetection(),
        provideRouter([]),
        {
          provide: SessionService,
          useValue: { ensureLoaded: () => Promise.resolve('signed-in'), isAdmin: signal(isAdmin) },
        },
      ],
    });
    return (await TestBed.runInInjectionContext(() =>
      adminGuard({} as ActivatedRouteSnapshot, {} as RouterStateSnapshot),
    )) as boolean | UrlTree;
  }

  it('lets admins in and sends members home', async () => {
    expect(await run(true)).toBe(true);
    TestBed.resetTestingModule();
    expect((await run(false)).toString()).toBe('/');
  });
});
