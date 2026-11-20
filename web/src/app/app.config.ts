import { provideHttpClient, withFetch, withInterceptors } from '@angular/common/http';
import {
  type ApplicationConfig,
  inject,
  isDevMode,
  provideAppInitializer,
  provideBrowserGlobalErrorListeners,
  provideZonelessChangeDetection,
} from '@angular/core';
import { TitleStrategy, provideRouter, withComponentInputBinding } from '@angular/router';
import { provideTransloco } from '@jsverse/transloco';

import { routes } from './app.routes';
import { csrfHeaderInterceptor } from './core/api/csrf.interceptor';
import { unauthorizedInterceptor } from './core/auth/unauthorized.interceptor';
import { initialLanguage } from './core/i18n/languages';
import { TranslatedTitleStrategy } from './core/i18n/title-strategy';
import { TranslocoHttpLoader } from './core/i18n/transloco-loader';
import { AVAILABLE_LANGUAGES } from './core/i18n/languages';
import { ThemeService } from './core/theme/theme.service';

export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideZonelessChangeDetection(),
    provideRouter(routes, withComponentInputBinding()),
    provideHttpClient(
      withFetch(),
      withInterceptors([csrfHeaderInterceptor, unauthorizedInterceptor]),
    ),
    provideTransloco({
      config: {
        availableLangs: [...AVAILABLE_LANGUAGES],
        defaultLang: initialLanguage(),
        fallbackLang: 'en',
        reRenderOnLangChange: true,
        prodMode: !isDevMode(),
        missingHandler: { useFallbackTranslation: true },
      },
      loader: TranslocoHttpLoader,
    }),
    { provide: TitleStrategy, useClass: TranslatedTitleStrategy },
    provideAppInitializer(() => inject(ThemeService).apply()),
  ],
};
