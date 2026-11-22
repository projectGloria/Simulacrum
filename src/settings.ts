// Persisted user settings and presets. Everything read back from localStorage is
// re-validated so stale v1 state or hand-edited storage cannot break the UI.

export type Shape = "rounded" | "squircle" | "circle" | "hexagon" | "octagon" | "shield" | "none";
export type FitMode = "crop" | "contain";
export type BgMode = "transparent" | "solid" | "linear" | "radial";
export type BadgeMode = "none" | "dot" | "ring";
export type BadgeCorner = "bottomright" | "topright" | "bottomleft" | "topleft";
export type BackdropMode = "checker" | "dark" | "light" | "custom" | "android";
export type PackKind = "tauri" | "web";
export type AndroidBgMode = "auto" | "color" | "image";
export type AndroidAnchor = "center" | "bottom";
export type AndroidMono = "off" | "auto" | "custom";

/** Android adaptive-icon options. `scale` is % of the visible 72 dp; offsets are dp. */
export interface AndroidSettings {
  bgMode: AndroidBgMode;
  bgColor: string;
  flatten: boolean;
  scale: number;
  anchor: AndroidAnchor;
  offsetX: number;
  offsetY: number;
  mono: AndroidMono;
  monoThreshold: number;
  stripCorners: boolean;
  legacy: boolean;
}

export const ANDROID_SCALE_PRESETS = {
  fullBleed: 110,
  fitVisible: 100,
  safeZone: Math.round((66 / 72) * 1000) / 10, // 91.7
} as const;

export const DEFAULT_ANDROID: AndroidSettings = {
  bgMode: "auto",
  bgColor: "#ffffff",
  flatten: false,
  scale: 100,
  anchor: "center",
  offsetX: 0,
  offsetY: 0,
  mono: "off",
  monoThreshold: 20,
  stripCorners: true,
  legacy: false,
};

export const DEFAULT_SIZES = [16, 24, 32, 48, 64, 128, 256] as const;
export const ALL_SIZES = [16, 20, 24, 32, 40, 48, 60, 64, 96, 128, 256] as const;

export interface Settings {
  shape: Shape;
  radius: number;
  padding: number;
  sizes: number[];

  fitMode: FitMode;
  zoom: number;
  panX: number;
  panY: number;
  rotation: number;
  flipH: boolean;
  flipV: boolean;

  bgMode: BgMode;
  bgColor1: string;
  bgColor2: string;
  bgAngle: number;

  borderWidth: number;
  borderColor: string;
  borderOpacity: number;

  shadow: boolean;
  shadowBlur: number;
  shadowOpacity: number;
  shadowOffsetY: number;

  badgeMode: BadgeMode;
  badgeCorner: BadgeCorner;
  badgeColor: string;
  badgeSize: number;

  sharpen: number;
  legacyBmp: boolean;

  android: AndroidSettings;
}

export interface Preset {
  id: string;
  name: string;
  builtin?: boolean;
  settings: Settings;
}

export interface Prefs {
  settings: Settings;
  alsoPng: boolean;
  alsoPng1024: boolean;
  backdrop: BackdropMode;
  backdropColor: string;
  pixelGrid: boolean;
  presets: Preset[];
  /** Folder last opened in the Directory reader; re-scanned on launch. */
  lastProject: string | null;
}

export const DEFAULT_SETTINGS: Settings = {
  shape: "rounded",
  radius: 22,
  padding: 0,
  sizes: [...DEFAULT_SIZES],
  fitMode: "crop",
  zoom: 1,
  panX: 0,
  panY: 0,
  rotation: 0,
  flipH: false,
  flipV: false,
  bgMode: "transparent",
  bgColor1: "#1e293b",
  bgColor2: "#0f172a",
  bgAngle: 135,
  borderWidth: 0,
  borderColor: "#ffffff",
  borderOpacity: 35,
  shadow: false,
  shadowBlur: 4,
  shadowOpacity: 45,
  shadowOffsetY: 2.5,
  badgeMode: "none",
  badgeCorner: "bottomright",
  badgeColor: "#22c55e",
  badgeSize: 20,
  sharpen: 0,
  legacyBmp: false,
  android: { ...DEFAULT_ANDROID },
};

export const BUILTIN_PRESETS: Preset[] = [
  {
    id: "builtin:default",
    name: "Default Rounded",
    builtin: true,
    settings: structuredClone(DEFAULT_SETTINGS),
  },
  {
    id: "builtin:win11",
    name: "Windows 11 App",
    builtin: true,
    settings: {
      ...structuredClone(DEFAULT_SETTINGS),
      shape: "squircle",
      radius: 24,
      padding: 8,
      bgMode: "linear",
      bgColor1: "#2563eb",
      bgColor2: "#1e3a8a",
      bgAngle: 135,
      borderWidth: 1.5,
      borderColor: "#ffffff",
      borderOpacity: 25,
      shadow: true,
    },
  },
  {
    id: "builtin:macos",
    name: "macOS Squircle",
    builtin: true,
    settings: {
      ...structuredClone(DEFAULT_SETTINGS),
      shape: "squircle",
      radius: 22,
      padding: 10,
      bgMode: "solid",
      bgColor1: "#f8fafc",
      shadow: true,
      shadowBlur: 4.5,
      shadowOpacity: 40,
      shadowOffsetY: 3,
    },
  },
  {
    id: "builtin:circle",
    name: "Circle Avatar",
    builtin: true,
    settings: {
      ...structuredClone(DEFAULT_SETTINGS),
      shape: "circle",
      padding: 0,
      borderWidth: 2,
      borderColor: "#ffffff",
      borderOpacity: 40,
    },
  },
  {
    id: "builtin:hex",
    name: "Hexagon Badge",
    builtin: true,
    settings: {
      ...structuredClone(DEFAULT_SETTINGS),
      shape: "hexagon",
      radius: 12,
      padding: 10,
      bgMode: "linear",
      bgColor1: "#7c3aed",
      bgColor2: "#4c1d95",
      bgAngle: 135,
      borderWidth: 2,
      borderColor: "#ffffff",
      borderOpacity: 35,
    },
  },
  {
    id: "builtin:raw",
    name: "Raw Pixel Art",
    builtin: true,
    settings: {
      ...structuredClone(DEFAULT_SETTINGS),
      shape: "none",
      padding: 0,
      legacyBmp: true,
    },
  },
];

const KEY_V2 = "simulacrum.prefs.v2";
const KEY_V1 = "simulacrum.prefs.v1";

export const DEFAULTS: Prefs = {
  settings: structuredClone(DEFAULT_SETTINGS),
  alsoPng: false,
  alsoPng1024: false,
  backdrop: "checker",
  backdropColor: "#2b303b",
  pixelGrid: false,
  presets: [],
  lastProject: null,
};

const SHAPES: readonly Shape[] = ["rounded", "squircle", "circle", "hexagon", "octagon", "shield", "none"];
const FIT_MODES: readonly FitMode[] = ["crop", "contain"];
const BG_MODES: readonly BgMode[] = ["transparent", "solid", "linear", "radial"];
const BADGE_MODES: readonly BadgeMode[] = ["none", "dot", "ring"];
const BADGE_CORNERS: readonly BadgeCorner[] = ["bottomright", "topright", "bottomleft", "topleft"];
const BACKDROPS: readonly BackdropMode[] = ["checker", "dark", "light", "custom", "android"];
const ANDROID_BG: readonly AndroidBgMode[] = ["auto", "color", "image"];
const ANDROID_ANCHORS: readonly AndroidAnchor[] = ["center", "bottom"];
const ANDROID_MONO: readonly AndroidMono[] = ["off", "auto", "custom"];

const clampInt = (v: unknown, lo: number, hi: number, dflt: number) =>
  typeof v === "number" && Number.isFinite(v) ? Math.min(hi, Math.max(lo, Math.round(v))) : dflt;

const clampFloat = (v: unknown, lo: number, hi: number, dflt: number, step = 10) =>
  typeof v === "number" && Number.isFinite(v) ? Math.min(hi, Math.max(lo, Math.round(v * step) / step)) : dflt;

const validHex = (v: unknown, dflt: string) =>
  typeof v === "string" && /^#[0-9a-fA-F]{6}$/.test(v.trim()) ? v.trim().toLowerCase() : dflt;

export function validateSettings(raw: unknown): Settings {
  const s = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const d = DEFAULT_SETTINGS;
  const sizes = Array.isArray(s.sizes) ? ALL_SIZES.filter((n) => (s.sizes as unknown[]).includes(n)) : [];
  const rot = s.rotation === 90 || s.rotation === 180 || s.rotation === 270 ? s.rotation : 0;

  return {
    shape: SHAPES.includes(s.shape as Shape) ? (s.shape as Shape) : d.shape,
    radius: clampInt(s.radius, 0, 50, d.radius),
    padding: clampInt(s.padding, 0, 30, d.padding),
    sizes: sizes.length ? sizes : [...DEFAULT_SIZES],
    fitMode: FIT_MODES.includes(s.fitMode as FitMode) ? (s.fitMode as FitMode) : d.fitMode,
    zoom: clampFloat(s.zoom, 0.5, 4.0, d.zoom, 100),
    panX: clampFloat(s.panX, -100, 100, d.panX, 10),
    panY: clampFloat(s.panY, -100, 100, d.panY, 10),
    rotation: rot,
    flipH: typeof s.flipH === "boolean" ? s.flipH : d.flipH,
    flipV: typeof s.flipV === "boolean" ? s.flipV : d.flipV,
    bgMode: BG_MODES.includes(s.bgMode as BgMode) ? (s.bgMode as BgMode) : d.bgMode,
    bgColor1: validHex(s.bgColor1, d.bgColor1),
    bgColor2: validHex(s.bgColor2, d.bgColor2),
    bgAngle: clampInt(s.bgAngle, 0, 360, d.bgAngle),
    borderWidth: clampFloat(s.borderWidth, 0, 10, d.borderWidth, 10),
    borderColor: validHex(s.borderColor, d.borderColor),
    borderOpacity: clampInt(s.borderOpacity, 0, 100, d.borderOpacity),
    shadow: typeof s.shadow === "boolean" ? s.shadow : d.shadow,
    shadowBlur: clampFloat(s.shadowBlur, 1, 12, d.shadowBlur, 10),
    shadowOpacity: clampInt(s.shadowOpacity, 0, 100, d.shadowOpacity),
    shadowOffsetY: clampFloat(s.shadowOffsetY, -8, 12, d.shadowOffsetY, 10),
    badgeMode: BADGE_MODES.includes(s.badgeMode as BadgeMode) ? (s.badgeMode as BadgeMode) : d.badgeMode,
    badgeCorner: BADGE_CORNERS.includes(s.badgeCorner as BadgeCorner) ? (s.badgeCorner as BadgeCorner) : d.badgeCorner,
    badgeColor: validHex(s.badgeColor, d.badgeColor),
    badgeSize: clampInt(s.badgeSize, 8, 35, d.badgeSize),
    sharpen: clampInt(s.sharpen, 0, 100, d.sharpen),
    legacyBmp: typeof s.legacyBmp === "boolean" ? s.legacyBmp : d.legacyBmp,
    android: validateAndroid(s.android),
  };
}

export function validateAndroid(raw: unknown): AndroidSettings {
  const a = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const d = DEFAULT_ANDROID;
  const bool = (v: unknown, dflt: boolean) => (typeof v === "boolean" ? v : dflt);
  return {
    bgMode: ANDROID_BG.includes(a.bgMode as AndroidBgMode) ? (a.bgMode as AndroidBgMode) : d.bgMode,
    bgColor: validHex(a.bgColor, d.bgColor),
    flatten: bool(a.flatten, d.flatten),
    scale: clampFloat(a.scale, 60, 130, d.scale, 10),
    anchor: ANDROID_ANCHORS.includes(a.anchor as AndroidAnchor) ? (a.anchor as AndroidAnchor) : d.anchor,
    offsetX: clampFloat(a.offsetX, -18, 18, d.offsetX, 2),
    offsetY: clampFloat(a.offsetY, -18, 18, d.offsetY, 2),
    mono: ANDROID_MONO.includes(a.mono as AndroidMono) ? (a.mono as AndroidMono) : d.mono,
    monoThreshold: clampInt(a.monoThreshold, 1, 100, d.monoThreshold),
    stripCorners: bool(a.stripCorners, d.stripCorners),
    legacy: bool(a.legacy, d.legacy),
  };
}

export function loadPrefs(): Prefs {
  try {
    const rawStr = localStorage.getItem(KEY_V2) ?? localStorage.getItem(KEY_V1) ?? "null";
    const raw = JSON.parse(rawStr);
    if (!raw || typeof raw !== "object") return structuredClone(DEFAULTS);
    const presets: Preset[] = Array.isArray(raw.presets)
      ? (raw.presets as unknown[])
          .filter((p): p is Record<string, unknown> => !!p && typeof p === "object")
          .map((p: Record<string, unknown>) => ({
            id: typeof p.id === "string" ? p.id : `user:${Date.now()}`,
            name: typeof p.name === "string" && p.name.trim() ? p.name.trim().slice(0, 48) : "Custom Preset",
            builtin: false,
            settings: validateSettings(p.settings),
          }))
      : [];
    return {
      settings: validateSettings(raw.settings),
      alsoPng: typeof raw.alsoPng === "boolean" ? raw.alsoPng : DEFAULTS.alsoPng,
      alsoPng1024: typeof raw.alsoPng1024 === "boolean" ? raw.alsoPng1024 : DEFAULTS.alsoPng1024,
      backdrop: BACKDROPS.includes(raw.backdrop) ? raw.backdrop : DEFAULTS.backdrop,
      backdropColor: validHex(raw.backdropColor, DEFAULTS.backdropColor),
      pixelGrid: typeof raw.pixelGrid === "boolean" ? raw.pixelGrid : DEFAULTS.pixelGrid,
      presets,
      lastProject:
        typeof raw.lastProject === "string" && raw.lastProject.trim() && raw.lastProject.length < 1024
          ? raw.lastProject
          : null,
    };
  } catch {
    return structuredClone(DEFAULTS);
  }
}

export function savePrefs(p: Prefs): void {
  try {
    localStorage.setItem(KEY_V2, JSON.stringify(p));
  } catch {
    // Ignore storage quota/privacy failures.
  }
}
