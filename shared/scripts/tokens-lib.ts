/**
 * Colour token model shared by the generator and the contrast check.
 *
 * Every colour is kept as HSL with lightness in percent, because the web side derives tints
 * and alpha variants from the separate channels. Conversion to RGB happens only for the
 * contrast maths and for the hex shown in reports.
 */

import { readFileSync } from 'node:fs';

export interface Hsl {
  h: number;
  s: number;
  l: number;
  a: number;
}

export interface ScaleDef {
  hue: number;
  steps: Record<string, [number, number]>;
}

export type SemanticDef = string | { ref: string; lighten?: number; alpha?: number };

export interface ContrastPair {
  fg: string;
  bg: string;
  min: number;
  use: string;
}

export interface TokenFile {
  brandBase: { scale: string; step: string };
  scales: Record<string, ScaleDef>;
  themes: Record<string, Record<string, SemanticDef>>;
  contrastPairs: ContrastPair[];
}

export const THEMES = ['dark', 'light'] as const;
export type ThemeName = (typeof THEMES)[number];

export function loadTokens(path: string): TokenFile {
  return JSON.parse(readFileSync(path, 'utf8')) as TokenFile;
}

/** Looks up a scale step such as `brand.500`, failing loudly on a typo in tokens.json. */
export function scaleColour(tokens: TokenFile, ref: string): Hsl {
  const [scale, step] = ref.split('.');
  const def = tokens.scales[scale ?? ''];
  const sl = def?.steps[step ?? ''];
  if (!def || !sl) {
    throw new Error(`unknown scale reference ${ref}`);
  }
  return { h: def.hue, s: sl[0], l: sl[1], a: 1 };
}

/** Resolves a semantic token of one theme to a concrete colour. */
export function semanticColour(tokens: TokenFile, theme: ThemeName, name: string): Hsl {
  const def = tokens.themes[theme]?.[name];
  if (def === undefined) {
    throw new Error(`unknown semantic token ${name} in theme ${theme}`);
  }
  if (typeof def === 'string') {
    return scaleColour(tokens, def);
  }
  const base = scaleColour(tokens, def.ref);
  return {
    h: base.h,
    s: base.s,
    l: clamp(base.l + (def.lighten ?? 0), 0, 100),
    a: def.alpha ?? 1,
  };
}

/** The scale step a semantic token points at, so CSS can reuse the step's channels. */
export function semanticRef(tokens: TokenFile, theme: ThemeName, name: string): string | undefined {
  const def = tokens.themes[theme]?.[name];
  if (typeof def === 'string') {
    return def;
  }
  return undefined;
}

export function semanticNames(tokens: TokenFile): string[] {
  const dark = Object.keys(tokens.themes['dark'] ?? {});
  for (const theme of THEMES) {
    const names = Object.keys(tokens.themes[theme] ?? {});
    const missing = dark.filter((n) => !names.includes(n));
    const extra = names.filter((n) => !dark.includes(n));
    if (missing.length > 0 || extra.length > 0) {
      throw new Error(`theme ${theme} differs from dark, missing ${missing.join(', ')} extra ${extra.join(', ')}`);
    }
  }
  return dark;
}

export function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

/** CSS Color 4 HSL to sRGB, channels in 0 to 1. */
export function hslToRgb({ h, s, l }: Hsl): [number, number, number] {
  const sat = s / 100;
  const light = l / 100;
  const f = (n: number): number => {
    const k = (n + h / 30) % 12;
    const a = sat * Math.min(light, 1 - light);
    return light - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
  };
  return [f(0), f(8), f(4)];
}

export function toHex(c: Hsl): string {
  return (
    '#' +
    hslToRgb(c)
      .map((v) => Math.round(v * 255).toString(16).padStart(2, '0'))
      .join('')
  );
}

/** Alpha-composites a translucent colour over an opaque background, in sRGB. */
export function composite(fg: Hsl, bg: Hsl): [number, number, number] {
  const f = hslToRgb(fg);
  const b = hslToRgb(bg);
  return [0, 1, 2].map((i) => f[i]! * fg.a + b[i]! * (1 - fg.a)) as [number, number, number];
}

/** WCAG 2.x relative luminance. Channels are quantised to 8 bits first, as browsers render them. */
export function luminance(rgb: [number, number, number]): number {
  const lin = (v: number): number => {
    const c = Math.round(v * 255) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2]);
}

export function contrastRatio(fg: Hsl, bg: Hsl): number {
  const fgRgb = composite(fg, bg);
  const a = luminance(fgRgb);
  const b = luminance(hslToRgb(bg));
  const [hi, lo] = a > b ? [a, b] : [b, a];
  return (hi + 0.05) / (lo + 0.05);
}

/** Formats a number without trailing zeros, so 39.4 stays 39.4 and 40 stays 40. */
export function num(v: number): string {
  return Number.parseFloat(v.toFixed(3)).toString();
}
