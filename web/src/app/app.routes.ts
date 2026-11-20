import { type Routes } from '@angular/router';

import { adminGuard } from './core/auth/admin.guard';
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
        children: [
          {
            path: '',
            loadComponent: () => import('./home/home.page').then((m) => m.HomePage),
            title: 'home.title',
          },
          {
            path: 'title/:id',
            loadComponent: () => import('./title/title.page').then((m) => m.TitlePage),
            title: 'title.pageTitle',
          },
          {
            path: 'admin',
            canActivate: [adminGuard],
            children: [
              {
                path: '',
                loadComponent: () =>
                  import('./admin/admin-titles.page').then((m) => m.AdminTitlesPage),
                title: 'admin.titles.heading',
              },
              {
                path: 'titles/new',
                loadComponent: () =>
                  import('./admin/admin-title-new.page').then((m) => m.AdminTitleNewPage),
                title: 'admin.titles.new',
              },
              {
                path: 'titles/:id',
                loadComponent: () =>
                  import('./admin/admin-title-edit.page').then((m) => m.AdminTitleEditPage),
                title: 'admin.titles.edit',
              },
            ],
          },
        ],
      },
    ],
  },
  { path: '**', redirectTo: '' },
];
