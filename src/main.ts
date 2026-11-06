import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open, save } from "@tauri-apps/plugin-dialog";
import * as api from "./api";
import { ALL_SIZES, loadPrefs, savePrefs, type Prefs, type Shape } from "./settings";

// ---- state ------------------------------------------------------------------------------
// Plain module state + explicit render calls. Anything derivable (mode, button labels,
// "last size can't be unticked") is computed in render, never stored, so it can't drift.

interface Item {
  id: number;
  name: string;
  width: number;
  height: number;
  thumb: string;
  warning: string | null;
}

const IMAGE_EXT = ["png", "jpg", "jpeg", "webp", "bmp"];
let items: Item[] = [];
let selectedId: number | null = null;
let prefs: Prefs = loadPrefs();
let batchRunning = false;

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const el = {
  drop: $("drop"),
  add: $<HTMLButtonElement>("add"),
  list: $<HTMLUListElement>("list"),
  count: $("count"),
  clear: $<HTMLButtonElement>("clear"),
  empty: $("empty"),
  previewArea: $("previewArea"),
  mainPreview: $<HTMLImageElement>("mainPreview"),
  smalls: $("smalls"),
  meta: $("meta"),
  radiusRow: $("radiusRow"),
  radius: $<HTMLInputElement>("radius"),
  radiusOut: $("radiusOut"),
  padding: $<HTMLInputElement>("padding"),
  paddingOut: $("paddingOut"),
  sizes: $("sizes"),
  alsoPng: $<HTMLInputElement>("alsoPng"),
  exportOne: $<HTMLButtonElement>("exportOne"),
  exportAll: $<HTMLButtonElement>("exportAll"),
  batch: $("batch"),
  bar: $("bar"),
  batchText: $("batchText"),
  summary: $("summary"),
  toasts: $("toasts"),
};

const selected = () => items.find((i) => i.id === selectedId) ?? null;
const stemOf = (name: string) => name.replace(/\.[^.]+$/, "") || "icon";

// ---- toasts -----------------------------------------------------------------------------

function toast(msg: string, kind: "info" | "error" = "info") {
  const t = document.createElement("div");
  t.className = `toast ${kind}`;
  const text = document.createElement("span");
  text.textContent = msg; // textContent, not innerHTML: file names are untrusted input
  const x = document.createElement("button");
  x.textContent = "×";
  x.className = "toast-x";
  x.onclick = () => t.remove();
  t.append(text, x);
  el.toasts.append(t);
  setTimeout(() => t.remove(), kind === "error" ? 12000 : 5000);
}
const errMsg = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

// ---- preview scheduling -----------------------------------------------------------------
// Single-flight with a trailing re-run: while a render is in flight, slider moves only mark
// the view dirty. When it lands we show it (it's still newer than what's on screen) and fire
// exactly one more for the latest settings. Dragging never queues dozens of renders.

let inFlight = false;
let dirty = false;

function schedulePreview() {
  dirty = true;
  if (!inFlight) void pump();
}

async function pump() {
  inFlight = true;
  while (dirty) {
    dirty = false;
    const id = selectedId;
    if (id === null) break;
    try {
      const p = await api.preview(id, prefs.settings);
      if (id === selectedId) showPreview(p); // selection may have changed while we waited
    } catch (e) {
      if (id === selectedId) toast(errMsg(e), "error");
    }
  }
  inFlight = false;
}

function showPreview(p: api.Preview) {
  el.mainPreview.src = p.main;
  const dpr = window.devicePixelRatio || 1;
  el.smalls.replaceChildren(
    ...p.small.map(([size, url]) => {
      // Show each size at *device* pixels so 16 px really is 16 screen pixels, plus an
      // integer-multiple pixelated zoom so individual pixels are judgeable.
      const zoom = Math.max(1, Math.floor((96 * dpr) / size));
      const fig = document.createElement("figure");
      const actual = new Image();
      actual.src = url;
      actual.style.width = `${size / dpr}px`;
      const big = new Image();
      big.src = url;
      big.className = "pixelated";
      big.style.width = `${(size * zoom) / dpr}px`;
      const a = document.createElement("div");
      a.className = "checker";
      a.append(actual);
      const b = document.createElement("div");
      b.className = "checker";
      b.append(big);
      const cap = document.createElement("figcaption");
      cap.textContent = `${size}px`;
      fig.append(a, b, cap);
      return fig;
    }),
  );
}

// ---- rendering --------------------------------------------------------------------------

function renderList() {
  el.list.replaceChildren(
    ...items.map((it) => {
      const li = document.createElement("li");
      li.className = it.id === selectedId ? "selected" : "";
      li.title = it.warning ?? it.name;
      li.onclick = () => select(it.id);
      const img = new Image();
      img.src = it.thumb;
      img.className = "thumb";
      const info = document.createElement("div");
      info.className = "info";
      const name = document.createElement("span");
      name.className = "name";
      name.textContent = it.name;
      const dims = document.createElement("span");
      dims.className = "muted dims";
      dims.textContent = `${it.width}×${it.height}${it.warning ? " · low-res" : ""}`;
      if (it.warning) dims.classList.add("warn");
      info.append(name, dims);
      const rm = document.createElement("button");
      rm.className = "btn ghost small rm";
      rm.textContent = "✕";
      rm.title = "Remove";
      rm.onclick = (ev) => {
        ev.stopPropagation(); // don't also select the row being removed
        void remove(it.id);
      };
      li.append(img, info, rm);
      return li;
    }),
  );
  el.count.textContent = items.length ? `${items.length} image${items.length > 1 ? "s" : ""}` : "";
  el.clear.hidden = items.length < 2;
}

function renderControls() {
  const s = prefs.settings;
  document.querySelectorAll<HTMLInputElement>('input[name="shape"]').forEach((r) => (r.checked = r.value === s.shape));
  el.radiusRow.hidden = s.shape !== "rounded";
  el.radius.value = String(s.radius);
  el.radiusOut.textContent = `${s.radius}%`;
  el.padding.value = String(s.padding);
  el.paddingOut.textContent = `${s.padding}%`;
  el.alsoPng.checked = prefs.alsoPng;
  el.sizes.querySelectorAll<HTMLInputElement>("input").forEach((cb) => {
    const n = Number(cb.value);
    cb.checked = s.sizes.includes(n);
    // Derived rule: the only remaining ticked size can't be unticked.
    cb.disabled = cb.checked && s.sizes.length === 1;
  });

  const sel = selected();
  const batch = items.length > 1; // mode is derived from the list, not a separate flag
  el.exportOne.textContent = batch ? "Export selected…" : "Export .ico…";
  el.exportOne.className = `btn ${batch ? "" : "primary"}`;
  el.exportOne.disabled = !sel || batchRunning;
  el.exportAll.hidden = !batch;
  el.exportAll.disabled = batchRunning;
  el.exportAll.textContent = batchRunning ? "Exporting…" : `Export all (${items.length})…`;

  el.empty.hidden = !!sel;
  el.previewArea.hidden = !sel;
  if (sel) {
    const side = Math.min(sel.width, sel.height);
    el.meta.textContent = `${sel.name} — ${sel.width}×${sel.height}, cropped to ${side}×${side}` + (sel.warning ? ` · ${sel.warning}` : "");
  }
}

function update(mutate: (p: Prefs) => void) {
  mutate(prefs);
  savePrefs(prefs);
  renderControls();
  schedulePreview();
}

// ---- actions ----------------------------------------------------------------------------

function select(id: number | null) {
  selectedId = id;
  if (id === null) el.mainPreview.removeAttribute("src");
  renderList();
  renderControls();
  schedulePreview();
}

async function addPaths(paths: string[]) {
  if (!paths.length) return;
  document.body.classList.add("loading");
  try {
    const results = await api.loadImages(paths);
    const failed = results.filter((r) => r.status === "error");
    const ok = results.filter((r): r is Extract<api.LoadResult, { status: "ok" }> => r.status === "ok");
    items.push(...ok.map(({ status: _s, ...it }) => it));
    if (failed.length) toast(failed.map((f) => `${f.name}: ${f.error}`).join("\n"), "error");
    if (ok.length && (selectedId === null || !selected())) select(ok[0].id);
    else {
      renderList();
      renderControls();
    }
  } catch (e) {
    toast(errMsg(e), "error");
  } finally {
    document.body.classList.remove("loading");
  }
}

async function remove(id: number) {
  const idx = items.findIndex((i) => i.id === id);
  if (idx < 0) return;
  items.splice(idx, 1);
  api.removeImage(id).catch(() => {}); // frees the cached decode; failure is harmless
  if (selectedId === id) select(items[Math.min(idx, items.length - 1)]?.id ?? null);
  else {
    renderList();
    renderControls();
  }
}

async function browse() {
  try {
    const picked = await open({ multiple: true, directory: false, filters: [{ name: "Images", extensions: IMAGE_EXT }] });
    if (picked) await addPaths(Array.isArray(picked) ? picked : [picked]);
  } catch (e) {
    toast(errMsg(e), "error");
  }
}

async function exportSelected() {
  const sel = selected();
  if (!sel) return;
  try {
    const path = await save({ defaultPath: `${stemOf(sel.name)}.ico`, filters: [{ name: "Windows icon", extensions: ["ico"] }] });
    if (!path) return; // user cancelled
    el.exportOne.disabled = true;
    const written = await api.exportSingle(sel.id, path, prefs.settings, prefs.alsoPng);
    toast(`Saved ${written.join(" and ")}`);
  } catch (e) {
    toast(`Export failed: ${errMsg(e)}`, "error");
  } finally {
    renderControls();
  }
}

async function exportAll() {
  if (batchRunning || !items.length) return;
  let dir: string | null;
  try {
    const picked = await open({ directory: true, multiple: false, title: "Choose output folder" });
    dir = Array.isArray(picked) ? (picked[0] ?? null) : picked;
  } catch (e) {
    toast(errMsg(e), "error");
    return;
  }
  if (!dir) return;

  batchRunning = true;
  renderControls();
  el.batch.hidden = false;
  el.summary.replaceChildren();
  el.bar.style.width = "0%";
  el.batchText.textContent = `Starting ${items.length} exports…`;
  try {
    const res = await api.exportBatch(items.map((i) => i.id), dir, prefs.settings, prefs.alsoPng, (done, total, name) => {
      el.bar.style.width = `${(done / total) * 100}%`;
      el.batchText.textContent = `${done} / ${total} — ${name}`;
    });
    el.bar.style.width = "100%";
    el.batchText.textContent = `Done: ${res.ok} succeeded, ${res.failed.length} failed → ${res.outDir}`;
    el.summary.replaceChildren(
      ...res.failed.map((f) => {
        const p = document.createElement("p");
        p.className = "fail";
        p.textContent = `${f.name}: ${f.error}`;
        return p;
      }),
    );
    el.batch.classList.toggle("has-fail", res.failed.length > 0);
  } catch (e) {
    el.batchText.textContent = "";
    toast(`Batch export failed: ${errMsg(e)}`, "error");
  } finally {
    batchRunning = false;
    renderControls();
  }
}

// ---- wiring -----------------------------------------------------------------------------

el.sizes.replaceChildren(
  ...ALL_SIZES.map((n) => {
    const l = document.createElement("label");
    l.className = "check";
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.value = String(n);
    cb.onchange = () =>
      update((p) => {
        const set = new Set(p.settings.sizes);
        cb.checked ? set.add(n) : set.delete(n);
        if (set.size === 0) set.add(n); // belt and braces with the disabled state
        p.settings.sizes = ALL_SIZES.filter((s) => set.has(s));
      });
    l.append(cb, ` ${n}`);
    return l;
  }),
);

document.querySelectorAll<HTMLInputElement>('input[name="shape"]').forEach((r) =>
  r.addEventListener("change", () => update((p) => (p.settings.shape = r.value as Shape))),
);
// `input` fires continuously while dragging; the single-flight pump keeps that cheap.
el.radius.addEventListener("input", () => update((p) => (p.settings.radius = Number(el.radius.value))));
el.padding.addEventListener("input", () => update((p) => (p.settings.padding = Number(el.padding.value))));
// The PNG toggle doesn't change pixels, so skip the re-render.
el.alsoPng.addEventListener("change", () => {
  prefs.alsoPng = el.alsoPng.checked;
  savePrefs(prefs);
});
el.add.onclick = () => void browse();
el.exportOne.onclick = () => void exportSelected();
el.exportAll.onclick = () => void exportAll();
el.clear.onclick = () => {
  items.forEach((i) => api.removeImage(i.id).catch(() => {}));
  items = [];
  select(null);
};

// Tauri intercepts OS drops before the DOM sees them (HTML5 drop events carry no paths), so
// listen on the webview instead.
void getCurrentWebview().onDragDropEvent((ev) => {
  const t = ev.payload.type;
  if (t === "enter" || t === "over") document.body.classList.add("dragging");
  else document.body.classList.remove("dragging");
  if (t === "drop") void addPaths(ev.payload.paths);
});

renderList();
renderControls();
