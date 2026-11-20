import { provideZonelessChangeDetection } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { type ImageSet, Picture, srcset } from './picture';

const set: ImageSet = {
  placeholderHsl: '160 84% 39%',
  sources: [
    { format: 'webp', width: 342, url: '/m/342.webp' },
    { format: 'avif', width: 342, url: '/m/342.avif' },
    { format: 'webp', width: 185, url: '/m/185.webp' },
    { format: 'avif', width: 185, url: '/m/185.avif' },
  ],
};

describe('Picture', () => {
  it('builds srcsets per format, narrowest first', () => {
    expect(srcset(set, 'avif')).toBe('/m/185.avif 185w, /m/342.avif 342w');
    expect(srcset(set, 'webp')).toBe('/m/185.webp 185w, /m/342.webp 342w');
  });

  it('renders AVIF first, a WebP fallback, and the placeholder colour', () => {
    TestBed.configureTestingModule({
      imports: [Picture],
      providers: [provideZonelessChangeDetection()],
    });
    const f = TestBed.createComponent(Picture);
    f.componentRef.setInput('image', set);
    f.detectChanges();
    const el = f.nativeElement as HTMLElement;
    expect(el.querySelector('source')?.getAttribute('type')).toBe('image/avif');
    expect(el.querySelector('img')?.getAttribute('src')).toBe('/m/185.webp');
    expect(el.querySelector('img')?.getAttribute('loading')).toBe('lazy');
    expect(f.componentInstance.placeholder()).toBe('hsl(160 84% 39%)');
  });

  it('ignores a placeholder that is not the expected form', () => {
    TestBed.configureTestingModule({
      imports: [Picture],
      providers: [provideZonelessChangeDetection()],
    });
    const f = TestBed.createComponent(Picture);
    f.componentRef.setInput('image', { ...set, placeholderHsl: 'red; background:url(x)' });
    expect(f.componentInstance.placeholder()).toBeNull();
  });
});
