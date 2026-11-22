// Typed wrappers over the Rust commands. Argument keys are camelCase; Tauri maps them to the
// snake_case Rust parameters (alsoPng → also_png).
import { Channel, invoke } from "@tauri-apps/api/core";
import type { AndroidBgMode, PackKind, Settings } from "./settings";

export interface IcoEntryInfo {
  width: number;
  height: number;
  bpp: number;
  isPng: boolean;
  bytes: number;
}

export type LoadResult =
  | {
      status: "ok";
      id: number;
      name: string;
      width: number;
      height: number;
      thumb: string;
      warning: string | null;
      isSvg: boolean;
      icoEntries: IcoEntryInfo[] | null;
    }
  | { status: "error"; name: string; error: string };

/** One rendered square: straight-alpha RGBA, ready for `new ImageData(...)`. */
export interface Frame {
  side: number;
  data: Uint8ClampedArray<ArrayBuffer>;
}

export type BatchEvent =
  | { event: "progress"; data: { done: number; total: number; name: string } }
  | {
      event: "finished";
      data: { ok: number; failed: { name: string; error: string }[]; outDir: string; cancelled: boolean };
    };

export const loadImages = (paths: string[]) => invoke<LoadResult[]>("load_images", { paths });

export const loadClipboardImage = (name: string, bytes: Uint8Array) =>
  invoke<LoadResult>("load_clipboard_image", { name, bytes: Array.from(bytes) });

export const removeImage = (id: number) => invoke<void>("remove_image", { id });

export const setSizeOverride = (id: number, size: number, path: string | null) =>
  invoke<number[]>("set_size_override", { id, size, path });

/** First frame is the 512 main preview, then every selected ICO size ascending (see Rust `preview`). */
export async function preview(id: number, settings: Settings): Promise<Frame[]> {
  const buf = await invoke<ArrayBuffer>("preview", { id, settings });
  const dv = new DataView(buf);
  const frames: Frame[] = [];
  let off = 4;
  for (let i = dv.getUint32(0, true); i > 0; i--) {
    const side = dv.getUint32(off, true);
    off += 4;
    const len = side * side * 4;
    frames.push({ side, data: new Uint8ClampedArray(buf, off, len) });
    off += len;
  }
  return frames;
}

export async function renderPngBytes(id: number, settings: Settings, size = 512): Promise<Uint8Array> {
  const buf = await invoke<ArrayBuffer>("render_png_bytes", { id, settings, size });
  return new Uint8Array(buf);
}

export const exportSingle = (
  id: number,
  path: string,
  settings: Settings,
  alsoPng: boolean,
  alsoPng1024 = false,
) => invoke<string[]>("export_single", { id, path, settings, alsoPng, alsoPng1024 });

export const exportIcns = (id: number, path: string, settings: Settings) =>
  invoke<string>("export_icns", { id, path, settings });

export const exportPack = (id: number, outDir: string, pack: PackKind, settings: Settings) =>
  invoke<string[]>("export_pack", { id, outDir, pack, settings });

export const applyFolderIcon = (id: number, folder: string, settings: Settings) =>
  invoke<string>("apply_folder_icon", { id, folder, settings });

/** Resolved Android adaptive-icon plan, mirrored from Rust `android::Analysis`. */
export interface AndroidAnalysis {
  bgMode: Exclude<AndroidBgMode, "auto">;
  bgColor: string;
  edge: "transparent" | "flat" | "varied";
  sampledColor: string;
  clippedPct: number;
  coveragePct: number;
  subjectBboxDp: [number, number, number, number] | null;
  cornersDetected: boolean;
  cornersStripped: boolean;
  mono: boolean;
  warnings: string[];
}

export interface AndroidPreview {
  analysis: AndroidAnalysis;
  /** Opaque full 108 dp composite (foreground over background). */
  composite: Frame;
  mono: Frame | null;
}

export async function androidPreview(id: number, settings: Settings): Promise<AndroidPreview> {
  const buf = await invoke<ArrayBuffer>("android_preview", { id, settings });
  const dv = new DataView(buf);
  let off = 0;
  const jsonLen = dv.getUint32(off, true);
  off += 4;
  const analysis = JSON.parse(new TextDecoder().decode(new Uint8Array(buf, off, jsonLen))) as AndroidAnalysis;
  off += jsonLen;
  const readFrame = (): Frame | null => {
    const side = dv.getUint32(off, true);
    off += 4;
    if (side === 0) return null;
    const len = side * side * 4;
    const data = new Uint8ClampedArray(buf, off, len);
    off += len;
    return { side, data };
  };
  const composite = readFrame();
  if (!composite) throw new Error("empty Android preview");
  return { analysis, composite, mono: readFrame() };
}

/** Pick (or clear, with `null`) the custom monochrome image; returns its file name. */
export const setAndroidMono = (id: number, path: string | null) =>
  invoke<string | null>("set_android_mono", { id, path });

export const exportAndroid = (id: number, outDir: string, settings: Settings) =>
  invoke<string[]>("export_android", { id, outDir, settings });

export const cancelBatch = () => invoke<void>("cancel_batch");

export function exportBatch(
  ids: number[],
  outDir: string,
  settings: Settings,
  alsoPng: boolean,
  alsoPng1024: boolean,
  itemSettings: Record<number, Settings> | null,
  onProgress: (done: number, total: number, name: string) => void,
): Promise<Extract<BatchEvent, { event: "finished" }>["data"]> {
  return new Promise((resolve, reject) => {
    const onEvent = new Channel<BatchEvent>();
    onEvent.onmessage = (m) => {
      if (m.event === "progress") onProgress(m.data.done, m.data.total, m.data.name);
      else resolve(m.data);
    };
    invoke("export_batch", {
      ids,
      outDir,
      settings,
      alsoPng,
      alsoPng1024,
      itemSettings,
      onEvent,
    }).catch(reject);
  });
}

// ---- Directory reader ----

export type ProjectKind =
  | "flutter"
  | "android"
  | "tauri"
  | "electron"
  | "browserExtension"
  | "vsCode"
  | "xcode"
  | "rustWindows"
  | "dotNet"
  | "pyInstaller"
  | "web"
  | "repoIcon";

export type TargetFormat =
  | { type: "ico"; favicon: boolean }
  | { type: "png"; side: number }
  | { type: "icns" }
  | { type: "androidRes"; base: string; round: string | null }
  | { type: "appIconSet" }
  | { type: "webManifest" };

export type TargetProfile = "asIs" | "fullBleedOpaque" | "maskable" | "android";

export interface ProjectTarget {
  id: string;
  label: string;
  relPath: string;
  format: TargetFormat;
  profile: TargetProfile;
  exists: boolean;
  defaultOn: boolean;
}

export interface ProjectDetection {
  kind: ProjectKind;
  label: string;
  evidence: string[];
  targets: ProjectTarget[];
  notes: string[];
}

export interface ProjectReport {
  root: string;
  name: string;
  primary: ProjectKind | null;
  primaryLabel: string | null;
  detections: ProjectDetection[];
  warnings: string[];
}

export interface ProjectExportOutcome {
  written: string[];
  replaced: number;
  removed: string[];
  backupDir: string | null;
  warnings: string[];
}

export const scanProject = (path: string) => invoke<ProjectReport>("scan_project", { path });

export const exportToProject = (id: number, root: string, targetIds: string[], settings: Settings) =>
  invoke<ProjectExportOutcome>("export_to_project", { id, root, targetIds, settings });
