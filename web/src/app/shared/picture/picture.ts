import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { type components } from '../../core/api/schema';

export type ImageSet = components['schemas']['ImageSet'];

/** `srcset` of one format, widest last. */
export function srcset(set: ImageSet, format: 'avif' | 'webp'): string {
  return set.sources
    .filter((s) => s.format === format)
    .sort((a, b) => a.width - b.width)
    .map((s) => `${s.url} ${s.width}w`)
    .join(', ');
}

/**
 * A responsive image from the media bucket. AVIF first, WebP as the fallback, lazy loaded, with
 * the average colour behind it while it loads. URLs are presigned per day, so the browser cache
 * keeps them (SPEC 8.3).
 */
@Component({
  selector: 'app-picture',
  templateUrl: './picture.html',
  styleUrl: './picture.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class Picture {
  readonly image = input.required<ImageSet>();
  /** The `sizes` attribute, how wide the image is shown. */
  readonly sizes = input('100vw');
  readonly alt = input('');
  readonly eager = input(false);

  readonly avif = computed(() => srcset(this.image(), 'avif'));
  readonly webp = computed(() => srcset(this.image(), 'webp'));
  /** The smallest WebP, for browsers that ignore `srcset`. */
  readonly fallback = computed(
    () =>
      [...this.image().sources]
        .filter((s) => s.format === 'webp')
        .sort((a, b) => a.width - b.width)[0]?.url ?? '',
  );
  /** The server checks the `H S% L%` form, the client only builds the colour from it. */
  readonly placeholder = computed(() => {
    const hsl = this.image().placeholderHsl;
    return hsl && /^\d{1,3} \d{1,3}% \d{1,3}%$/.test(hsl) ? `hsl(${hsl})` : null;
  });
}
