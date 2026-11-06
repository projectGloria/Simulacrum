// Persisted user settings. localStorage survives restarts in WebView2's per-app profile.
// Everything read back is re-validated: old versions or hand-edited storage must not brick the UI.

export type Shape = "rounded" | "circle" | "none";
export const ALL_SIZES = [16, 24, 32, 48, 64, 128, 256] as const;

export interface Settings {
  shape: Shape;
  radius: number; // % of side, 0–50
  padding: number; // % of side per edge, 0–30
  sizes: number[];
}

export interface Prefs {
  settings: Settings;
  alsoPng: boolean;
}

const KEY = "simulacrum.prefs.v1";

export const DEFAULTS: Prefs = {
  settings: { shape: "rounded", radius: 22, padding: 0, sizes: [...ALL_SIZES] },
  alsoPng: false,
};

const clamp = (v: unknown, lo: number, hi: number, dflt: number) =>
  typeof v === "number" && Number.isFinite(v) ? Math.min(hi, Math.max(lo, Math.round(v))) : dflt;

export function loadPrefs(): Prefs {
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (!raw || typeof raw !== "object") return structuredClone(DEFAULTS);
    const s = raw.settings ?? {};
    const sizes = Array.isArray(s.sizes) ? ALL_SIZES.filter((n) => s.sizes.includes(n)) : [];
    return {
      settings: {
        shape: (["rounded", "circle", "none"] as const).includes(s.shape) ? s.shape : DEFAULTS.settings.shape,
        radius: clamp(s.radius, 0, 50, DEFAULTS.settings.radius),
        padding: clamp(s.padding, 0, 30, DEFAULTS.settings.padding),
        sizes: sizes.length ? sizes : [...ALL_SIZES],
      },
      alsoPng: typeof raw.alsoPng === "boolean" ? raw.alsoPng : DEFAULTS.alsoPng,
    };
  } catch {
    return structuredClone(DEFAULTS); // corrupt JSON → defaults, never a crash
  }
}

export function savePrefs(p: Prefs): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(p));
  } catch {
    // Quota/private mode: losing persistence is acceptable, breaking the app is not.
  }
}
