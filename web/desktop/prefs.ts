// Per-viewer UI preferences. Storage can be unavailable (private mode, blocked site data): never throw.

export function readPref(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function writePref(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // preference just won't persist
  }
}

export type Theme = "system" | "light" | "dark";

export function applyTheme(theme: Theme) {
  // No data-theme = DaisyUI follows prefers-color-scheme (see tailwind.css).
  if (theme === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme;
  writePref("theme", theme === "system" ? null : theme);
}

export function savedTheme(): Theme {
  const t = readPref("theme");
  return t === "light" || t === "dark" ? t : "system";
}
