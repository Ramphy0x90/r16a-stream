import { type Routes } from '@angular/router';

import { authGuard } from './core/auth/auth.guard';

/** Every route is lazy, so the initial bundle holds only the bootstrap code. */
export const routes: Routes = [
  {
    path: '',
    loadComponent: () => import('./shell/shell').then((m) => m.Shell),
    children: [
      {
        path: 'signin',
        loadComponent: () => import('./signin/signin.page').then((m) => m.SigninPage),
        title: 'signin.title',
      },
      {
        path: '',
        canActivate: [authGuard],
        loadComponent: () => import('./home/home.page').then((m) => m.HomePage),
        title: 'home.title',
      },
    ],
  },
  { path: '**', redirectTo: '' },
];
