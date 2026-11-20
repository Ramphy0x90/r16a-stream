import { Injectable, inject } from '@angular/core';
import { Title } from '@angular/platform-browser';
import { type RouterStateSnapshot, TitleStrategy } from '@angular/router';
import { TranslocoService } from '@jsverse/transloco';

/**
 * Route titles are translation keys. The page title becomes "<page> · <app name>" in the
 * active language and follows language switches.
 */
@Injectable()
export class TranslatedTitleStrategy extends TitleStrategy {
  private readonly title = inject(Title);
  private readonly transloco = inject(TranslocoService);

  override updateTitle(snapshot: RouterStateSnapshot): void {
    const key = this.buildTitle(snapshot);
    this.transloco
      .selectTranslate(key ? [key, 'app.name'] : ['app.name'])
      .subscribe((parts: string[]) => this.title.setTitle(parts.join(' · ')));
  }
}
