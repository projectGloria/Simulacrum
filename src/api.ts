// Typed wrappers over the Rust commands. Argument keys are camelCase; Tauri maps them to the
// snake_case Rust parameters (alsoPng → also_png).
import { Channel, invoke } from "@tauri-apps/api/core";
import type { Settings } from "./settings";

export type LoadResult =
  | { status: "ok"; id: number; name: string; width: number; height: number; thumb: string; warning: string | null }
  | { status: "error"; name: string; error: string };

export interface Preview {
  main: string; // data URL, 512 px
  small: [number, string][]; // [size, data URL]
}

export type BatchEvent =
  | { event: "progress"; data: { done: number; total: number; name: string } }
  | { event: "finished"; data: { ok: number; failed: { name: string; error: string }[]; outDir: string } };

export const loadImages = (paths: string[]) => invoke<LoadResult[]>("load_images", { paths });
export const removeImage = (id: number) => invoke<void>("remove_image", { id });
export const preview = (id: number, settings: Settings) => invoke<Preview>("preview", { id, settings });
export const exportSingle = (id: number, path: string, settings: Settings, alsoPng: boolean) =>
  invoke<string[]>("export_single", { id, path, settings, alsoPng });

/** Resolves when the batch *finishes* (not when the command returns), so callers can just await. */
export function exportBatch(
  ids: number[],
  outDir: string,
  settings: Settings,
  alsoPng: boolean,
  onProgress: (done: number, total: number, name: string) => void,
): Promise<Extract<BatchEvent, { event: "finished" }>["data"]> {
  return new Promise((resolve, reject) => {
    const onEvent = new Channel<BatchEvent>();
    onEvent.onmessage = (m) => {
      if (m.event === "progress") onProgress(m.data.done, m.data.total, m.data.name);
      else resolve(m.data);
    };
    // The command itself only validates and spawns the worker; a rejection here means the
    // batch never started (bad folder, already running).
    invoke("export_batch", { ids, outDir, settings, alsoPng, onEvent }).catch(reject);
  });
}
