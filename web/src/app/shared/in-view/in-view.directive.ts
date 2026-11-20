import { DestroyRef, Directive, ElementRef, inject, output } from '@angular/core';

/**
 * Emits when the element scrolls into view, for infinite lists. The observer is disconnected
 * with the directive, so nothing outlives the page.
 */
@Directive({ selector: '[appInView]' })
export class InViewDirective {
  readonly appInView = output<null>();

  constructor() {
    const el = inject(ElementRef<HTMLElement>).nativeElement as HTMLElement;
    if (typeof IntersectionObserver === 'undefined') {
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          this.appInView.emit(null);
        }
      },
      { rootMargin: '400px 0px' },
    );
    observer.observe(el);
    inject(DestroyRef).onDestroy(() => observer.disconnect());
  }
}
