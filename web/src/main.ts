import { bootstrapApplication } from '@angular/platform-browser';

import { App } from './app/app';
import { appConfig } from './app/app.config';

bootstrapApplication(App, appConfig).catch(() => {
  // Translations are not available when bootstrap itself fails, so this one message is fixed.
  document.body.textContent = 'The app could not start. Please reload the page.';
});
