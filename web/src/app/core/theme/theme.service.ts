import { DOCUMENT } from '@angular/common';
import { Injectable, inject, signal } from '@angular/core';

export type Theme = 'dark' | 'light' | 'system';

const STORAGE_KEY = 'r16a-stream.theme';

function isTheme(value: unknown): value is Theme {
  return value === 'dark' || value === 'light' || value === 'system';
}

function readStored(): Theme | null {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return isTheme(stored) ? stored : null;
  } catch {
    // Storage can be unavailable in private windows.
    return null;
  }
}

/**
 * Holds the theme choice and mirrors it on `<html data-theme>`, which the token CSS reads.
 * `theme-init.js` applies the same stored value before Angular starts, so there is no flash.
 */
@Injectable({ providedIn: 'root' })
export class ThemeService {
  private readonly document = inject(DOCUMENT);
  readonly theme = signal<Theme>('dark');

  /** Reads the stored choice. Called once at startup. */
  apply(): void {
    this.set(readStored() ?? 'dark', false);
  }

  set(theme: Theme, persist = true): void {
    this.theme.set(theme);
    this.document.documentElement.setAttribute('data-theme', theme);
    if (!persist) {
      return;
    }
    try {
      localStorage.setItem(STORAGE_KEY, theme);
    } catch {
      // The choice still applies for this page view.
    }
  }
}
