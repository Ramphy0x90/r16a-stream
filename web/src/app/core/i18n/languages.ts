/**
 * UI languages. Adding one means adding `public/i18n/<code>.json` and its code here, and
 * `npm run check:i18n` then requires every key to exist in it.
 */
export const AVAILABLE_LANGUAGES = ['en', 'it', 'es', 'de'] as const;
export type UiLanguage = (typeof AVAILABLE_LANGUAGES)[number];

const STORAGE_KEY = 'r16a-stream.lang';

export function isUiLanguage(value: string): value is UiLanguage {
  return (AVAILABLE_LANGUAGES as readonly string[]).includes(value);
}

/**
 * The stored choice, else the first browser language we support, else English. The server
 * setting from `/me` overrides this once the user is signed in (Phase 8).
 */
export function initialLanguage(): UiLanguage {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored && isUiLanguage(stored)) {
      return stored;
    }
  } catch {
    // Storage can be unavailable in private windows.
  }
  for (const tag of navigator.languages ?? []) {
    const base = tag.split('-')[0]?.toLowerCase() ?? '';
    if (isUiLanguage(base)) {
      return base;
    }
  }
  return 'en';
}
