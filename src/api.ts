// Typed wrappers over the Rust commands. Argument keys are camelCase; Tauri maps them to the
// snake_case Rust parameters (alsoPng → also_png).
import { Channel, invoke } from "@tauri-apps/api/core";
import type { PackKind, Settings } from "./settings";

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
