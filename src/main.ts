import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import * as api from "./api";
import {
  ALL_SIZES,
  ANDROID_SCALE_PRESETS,
  BUILTIN_PRESETS,
  loadPrefs,
  savePrefs,
  type AndroidAnchor,
  type AndroidBgMode,
  type AndroidMono,
  type BackdropMode,
  type BadgeCorner,
  type BadgeMode,
  type BgMode,
  type FitMode,
  type PackKind,
  type Prefs,
  type Settings,
  type Shape,
} from "./settings";

// ---- state ------------------------------------------------------------------------------

interface Item {
  id: number;
  name: string;
  width: number;
  height: number;
  thumb: string;
  warning: string | null;
  isSvg: boolean;
  icoEntries: api.IcoEntryInfo[] | null;
  overriddenSizes: number[];
  customSettings: Settings | null;
  /** File name of the custom Android monochrome image held by the backend, if any. */
  androidMonoName: string | null;
}

const IMAGE_EXT = ["png", "jpg", "jpeg", "webp", "bmp", "svg", "svgz", "tif", "tiff", "gif", "ico"];
let items: Item[] = [];
let selectedId: number | null = null;
let prefs: Prefs = loadPrefs();
let batchRunning = false;
let comparing = false;

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const el = {
  drop: $("drop"),
  add: $<HTMLButtonElement>("add"),
  list: $<HTMLUListElement>("list"),
  count: $("count"),
  clear: $<HTMLButtonElement>("clear"),
  empty: $("empty"),
  previewArea: $("previewArea"),
  backdropColor: $<HTMLInputElement>("backdropColor"),
  pixelGrid: $<HTMLInputElement>("pixelGrid"),
  compareBtn: $<HTMLButtonElement>("compareBtn"),
  copyPngBtn: $<HTMLButtonElement>("copyPngBtn"),
  mainBox: $("mainBox"),
  mainPreview: $<HTMLCanvasElement>("mainPreview"),
  taskbarStrip: $("taskbarStrip"),
  taskbarMock: $("taskbarMock"),
  smalls: $("smalls"),
  meta: $("meta"),

  presetSelect: $<HTMLSelectElement>("presetSelect"),
  savePresetBtn: $<HTMLButtonElement>("savePresetBtn"),
  deletePresetBtn: $<HTMLButtonElement>("deletePresetBtn"),
  presetSaveRow: $("presetSaveRow"),
  presetName: $<HTMLInputElement>("presetName"),
  presetSaveOk: $<HTMLButtonElement>("presetSaveOk"),
  presetSaveCancel: $<HTMLButtonElement>("presetSaveCancel"),

  itemOverrideBar: $("itemOverrideBar"),
  itemCustom: $<HTMLInputElement>("itemCustom"),
  applyAllBtn: $<HTMLButtonElement>("applyAllBtn"),

  radiusRow: $("radiusRow"),
  radius: $<HTMLInputElement>("radius"),
  radiusOut: $("radiusOut"),
  padding: $<HTMLInputElement>("padding"),
  paddingOut: $("paddingOut"),

  zoom: $<HTMLInputElement>("zoom"),
  zoomOut: $("zoomOut"),
  rotLeft: $<HTMLButtonElement>("rotLeft"),
  rotRight: $<HTMLButtonElement>("rotRight"),
  flipH: $<HTMLButtonElement>("flipH"),
  flipV: $<HTMLButtonElement>("flipV"),
  resetFrame: $<HTMLButtonElement>("resetFrame"),

  bgControls: $("bgControls"),
  bgColor1: $<HTMLInputElement>("bgColor1"),
  bgColor2Wrap: $("bgColor2Wrap"),
  bgColor2: $<HTMLInputElement>("bgColor2"),
  bgAngleRow: $("bgAngleRow"),
  bgAngle: $<HTMLInputElement>("bgAngle"),
  bgAngleOut: $("bgAngleOut"),

  borderWidth: $<HTMLInputElement>("borderWidth"),
  borderWidthOut: $("borderWidthOut"),
  borderControls: $("borderControls"),
  borderColor: $<HTMLInputElement>("borderColor"),
  borderOpacity: $<HTMLInputElement>("borderOpacity"),
  borderOpacityOut: $("borderOpacityOut"),

  shadow: $<HTMLInputElement>("shadow"),
  shadowControls: $("shadowControls"),
  shadowBlur: $<HTMLInputElement>("shadowBlur"),
  shadowBlurOut: $("shadowBlurOut"),
  shadowOpacity: $<HTMLInputElement>("shadowOpacity"),
  shadowOpacityOut: $("shadowOpacityOut"),
  shadowOffsetY: $<HTMLInputElement>("shadowOffsetY"),
  shadowOffsetYOut: $("shadowOffsetYOut"),

  badgeControls: $("badgeControls"),
  badgeCorner: $<HTMLSelectElement>("badgeCorner"),
  badgeColor: $<HTMLInputElement>("badgeColor"),
  badgeSize: $<HTMLInputElement>("badgeSize"),
  badgeSizeOut: $("badgeSizeOut"),

  sizes: $("sizes"),
  sharpen: $<HTMLInputElement>("sharpen"),
  sharpenOut: $("sharpenOut"),

  alsoPng: $<HTMLInputElement>("alsoPng"),
  alsoPng1024: $<HTMLInputElement>("alsoPng1024"),
  legacyBmp: $<HTMLInputElement>("legacyBmp"),

  exportOne: $<HTMLButtonElement>("exportOne"),
  exportAll: $<HTMLButtonElement>("exportAll"),
  cancelBatch: $<HTMLButtonElement>("cancelBatch"),
  exportIcns: $<HTMLButtonElement>("exportIcns"),
  exportTauri: $<HTMLButtonElement>("exportTauri"),
  exportAndroid: $<HTMLButtonElement>("exportAndroid"),
  androidBox: $("androidBox"),
  androidFull: $<HTMLCanvasElement>("androidFull"),
  androidShowVisible: $<HTMLInputElement>("androidShowVisible"),
  androidShowSafe: $<HTMLInputElement>("androidShowSafe"),
  androidStats: $("androidStats"),
  androidMonoWrap: $("androidMonoWrap"),
  androidMono: $<HTMLCanvasElement>("androidMono"),
  androidMasks: $("androidMasks"),
  androidWarnings: $<HTMLUListElement>("androidWarnings"),
  androidGroup: $<HTMLDetailsElement>("androidGroup"),
  androidBgColorWrap: $("androidBgColorWrap"),
  androidBgColor: $<HTMLInputElement>("androidBgColor"),
  androidSample: $<HTMLButtonElement>("androidSample"),
  androidFlatten: $<HTMLInputElement>("androidFlatten"),
  androidScale: $<HTMLInputElement>("androidScale"),
  androidScaleOut: $("androidScaleOut"),
  androidOffsetX: $<HTMLInputElement>("androidOffsetX"),
  androidOffsetXOut: $("androidOffsetXOut"),
  androidOffsetY: $<HTMLInputElement>("androidOffsetY"),
  androidOffsetYOut: $("androidOffsetYOut"),
  androidMonoAuto: $("androidMonoAuto"),
  androidMonoThreshold: $<HTMLInputElement>("androidMonoThreshold"),
  androidMonoThresholdOut: $("androidMonoThresholdOut"),
  androidMonoCustom: $("androidMonoCustom"),
  androidMonoPick: $<HTMLButtonElement>("androidMonoPick"),
  androidMonoClear: $<HTMLButtonElement>("androidMonoClear"),
  androidMonoName: $("androidMonoName"),
  androidStripCorners: $<HTMLInputElement>("androidStripCorners"),
  androidLegacy: $<HTMLInputElement>("androidLegacy"),
  exportWeb: $<HTMLButtonElement>("exportWeb"),
  applyFolder: $<HTMLButtonElement>("applyFolder"),

  batch: $("batch"),
  bar: $("bar"),
  batchText: $("batchText"),
  summary: $("summary"),
  toasts: $("toasts"),
};

const selected = () => items.find((i) => i.id === selectedId) ?? null;
const effectiveSettings = (item: Item | null = selected()): Settings => item?.customSettings ?? prefs.settings;
const stemOf = (name: string) => name.replace(/\.[^.]+$/, "") || "icon";

// ---- toasts -----------------------------------------------------------------------------

function toast(msg: string, kind: "info" | "error" = "info") {
  const t = document.createElement("div");
  t.className = `toast ${kind}`;
  const text = document.createElement("span");
  text.textContent = msg;
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
    const item = selected();
    if (!item) break;
    const base = effectiveSettings(item);
    if (prefs.backdrop === "android") {
      try {
        const p = await api.androidPreview(item.id, base);
        if (item.id === selectedId) showAndroidPreview(p);
      } catch (e) {
        if (item.id === selectedId) toast(errMsg(e), "error");
      }
      continue;
    }
    const reqSettings: Settings = comparing
      ? {
          ...base,
          shape: "none",
          padding: 0,
          bgMode: "transparent",
          borderWidth: 0,
          shadow: false,
          badgeMode: "none",
          sharpen: 0,
        }
      : base;
    try {
      const p = await api.preview(item.id, reqSettings);
      if (item.id === selectedId) showPreview(p, item);
    } catch (e) {
      if (item.id === selectedId) toast(errMsg(e), "error");
    }
  }
  inFlight = false;
}

interface SizeView {
  actual: HTMLCanvasElement;
  zoom: HTMLCanvasElement | null;
  fig: HTMLElement;
  badgeSlot: HTMLSpanElement;
}
const views = new Map<number, SizeView>();
let viewsKey = "";

function backdropClass(): string {
  return `backdrop ${prefs.backdrop}`;
}

function applyBackdropStyles() {
  const bg = prefs.backdrop === "custom" ? prefs.backdropColor : "";
  el.mainBox.className = `${backdropClass()} big`;
  el.mainBox.style.backgroundColor = bg;

  el.smalls.querySelectorAll<HTMLElement>(".backdrop").forEach((box) => {
    box.className = backdropClass();
    box.style.backgroundColor = bg;
  });

  el.smalls.classList.toggle("show-grid", prefs.pixelGrid);

  const android = prefs.backdrop === "android";
  el.androidBox.hidden = !android;
  el.mainBox.hidden = android;
  el.smalls.hidden = android;
  el.compareBtn.hidden = android;
  el.pixelGrid.parentElement!.hidden = android;

  const showTaskbar = prefs.backdrop === "dark" || prefs.backdrop === "light";
  el.taskbarStrip.hidden = !showTaskbar;
  if (showTaskbar) {
    el.taskbarMock.className = `tb-mock-bar ${prefs.backdrop}`;
  }
}

function syncSizeViews(sides: number[]) {
  const dpr = window.devicePixelRatio || 1;
  const key = `${dpr}|${sides.join(",")}`;
  if (key === viewsKey) return;
  viewsKey = key;
  views.clear();

  const makeBox = (side: number, cssWidth: number, cls = "") => {
    const c = document.createElement("canvas");
    c.width = c.height = side;
    c.style.width = c.style.height = `${cssWidth}px`;
    if (cls) c.className = cls;
    const box = document.createElement("div");
    box.className = backdropClass();
    if (prefs.backdrop === "custom") box.style.backgroundColor = prefs.backdropColor;
    box.append(c);
    return [c, box] as const;
  };

  el.smalls.replaceChildren(
    ...sides.map((side) => {
      const zoomBy = Math.max(1, Math.floor((96 * dpr) / side));
      const fig = document.createElement("figure");
      fig.title = `Click to assign a custom ${side}×${side} PNG override`;
      fig.onclick = () => void pickSizeOverride(side);

      const [actual, a] = makeBox(side, side / dpr);
      fig.append(a);

      let zoom: HTMLCanvasElement | null = null;
      if (zoomBy > 1) {
        const [z, b] = makeBox(side, (side * zoomBy) / dpr, "pixelated");
        const wrap = document.createElement("div");
        wrap.className = "zoom-wrap";
        wrap.style.setProperty("--cell", `${zoomBy / dpr}px`);
        wrap.append(z);
        b.replaceChildren(wrap);
        zoom = z;
        fig.append(b);
      }

      const cap = document.createElement("figcaption");
      cap.textContent = `${side}px`;
      const badgeSlot = document.createElement("span");
      cap.append(badgeSlot);
      fig.append(cap);

      views.set(side, { actual, zoom, fig, badgeSlot });
      return fig;
    }),
  );
  syncOverrideBadges();
}

function syncOverrideBadges() {
  const item = selected();
  const overrides = new Set(item?.overriddenSizes ?? []);
  for (const [side, v] of views.entries()) {
    v.badgeSlot.replaceChildren();
    if (overrides.has(side)) {
      const tag = document.createElement("span");
      tag.className = "ov-badge";
      tag.textContent = "PNG";
      const clr = document.createElement("button");
      clr.type = "button";
      clr.className = "ov-clear";
      clr.textContent = "×";
      clr.title = `Clear ${side}px override`;
      clr.onclick = (ev) => {
        ev.stopPropagation();
        void clearSizeOverride(side);
      };
      v.badgeSlot.append(tag, clr);
    }
  }
}

function paint(c: HTMLCanvasElement, f: api.Frame) {
  c.getContext("2d")!.putImageData(new ImageData(f.data, f.side, f.side), 0, 0);
}

function showPreview(frames: api.Frame[], item: Item) {
  const [main, ...sizes] = frames;
  if (main) paint(el.mainPreview, main);
  syncSizeViews(sizes.map((f) => f.side));
  for (const f of sizes) {
    const v = views.get(f.side);
    if (!v) continue;
    paint(v.actual, f);
    if (v.zoom) paint(v.zoom, f);
  }
  syncOverrideBadges();

  if (prefs.backdrop === "dark" || prefs.backdrop === "light") {
    const tbFrame = sizes.find((f) => f.side === 24) ?? sizes.find((f) => f.side === 32) ?? sizes[0];
    if (tbFrame) {
      const dpr = window.devicePixelRatio || 1;
      const c = document.createElement("canvas");
      c.width = c.height = tbFrame.side;
      c.style.width = c.style.height = `${tbFrame.side / dpr}px`;
      paint(c, tbFrame);
      const t1 = document.createElement("div");
      t1.className = "dummy-tile";
      const t2 = document.createElement("div");
      t2.className = "dummy-tile";
      const t3 = document.createElement("div");
      t3.className = "dummy-tile";
      el.taskbarMock.replaceChildren(t1, t2, c, t3);
    }
  }
  void item;
}

window.addEventListener("resize", () => {
  if (!viewsKey.startsWith(`${window.devicePixelRatio || 1}|`)) schedulePreview();
});


// ---- Android adaptive-icon preview ------------------------------------------------------

/** Last analysis for the selected item (drives "Sample edge"). */
let androidAnalysis: api.AndroidAnalysis | null = null;
let androidComposite: HTMLCanvasElement | null = null;
let androidMonoLayer: HTMLCanvasElement | null = null;

type MaskKind = "circle" | "squircle" | "rounded" | "teardrop";
const ANDROID_MASKS: { kind: MaskKind; label: string }[] = [
  { kind: "circle", label: "Circle" },
  { kind: "squircle", label: "Squircle" },
  { kind: "rounded", label: "Rounded square" },
  { kind: "teardrop", label: "Teardrop" },
];
const ANDROID_MASK_SIZES = [96, 48];

function frameCanvas(f: api.Frame): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = c.height = f.side;
  paint(c, f);
  return c;
}

/** Launcher mask outline over a `d`-sized square at the origin. */
function maskPath(ctx: CanvasRenderingContext2D, kind: MaskKind, d: number) {
  ctx.beginPath();
  const r = d / 2;
  switch (kind) {
    case "circle":
      ctx.arc(r, r, r, 0, Math.PI * 2);
      break;
    case "squircle": {
      // Superellipse |x|^4 + |y|^4 = 1.
      const steps = 96;
      for (let i = 0; i <= steps; i++) {
        const t = (i / steps) * Math.PI * 2;
        const c = Math.cos(t);
        const s = Math.sin(t);
        const x = r + r * Math.sign(c) * Math.abs(c) ** 0.5;
        const y = r + r * Math.sign(s) * Math.abs(s) ** 0.5;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      break;
    }
    case "rounded":
      ctx.roundRect(0, 0, d, d, d * 0.16);
      break;
    case "teardrop":
      ctx.roundRect(0, 0, d, d, [r, r, d * 0.12, r]);
      break;
  }
  ctx.closePath();
}

/** Crop the composite to the visible 72 of 108 dp and draw it through a launcher mask. */
function drawMasked(c: HTMLCanvasElement, kind: MaskKind, cssSide: number) {
  if (!androidComposite) return;
  const dpr = window.devicePixelRatio || 1;
  const d = Math.round(cssSide * dpr);
  if (c.width !== d) c.width = c.height = d;
  c.style.width = c.style.height = `${cssSide}px`;
  const ctx = c.getContext("2d")!;
  ctx.clearRect(0, 0, d, d);
  ctx.save();
  maskPath(ctx, kind, d);
  ctx.clip();
  ctx.imageSmoothingEnabled = true;
  ctx.imageSmoothingQuality = "high";
  const side = androidComposite.width;
  const inset = side / 6;
  ctx.drawImage(androidComposite, inset, inset, side - 2 * inset, side - 2 * inset, 0, 0, d, d);
  ctx.restore();
}

function drawAndroidFull() {
  const c = el.androidFull;
  const ctx = c.getContext("2d")!;
  ctx.clearRect(0, 0, c.width, c.height);
  if (!androidComposite) return;
  const n = c.width;
  ctx.drawImage(androidComposite, 0, 0, n, n);
  const inset = n / 6;
  // Dim the outer 18 dp that no launcher shows.
  ctx.save();
  ctx.fillStyle = "rgba(10, 12, 16, 0.55)";
  ctx.beginPath();
  ctx.rect(0, 0, n, n);
  ctx.rect(inset, inset, n - 2 * inset, n - 2 * inset);
  ctx.fill("evenodd");
  ctx.restore();
  if (el.androidShowVisible.checked) {
    ctx.save();
    ctx.strokeStyle = "rgba(255, 255, 255, 0.85)";
    ctx.setLineDash([6, 4]);
    ctx.lineWidth = 1;
    ctx.strokeRect(inset + 0.5, inset + 0.5, n - 2 * inset - 1, n - 2 * inset - 1);
    ctx.restore();
  }
  if (el.androidShowSafe.checked) {
    ctx.save();
    ctx.strokeStyle = "rgba(108, 140, 255, 0.95)";
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.arc(n / 2, n / 2, (n * 33) / 108, 0, Math.PI * 2);
    ctx.stroke();
    ctx.restore();
  }
}

function drawAndroidMono() {
  el.androidMonoWrap.hidden = !androidMonoLayer;
  if (!androidMonoLayer) return;
  const dpr = window.devicePixelRatio || 1;
  const d = Math.round(96 * dpr);
  const c = el.androidMono;
  if (c.width !== d) c.width = c.height = d;
  const ctx = c.getContext("2d")!;
  ctx.clearRect(0, 0, d, d);
  const side = androidMonoLayer.width;
  const inset = side / 6;
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(androidMonoLayer, inset, inset, side - 2 * inset, side - 2 * inset, 0, 0, d, d);
  // Themed icons tint the silhouette with a wallpaper-derived color.
  ctx.globalCompositeOperation = "source-in";
  ctx.fillStyle = "#c4eed0";
  ctx.fillRect(0, 0, d, d);
  ctx.globalCompositeOperation = "source-over";
}

function buildAndroidMasks() {
  if (el.androidMasks.childElementCount) return;
  for (const m of ANDROID_MASKS) {
    const fig = document.createElement("figure");
    const pair = document.createElement("div");
    pair.className = "mask-pair";
    for (const size of ANDROID_MASK_SIZES) {
      const c = document.createElement("canvas");
      c.dataset.kind = m.kind;
      c.dataset.size = String(size);
      c.title = `${m.label} · ${size}px`;
      pair.append(c);
    }
    const cap = document.createElement("figcaption");
    cap.textContent = m.label;
    fig.append(pair, cap);
    el.androidMasks.append(fig);
  }
}

function showAndroidPreview(p: api.AndroidPreview) {
  androidAnalysis = p.analysis;
  androidComposite = frameCanvas(p.composite);
  androidMonoLayer = p.mono ? frameCanvas(p.mono) : null;
  buildAndroidMasks();
  drawAndroidFull();
  el.androidMasks
    .querySelectorAll<HTMLCanvasElement>("canvas")
    .forEach((c) => drawMasked(c, c.dataset.kind as MaskKind, Number(c.dataset.size)));
  drawAndroidMono();

  const a = p.analysis;
  const auto = effectiveSettings().android.bgMode === "auto";
  const bgLine =
    a.bgMode === "color"
      ? `Background: color ${a.bgColor.toUpperCase()}${auto ? ` (auto · ${a.edge} edges)` : ""}`
      : `Background: image${auto ? " (auto · varied edges)" : ""}`;
  const lines = [
    bgLine,
    `Coverage: ${a.coveragePct.toFixed(0)}% of the visible 72 dp`,
    `Outside the 72 dp circle: ${a.clippedPct.toFixed(1)}%`,
  ];
  if (a.cornersDetected) lines.push(a.cornersStripped ? "Rounded source corners stripped" : "Rounded source corners kept");
  el.androidStats.textContent = lines.join("\n");
  el.androidWarnings.replaceChildren(
    ...a.warnings.map((w) => {
      const li = document.createElement("li");
      li.textContent = w;
      return li;
    }),
  );
  el.androidSample.disabled = false;
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
      const tags: string[] = [];
      if (it.isSvg) tags.push("SVG");
      if (it.icoEntries) tags.push(`ICO (${it.icoEntries.length})`);
      if (it.customSettings) tags.push("custom");
      if (it.warning) tags.push("low-res");
      dims.textContent = `${it.width}×${it.height}` + (tags.length ? ` · ${tags.join(" · ")}` : "");
      if (it.warning) dims.classList.add("warn");
      info.append(name, dims);

      const rm = document.createElement("button");
      rm.className = "btn ghost small rm";
      rm.textContent = "✕";
      rm.title = "Remove";
      rm.onclick = (ev) => {
        ev.stopPropagation();
        void remove(it.id);
      };
      li.append(img, info, rm);
      return li;
    }),
  );
  el.count.textContent = items.length ? `${items.length} image${items.length > 1 ? "s" : ""}` : "";
  el.clear.hidden = items.length < 2;
}

function allPresets() {
  return [...BUILTIN_PRESETS, ...prefs.presets];
}

function settingsEqual(a: Settings, b: Settings): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

function renderPresets(s: Settings) {
  const list = allPresets();
  const match = list.find((p) => settingsEqual(p.settings, s));

  el.presetSelect.replaceChildren(
    ...list.map((p) => {
      const opt = document.createElement("option");
      opt.value = p.id;
      opt.textContent = p.builtin ? p.name : `★ ${p.name}`;
      return opt;
    }),
    (() => {
      const opt = document.createElement("option");
      opt.value = "__custom__";
      opt.textContent = "Custom";
      opt.disabled = true;
      return opt;
    })(),
  );

  el.presetSelect.value = match ? match.id : "__custom__";
  el.deletePresetBtn.hidden = !match || !!match.builtin;
}

function renderControls() {
  const sel = selected();
  const s = effectiveSettings(sel);
  const batch = items.length > 1;

  renderPresets(s);

  el.itemOverrideBar.hidden = !batch || !sel;
  if (sel) {
    el.itemCustom.checked = sel.customSettings !== null;
  }

  document.querySelectorAll<HTMLInputElement>('input[name="shape"]').forEach((r) => (r.checked = r.value === s.shape));
  el.radiusRow.hidden = s.shape === "circle" || s.shape === "none";
  el.radius.value = String(s.radius);
  el.radiusOut.textContent = `${s.radius}%`;
  el.padding.value = String(s.padding);
  el.paddingOut.textContent = `${s.padding}%`;

  document.querySelectorAll<HTMLInputElement>('input[name="fitMode"]').forEach((r) => (r.checked = r.value === s.fitMode));
  el.zoom.value = String(s.zoom);
  el.zoomOut.textContent = `${s.zoom.toFixed(2)}×`;
  el.flipH.classList.toggle("active", s.flipH);
  el.flipV.classList.toggle("active", s.flipV);

  document.querySelectorAll<HTMLInputElement>('input[name="bgMode"]').forEach((r) => (r.checked = r.value === s.bgMode));
  el.bgControls.hidden = s.bgMode === "transparent";
  el.bgColor1.value = s.bgColor1;
  el.bgColor2Wrap.hidden = s.bgMode === "solid";
  el.bgColor2.value = s.bgColor2;
  el.bgAngleRow.hidden = s.bgMode !== "linear";
  el.bgAngle.value = String(s.bgAngle);
  el.bgAngleOut.textContent = `${s.bgAngle}°`;

  el.borderWidth.value = String(s.borderWidth);
  el.borderWidthOut.textContent = `${s.borderWidth}%`;
  el.borderControls.hidden = s.borderWidth <= 0;
  el.borderColor.value = s.borderColor;
  el.borderOpacity.value = String(s.borderOpacity);
  el.borderOpacityOut.textContent = `${s.borderOpacity}%`;

  el.shadow.checked = s.shadow;
  el.shadowControls.hidden = !s.shadow;
  el.shadowBlur.value = String(s.shadowBlur);
  el.shadowBlurOut.textContent = `${s.shadowBlur}%`;
  el.shadowOpacity.value = String(s.shadowOpacity);
  el.shadowOpacityOut.textContent = `${s.shadowOpacity}%`;
  el.shadowOffsetY.value = String(s.shadowOffsetY);
  el.shadowOffsetYOut.textContent = `${s.shadowOffsetY}%`;

  document.querySelectorAll<HTMLInputElement>('input[name="badgeMode"]').forEach((r) => (r.checked = r.value === s.badgeMode));
  el.badgeControls.hidden = s.badgeMode === "none";
  el.badgeCorner.value = s.badgeCorner;
  el.badgeColor.value = s.badgeColor;
  el.badgeSize.value = String(s.badgeSize);
  el.badgeSizeOut.textContent = `${s.badgeSize}%`;

  el.sizes.querySelectorAll<HTMLInputElement>("input").forEach((cb) => {
    const n = Number(cb.value);
    cb.checked = s.sizes.includes(n);
    cb.disabled = cb.checked && s.sizes.length === 1;
  });

  el.sharpen.value = String(s.sharpen);
  el.sharpenOut.textContent = `${s.sharpen}%`;

  el.alsoPng.checked = prefs.alsoPng;
  el.alsoPng1024.checked = prefs.alsoPng1024;
  el.legacyBmp.checked = s.legacyBmp;
  renderAndroidControls(s, sel);

  document.querySelectorAll<HTMLInputElement>('input[name="backdrop"]').forEach((r) => (r.checked = r.value === prefs.backdrop));
  el.backdropColor.hidden = prefs.backdrop !== "custom";
  el.backdropColor.value = prefs.backdropColor;
  el.pixelGrid.checked = prefs.pixelGrid;
  applyBackdropStyles();

  el.exportOne.textContent = batch ? "Export selected…" : "Export .ico…";
  el.exportOne.className = `btn ${batch ? "" : "primary"}`;
  el.exportOne.disabled = !sel || batchRunning;
  el.exportAll.hidden = !batch || batchRunning;
  el.exportAll.disabled = batchRunning;
  el.exportAll.textContent = `Export all (${items.length})…`;
  el.cancelBatch.hidden = !batchRunning;

  const canSingleAction = !!sel && !batchRunning;
  el.exportIcns.disabled = !canSingleAction;
  el.exportTauri.disabled = !canSingleAction;
  el.exportAndroid.disabled = !canSingleAction;
  el.exportWeb.disabled = !canSingleAction;
  el.applyFolder.disabled = !canSingleAction;
  renderProjectButton();

  el.empty.hidden = !!sel;
  el.previewArea.hidden = !sel;
  if (sel) {
    const parts: string[] = [];
    if (sel.isSvg) {
      parts.push(`${sel.name} — SVG vector (${sel.width}×${sel.height} viewBox)`);
    } else if (sel.icoEntries) {
      const summary = sel.icoEntries.map((e) => `${e.width}px ${e.isPng ? "PNG" : "BMP"}`).join(", ");
      parts.push(`${sel.name} — ICO [${summary}]`);
    } else {
      const side = Math.min(sel.width, sel.height);
      parts.push(`${sel.name} — ${sel.width}×${sel.height} (${s.fitMode === "crop" ? `cropped ${side}×${side}` : "contained"})`);
    }
    if (s.zoom !== 1 || s.panX !== 0 || s.panY !== 0) {
      parts.push(`${s.zoom.toFixed(2)}×`);
    }
    if (sel.warning) parts.push(sel.warning);
    el.meta.textContent = parts.join(" · ");
  }
}

function renderAndroidControls(s: Settings, sel: Item | null) {
  const a = s.android;
  const radio = (name: string, value: string) =>
    document.querySelectorAll<HTMLInputElement>(`input[name="${name}"]`).forEach((r) => (r.checked = r.value === value));
  radio("androidBg", a.bgMode);
  radio("androidAnchor", a.anchor);
  radio("androidMono", a.mono);
  el.androidBgColorWrap.hidden = a.bgMode === "image";
  el.androidBgColor.value = a.bgColor;
  el.androidSample.disabled = !androidAnalysis;
  el.androidFlatten.checked = a.flatten;
  el.androidScale.value = String(a.scale);
  el.androidScaleOut.textContent = `${Number(a.scale.toFixed(1))}%`;
  el.androidOffsetX.value = String(a.offsetX);
  el.androidOffsetXOut.textContent = `${a.offsetX} dp`;
  el.androidOffsetY.value = String(a.offsetY);
  el.androidOffsetYOut.textContent = `${a.offsetY} dp`;
  el.androidMonoAuto.hidden = a.mono !== "auto";
  el.androidMonoThreshold.value = String(a.monoThreshold);
  el.androidMonoThresholdOut.textContent = String(a.monoThreshold);
  el.androidMonoCustom.hidden = a.mono !== "custom";
  const monoName = sel?.androidMonoName ?? null;
  el.androidMonoName.textContent = monoName ?? "No image chosen";
  el.androidMonoClear.hidden = !monoName;
  el.androidMonoPick.disabled = !sel;
  el.androidStripCorners.checked = a.stripCorners;
  el.androidLegacy.checked = a.legacy;
}

let saveTimer = 0;
function persist() {
  clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => savePrefs(prefs), 300);
}
window.addEventListener("beforeunload", () => savePrefs(prefs));

function updateSettings(mutate: (s: Settings) => void) {
  const sel = selected();
  if (sel && sel.customSettings) {
    mutate(sel.customSettings);
    renderList();
  } else {
    mutate(prefs.settings);
    persist();
  }
  renderControls();
  schedulePreview();
}

// ---- actions ----------------------------------------------------------------------------

function select(id: number | null) {
  if (id !== selectedId) androidAnalysis = null;
  selectedId = id;
  renderList();
  renderControls();
  schedulePreview();
}

function toItem(r: Extract<api.LoadResult, { status: "ok" }>): Item {
  return {
    id: r.id,
    name: r.name,
    width: r.width,
    height: r.height,
    thumb: r.thumb,
    warning: r.warning,
    isSvg: r.isSvg,
    icoEntries: r.icoEntries,
    overriddenSizes: [],
    customSettings: null,
    androidMonoName: null,
  };
}

async function addPaths(paths: string[]) {
  if (!paths.length) return;
  document.body.classList.add("loading");
  try {
    const results = await api.loadImages(paths);
    const failed = results.filter((r) => r.status === "error");
    const ok = results.filter((r): r is Extract<api.LoadResult, { status: "ok" }> => r.status === "ok");
    items.push(...ok.map(toItem));
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

async function addClipboardBytes(name: string, bytes: Uint8Array) {
  document.body.classList.add("loading");
  try {
    const res = await api.loadClipboardImage(name, bytes);
    if (res.status === "error") {
      toast(`${res.name}: ${res.error}`, "error");
      return;
    }
    const it = toItem(res);
    items.push(it);
    select(it.id);
    toast(`Pasted ${it.name} (${it.width}×${it.height})`);
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
  api.removeImage(id).catch(() => {});
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

async function pickSizeOverride(size: number) {
  const sel = selected();
  if (!sel) return;
  try {
    const picked = await open({
      multiple: false,
      directory: false,
      title: `Choose custom ${size}×${size} icon image`,
      filters: [{ name: "Images", extensions: IMAGE_EXT }],
    });
    const path = Array.isArray(picked) ? picked[0] : picked;
    if (!path) return;
    sel.overriddenSizes = await api.setSizeOverride(sel.id, size, path);
    schedulePreview();
    toast(`Custom ${size}×${size} override set`);
  } catch (e) {
    toast(errMsg(e), "error");
  }
}

async function clearSizeOverride(size: number) {
  const sel = selected();
  if (!sel) return;
  try {
    sel.overriddenSizes = await api.setSizeOverride(sel.id, size, null);
    schedulePreview();
    toast(`Cleared ${size}×${size} override`);
  } catch (e) {
    toast(errMsg(e), "error");
  }
}

async function copyPngToClipboard() {
  const sel = selected();
  if (!sel) return;
  try {
    const bytes = await api.renderPngBytes(sel.id, effectiveSettings(sel), 512);
    const copy = new Uint8Array(bytes.byteLength);
    copy.set(bytes);
    const blob = new Blob([copy.buffer], { type: "image/png" });
    await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
    toast("Copied 512×512 PNG to clipboard");
  } catch (e) {
    toast(`Copy failed: ${errMsg(e)}`, "error");
  }
}

async function exportSelected() {
  const sel = selected();
  if (!sel) return;
  try {
    const path = await save({
      defaultPath: `${stemOf(sel.name)}.ico`,
      filters: [{ name: "Windows icon", extensions: ["ico"] }],
    });
    if (!path) return;
    el.exportOne.disabled = true;
    const written = await api.exportSingle(
      sel.id,
      path,
      effectiveSettings(sel),
      prefs.alsoPng,
      prefs.alsoPng1024,
    );
    toast(`Saved ${written.join(" and ")}`);
  } catch (e) {
    toast(`Export failed: ${errMsg(e)}`, "error");
  } finally {
    renderControls();
  }
}

async function exportIcnsSelected() {
  const sel = selected();
  if (!sel) return;
  try {
    const path = await save({
      defaultPath: `${stemOf(sel.name)}.icns`,
      filters: [{ name: "macOS icon", extensions: ["icns"] }],
    });
    if (!path) return;
    const out = await api.exportIcns(sel.id, path, effectiveSettings(sel));
    toast(`Saved ${out}`);
  } catch (e) {
    toast(`ICNS export failed: ${errMsg(e)}`, "error");
  }
}

async function exportPackSelected(pack: PackKind) {
  const sel = selected();
  if (!sel) return;
  try {
    const picked = await open({
      directory: true,
      multiple: false,
      title: `Choose folder for ${pack === "tauri" ? "Tauri" : "Web/PWA"} icon pack`,
    });
    const dir = Array.isArray(picked) ? picked[0] : picked;
    if (!dir) return;
    const written = await api.exportPack(sel.id, dir, pack, effectiveSettings(sel));
    toast(`Exported ${written.length} files to ${dir}`);
  } catch (e) {
    toast(`Pack export failed: ${errMsg(e)}`, "error");
  }
}

async function exportAndroidSelected() {
  const sel = selected();
  if (!sel) return;
  try {
    const picked = await open({
      directory: true,
      multiple: false,
      title: "Choose folder for the Android icon pack (an android folder is created inside)",
    });
    const dir = Array.isArray(picked) ? picked[0] : picked;
    if (!dir) return;
    const written = await api.exportAndroid(sel.id, dir, effectiveSettings(sel));
    toast(`Exported ${written.length} files to the android folder in ${dir}`);
  } catch (e) {
    toast(`Android export failed: ${errMsg(e)}`, "error");
  }
}

async function pickAndroidMono() {
  const sel = selected();
  if (!sel) return;
  try {
    const picked = await open({
      multiple: false,
      title: "Choose a monochrome (themed) icon image",
      filters: [{ name: "Images", extensions: IMAGE_EXT }],
    });
    const path = Array.isArray(picked) ? picked[0] : picked;
    if (!path) return;
    sel.androidMonoName = await api.setAndroidMono(sel.id, path);
    renderControls();
    schedulePreview();
  } catch (e) {
    toast(`Could not load monochrome image: ${errMsg(e)}`, "error");
  }
}

async function clearAndroidMono() {
  const sel = selected();
  if (!sel) return;
  try {
    await api.setAndroidMono(sel.id, null);
    sel.androidMonoName = null;
    renderControls();
    schedulePreview();
  } catch (e) {
    toast(errMsg(e), "error");
  }
}

async function applyToFolderSelected() {
  const sel = selected();
  if (!sel) return;
  try {
    const picked = await open({
      directory: true,
      multiple: false,
      title: "Choose Windows folder to customize icon",
    });
    const dir = Array.isArray(picked) ? picked[0] : picked;
    if (!dir) return;
    const out = await api.applyFolderIcon(sel.id, dir, effectiveSettings(sel));
    toast(`Applied folder icon to ${out}`);
  } catch (e) {
    toast(`Apply folder icon failed: ${errMsg(e)}`, "error");
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

  const itemSettings: Record<number, Settings> = {};
  for (const it of items) {
    if (it.customSettings) itemSettings[it.id] = it.customSettings;
  }
  const hasCustom = Object.keys(itemSettings).length > 0;

  batchRunning = true;
  renderControls();
  el.batch.hidden = false;
  el.summary.replaceChildren();
  el.bar.style.width = "0%";
  el.batchText.textContent = `Starting ${items.length} exports…`;
  try {
    const res = await api.exportBatch(
      items.map((i) => i.id),
      dir,
      prefs.settings,
      prefs.alsoPng,
      prefs.alsoPng1024,
      hasCustom ? itemSettings : null,
      (done, total, name) => {
        el.bar.style.width = `${(done / total) * 100}%`;
        el.batchText.textContent = `${done} / ${total} — ${name}`;
      },
    );
    el.bar.style.width = res.cancelled ? el.bar.style.width : "100%";
    el.batchText.textContent = res.cancelled
      ? `Cancelled: ${res.ok} exported, ${res.failed.length} failed → ${res.outDir}`
      : `Done: ${res.ok} succeeded, ${res.failed.length} failed → ${res.outDir}`;
    el.summary.replaceChildren(
      ...res.failed.map((f) => {
        const p = document.createElement("p");
        p.className = "fail";
        p.textContent = `${f.name}: ${f.error}`;
        return p;
      }),
    );
    el.batch.classList.toggle("has-fail", res.failed.length > 0 || res.cancelled);
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
      updateSettings((s) => {
        const set = new Set(s.sizes);
        cb.checked ? set.add(n) : set.delete(n);
        if (set.size === 0) set.add(n);
        s.sizes = ALL_SIZES.filter((v) => set.has(v));
      });
    l.append(cb, ` ${n}`);
    return l;
  }),
);

document.querySelectorAll<HTMLInputElement>('input[name="shape"]').forEach((r) =>
  r.addEventListener("change", () => updateSettings((s) => (s.shape = r.value as Shape))),
);
el.radius.addEventListener("input", () => updateSettings((s) => (s.radius = Number(el.radius.value))));
el.padding.addEventListener("input", () => updateSettings((s) => (s.padding = Number(el.padding.value))));

document.querySelectorAll<HTMLInputElement>('input[name="fitMode"]').forEach((r) =>
  r.addEventListener("change", () => updateSettings((s) => (s.fitMode = r.value as FitMode))),
);
el.zoom.addEventListener("input", () => updateSettings((s) => (s.zoom = Number(el.zoom.value))));
el.rotLeft.onclick = () => updateSettings((s) => (s.rotation = (s.rotation + 270) % 360));
el.rotRight.onclick = () => updateSettings((s) => (s.rotation = (s.rotation + 90) % 360));
el.flipH.onclick = () => updateSettings((s) => (s.flipH = !s.flipH));
el.flipV.onclick = () => updateSettings((s) => (s.flipV = !s.flipV));
el.resetFrame.onclick = () =>
  updateSettings((s) => {
    s.fitMode = "crop";
    s.zoom = 1;
    s.panX = 0;
    s.panY = 0;
    s.rotation = 0;
    s.flipH = false;
    s.flipV = false;
  });

document.querySelectorAll<HTMLInputElement>('input[name="bgMode"]').forEach((r) =>
  r.addEventListener("change", () => updateSettings((s) => (s.bgMode = r.value as BgMode))),
);
el.bgColor1.addEventListener("input", () => updateSettings((s) => (s.bgColor1 = el.bgColor1.value)));
el.bgColor2.addEventListener("input", () => updateSettings((s) => (s.bgColor2 = el.bgColor2.value)));
el.bgAngle.addEventListener("input", () => updateSettings((s) => (s.bgAngle = Number(el.bgAngle.value))));

el.borderWidth.addEventListener("input", () => updateSettings((s) => (s.borderWidth = Number(el.borderWidth.value))));
el.borderColor.addEventListener("input", () => updateSettings((s) => (s.borderColor = el.borderColor.value)));
el.borderOpacity.addEventListener("input", () => updateSettings((s) => (s.borderOpacity = Number(el.borderOpacity.value))));

el.shadow.addEventListener("change", () => updateSettings((s) => (s.shadow = el.shadow.checked)));
el.shadowBlur.addEventListener("input", () => updateSettings((s) => (s.shadowBlur = Number(el.shadowBlur.value))));
el.shadowOpacity.addEventListener("input", () => updateSettings((s) => (s.shadowOpacity = Number(el.shadowOpacity.value))));
el.shadowOffsetY.addEventListener("input", () => updateSettings((s) => (s.shadowOffsetY = Number(el.shadowOffsetY.value))));

document.querySelectorAll<HTMLInputElement>('input[name="badgeMode"]').forEach((r) =>
  r.addEventListener("change", () => updateSettings((s) => (s.badgeMode = r.value as BadgeMode))),
);
el.badgeCorner.addEventListener("change", () => updateSettings((s) => (s.badgeCorner = el.badgeCorner.value as BadgeCorner)));
el.badgeColor.addEventListener("input", () => updateSettings((s) => (s.badgeColor = el.badgeColor.value)));
el.badgeSize.addEventListener("input", () => updateSettings((s) => (s.badgeSize = Number(el.badgeSize.value))));

el.sharpen.addEventListener("input", () => updateSettings((s) => (s.sharpen = Number(el.sharpen.value))));
el.legacyBmp.addEventListener("change", () => updateSettings((s) => (s.legacyBmp = el.legacyBmp.checked)));

// Android adaptive icon
const onRadio = (name: string, fn: (v: string) => void) =>
  document
    .querySelectorAll<HTMLInputElement>(`input[name="${name}"]`)
    .forEach((r) => r.addEventListener("change", () => fn(r.value)));
onRadio("androidBg", (v) => updateSettings((s) => (s.android.bgMode = v as AndroidBgMode)));
onRadio("androidAnchor", (v) => updateSettings((s) => (s.android.anchor = v as AndroidAnchor)));
onRadio("androidMono", (v) => updateSettings((s) => (s.android.mono = v as AndroidMono)));
el.androidBgColor.addEventListener("input", () => updateSettings((s) => (s.android.bgColor = el.androidBgColor.value)));
el.androidSample.onclick = () => {
  const a = androidAnalysis;
  if (!a) return;
  updateSettings((s) => {
    s.android.bgColor = a.sampledColor.toLowerCase();
    s.android.bgMode = "color";
  });
};
el.androidFlatten.addEventListener("change", () => updateSettings((s) => (s.android.flatten = el.androidFlatten.checked)));
el.androidScale.addEventListener("input", () => updateSettings((s) => (s.android.scale = Number(el.androidScale.value))));
document.querySelectorAll<HTMLButtonElement>("[data-android-scale]").forEach((b) => {
  const key = b.dataset.androidScale as keyof typeof ANDROID_SCALE_PRESETS;
  b.onclick = () => updateSettings((s) => (s.android.scale = ANDROID_SCALE_PRESETS[key]));
});
el.androidOffsetX.addEventListener("input", () => updateSettings((s) => (s.android.offsetX = Number(el.androidOffsetX.value))));
el.androidOffsetY.addEventListener("input", () => updateSettings((s) => (s.android.offsetY = Number(el.androidOffsetY.value))));
el.androidMonoThreshold.addEventListener("input", () =>
  updateSettings((s) => (s.android.monoThreshold = Number(el.androidMonoThreshold.value))),
);
el.androidMonoPick.onclick = () => void pickAndroidMono();
el.androidMonoClear.onclick = () => void clearAndroidMono();
el.androidStripCorners.addEventListener("change", () =>
  updateSettings((s) => (s.android.stripCorners = el.androidStripCorners.checked)),
);
el.androidLegacy.addEventListener("change", () => updateSettings((s) => (s.android.legacy = el.androidLegacy.checked)));
el.androidShowVisible.addEventListener("change", drawAndroidFull);
el.androidShowSafe.addEventListener("change", drawAndroidFull);

el.alsoPng.addEventListener("change", () => {
  prefs.alsoPng = el.alsoPng.checked;
  savePrefs(prefs);
});
el.alsoPng1024.addEventListener("change", () => {
  prefs.alsoPng1024 = el.alsoPng1024.checked;
  savePrefs(prefs);
});

// Backdrop & stage controls
document.querySelectorAll<HTMLInputElement>('input[name="backdrop"]').forEach((r) =>
  r.addEventListener("change", () => {
    prefs.backdrop = r.value as BackdropMode;
    if (prefs.backdrop === "android") el.androidGroup.open = true;
    savePrefs(prefs);
    renderControls();
    schedulePreview();
  }),
);
el.backdropColor.addEventListener("input", () => {
  prefs.backdropColor = el.backdropColor.value;
  persist();
  applyBackdropStyles();
});
el.pixelGrid.addEventListener("change", () => {
  prefs.pixelGrid = el.pixelGrid.checked;
  savePrefs(prefs);
  applyBackdropStyles();
});

// Compare original hold
const setCompare = (on: boolean) => {
  if (comparing === on) return;
  comparing = on;
  el.compareBtn.classList.toggle("active", on);
  schedulePreview();
};
el.compareBtn.addEventListener("pointerdown", () => setCompare(true));
el.compareBtn.addEventListener("pointerup", () => setCompare(false));
el.compareBtn.addEventListener("pointerleave", () => setCompare(false));
el.compareBtn.addEventListener("pointercancel", () => setCompare(false));
el.copyPngBtn.onclick = () => void copyPngToClipboard();

// Interactive pan / zoom / reset on main preview canvas
let dragPan: { x: number; y: number; panX: number; panY: number } | null = null;
el.mainBox.addEventListener("pointerdown", (ev) => {
  if (ev.button !== 0 || !selected()) return;
  const s = effectiveSettings();
  el.mainBox.setPointerCapture(ev.pointerId);
  dragPan = { x: ev.clientX, y: ev.clientY, panX: s.panX, panY: s.panY };
});
el.mainBox.addEventListener("pointermove", (ev) => {
  if (!dragPan) return;
  const dx = ev.clientX - dragPan.x;
  const dy = ev.clientY - dragPan.y;
  updateSettings((s) => {
    const scale = 100 / (128 * Math.max(1, s.zoom));
    s.panX = Math.min(100, Math.max(-100, Math.round((dragPan!.panX + dx * scale) * 10) / 10));
    s.panY = Math.min(100, Math.max(-100, Math.round((dragPan!.panY + dy * scale) * 10) / 10));
  });
});
const endPan = () => {
  dragPan = null;
};
el.mainBox.addEventListener("pointerup", endPan);
el.mainBox.addEventListener("pointercancel", endPan);
el.mainBox.addEventListener(
  "wheel",
  (ev) => {
    if (!selected()) return;
    ev.preventDefault();
    const delta = ev.deltaY < 0 ? 0.1 : -0.1;
    updateSettings((s) => {
      s.zoom = Math.min(4, Math.max(0.5, Math.round((s.zoom + delta) * 100) / 100));
    });
  },
  { passive: false },
);
el.mainBox.addEventListener("dblclick", () => {
  if (!selected()) return;
  updateSettings((s) => {
    s.zoom = 1;
    s.panX = 0;
    s.panY = 0;
  });
});

// Presets
el.presetSelect.addEventListener("change", () => {
  const found = allPresets().find((p) => p.id === el.presetSelect.value);
  if (!found) return;
  updateSettings((s) => Object.assign(s, structuredClone(found.settings)));
});
el.savePresetBtn.onclick = () => {
  el.presetSaveRow.hidden = false;
  el.presetName.value = "";
  el.presetName.focus();
};
el.presetSaveCancel.onclick = () => {
  el.presetSaveRow.hidden = true;
};
const commitPreset = () => {
  const name = el.presetName.value.trim();
  if (!name) return;
  const preset = {
    id: `user:${Date.now()}`,
    name,
    builtin: false,
    settings: structuredClone(effectiveSettings()),
  };
  prefs.presets.push(preset);
  savePrefs(prefs);
  el.presetSaveRow.hidden = true;
  renderControls();
  toast(`Saved preset "${name}"`);
};
el.presetSaveOk.onclick = commitPreset;
el.presetName.addEventListener("keydown", (ev) => {
  if (ev.key === "Enter") commitPreset();
  if (ev.key === "Escape") el.presetSaveRow.hidden = true;
});
el.deletePresetBtn.onclick = () => {
  const id = el.presetSelect.value;
  const idx = prefs.presets.findIndex((p) => p.id === id);
  if (idx < 0) return;
  const [removed] = prefs.presets.splice(idx, 1);
  savePrefs(prefs);
  renderControls();
  toast(`Deleted preset "${removed.name}"`);
};

// Per-item batch settings override
el.itemCustom.addEventListener("change", () => {
  const sel = selected();
  if (!sel) return;
  sel.customSettings = el.itemCustom.checked ? structuredClone(prefs.settings) : null;
  renderList();
  renderControls();
  schedulePreview();
});
el.applyAllBtn.onclick = () => {
  const s = structuredClone(effectiveSettings());
  prefs.settings = s;
  items.forEach((it) => (it.customSettings = null));
  savePrefs(prefs);
  renderList();
  renderControls();
  schedulePreview();
  toast("Applied settings to all items");
};

// Buttons & file actions
el.add.onclick = () => void browse();
el.exportOne.onclick = () => void exportSelected();
el.exportAll.onclick = () => void exportAll();
el.cancelBatch.onclick = () => void api.cancelBatch();
el.exportIcns.onclick = () => void exportIcnsSelected();
el.exportTauri.onclick = () => void exportPackSelected("tauri");
el.exportAndroid.onclick = () => void exportAndroidSelected();
el.exportWeb.onclick = () => void exportPackSelected("web");
el.applyFolder.onclick = () => void applyToFolderSelected();
el.clear.onclick = () => {
  items.forEach((i) => api.removeImage(i.id).catch(() => {}));
  items = [];
  select(null);
};

// Clipboard paste (Ctrl+V) & keyboard shortcuts (Space compare, Ctrl+C copy)
window.addEventListener("paste", async (ev) => {
  const target = ev.target as HTMLElement | null;
  if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) return;
  const cd = ev.clipboardData;
  if (!cd) return;
  const files: File[] = [];
  for (const item of Array.from(cd.items)) {
    if (item.kind === "file" && (item.type.startsWith("image/") || item.type === "")) {
      const f = item.getAsFile();
      if (f) files.push(f);
    }
  }
  if (!files.length && cd.files.length) files.push(...Array.from(cd.files));
  if (!files.length) return;
  ev.preventDefault();
  for (const f of files) {
    const buf = new Uint8Array(await f.arrayBuffer());
    const ext = f.type === "image/svg+xml" ? "svg" : f.type.split("/")[1] || "png";
    const name = f.name && f.name !== "image.png" ? f.name : `clipboard-${Date.now()}.${ext}`;
    await addClipboardBytes(name, buf);
  }
});

window.addEventListener("keydown", (ev) => {
  const target = ev.target as HTMLElement | null;
  const editing = target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.tagName === "SELECT");
  if (editing) return;
  if (ev.code === "Space" && selected() && !ev.repeat) {
    ev.preventDefault();
    setCompare(true);
  }
  if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === "c" && selected() && !window.getSelection()?.toString()) {
    ev.preventDefault();
    void copyPngToClipboard();
  }
});
window.addEventListener("keyup", (ev) => {
  if (ev.code === "Space") setCompare(false);
});

// Tauri OS drag-drop
void getCurrentWebview().onDragDropEvent((ev) => {
  const t = ev.payload.type;
  if (t === "enter" || t === "over") document.body.classList.add("dragging");
  else document.body.classList.remove("dragging");
  if (t === "drop") void addPaths(ev.payload.paths);
});

// ---- title bar --------------------------------------------------------------------------
const win = getCurrentWindow();
const maxBtn = $<HTMLButtonElement>("winMax");
$("winMin").onclick = () => void win.minimize();
maxBtn.onclick = () => void win.toggleMaximize();
$("winClose").onclick = () => void win.close();
async function syncMaxGlyph() {
  const m = await win.isMaximized();
  maxBtn.innerHTML = m ? "&#xE923;" : "&#xE922;";
  maxBtn.title = maxBtn.ariaLabel = m ? "Restore" : "Maximize";
}
void win.onResized(() => void syncMaxGlyph());
void syncMaxGlyph();

// ---- Directory reader -------------------------------------------------------------------

const pj = {
  open: $<HTMLButtonElement>("projectOpen"),
  body: $("projectBody"),
  name: $("projectName"),
  path: $("projectPath"),
  headline: $("projectHeadline"),
  chips: $("projectChips"),
  targets: $<HTMLUListElement>("projectTargets"),
  notes: $<HTMLUListElement>("projectNotes"),
  rescan: $<HTMLButtonElement>("projectRescan"),
  close: $<HTMLButtonElement>("projectClose"),
  export: $<HTMLButtonElement>("exportProject"),
};
let project: api.ProjectReport | null = null;
/** Ticked target ids. Survives rescans for targets that are still there. */
let projectSel = new Set<string>();
let projectBusy = false;

const article = (word: string) => (/^[aeiou]/i.test(word) || /^(x|\.n)/i.test(word) ? "an" : "a");

function describeTarget(t: api.ProjectTarget): string {
  const f = t.format;
  const base =
    f.type === "ico"
      ? f.favicon
        ? "ICO 16–48"
        : "ICO"
      : f.type === "png"
        ? `PNG ${f.side} px`
        : f.type === "icns"
          ? "ICNS"
          : f.type === "androidRes"
            ? `adaptive · ${f.base}`
            : f.type === "appIconSet"
              ? "icon set"
              : "manifest";
  if (t.profile === "fullBleedOpaque") return `${base} · opaque`;
  if (t.profile === "maskable") return `${base} · maskable`;
  return base;
}

function renderProject() {
  pj.body.hidden = !project;
  if (!project) {
    renderProjectButton();
    return;
  }
  const p = project;
  pj.name.textContent = p.name;
  pj.path.textContent = p.root;
  pj.path.title = p.root;
  const label = p.primaryLabel;
  pj.headline.textContent = !label
    ? "No known project type found."
    : p.primary === "repoIcon"
      ? "This folder has a repository icon."
      : `This is ${article(label)} ${label} project.`;
  pj.chips.replaceChildren(
    ...p.detections.slice(1).map((d) => {
      const c = document.createElement("span");
      c.className = "chip";
      c.textContent = d.label;
      c.title = d.evidence.join("\n");
      return c;
    }),
  );
  pj.chips.hidden = p.detections.length < 2;

  pj.targets.replaceChildren();
  for (const d of p.detections) {
    if (!d.targets.length) continue;
    const head = document.createElement("li");
    head.className = "pt-group";
    head.textContent = d.label;
    head.title = d.evidence.join("\n");
    pj.targets.append(head);
    for (const t of d.targets) {
      const li = document.createElement("li");
      const lab = document.createElement("label");
      lab.className = "pt-row";
      lab.title = `${t.label}\n${t.relPath}`;
      const cb = document.createElement("input");
      cb.type = "checkbox";
      cb.checked = projectSel.has(t.id);
      cb.onchange = () => {
        if (cb.checked) projectSel.add(t.id);
        else projectSel.delete(t.id);
        renderProjectButton();
      };
      const text = document.createElement("span");
      text.className = "pt-text";
      const path = document.createElement("span");
      path.className = "pt-path";
      path.textContent = t.relPath;
      const meta = document.createElement("span");
      meta.className = "pt-meta muted";
      meta.textContent = describeTarget(t);
      text.append(path, meta);
      const tag = document.createElement("span");
      tag.className = `pt-tag ${t.exists ? "replace" : "new"}`;
      tag.textContent = t.exists ? "replace" : "new";
      lab.append(cb, text, tag);
      li.append(lab);
      pj.targets.append(li);
    }
  }
  const notes = [...p.warnings, ...p.detections.flatMap((d) => d.notes)];
  pj.notes.replaceChildren(
    ...notes.map((n) => {
      const li = document.createElement("li");
      li.textContent = n;
      return li;
    }),
  );
  pj.notes.hidden = !notes.length;
  renderProjectButton();
}

function renderProjectButton() {
  pj.export.hidden = !project;
  el.exportOne.classList.toggle("primary", !project && items.length <= 1);
  if (!project) return;
  const n = project.detections.flatMap((d) => d.targets).filter((t) => projectSel.has(t.id)).length;
  pj.export.textContent = `Export to ${project.name}${n ? ` (${n})` : ""}…`;
  pj.export.disabled = !selected() || batchRunning || projectBusy || n === 0;
}

/** Scan `root`. `fresh` resets the ticks to each target's default and applies the project's look. */
async function loadProject(root: string, fresh: boolean, quiet = false) {
  try {
    const report = await api.scanProject(root);
    const ids = new Set(report.detections.flatMap((d) => d.targets.map((t) => t.id)));
    if (fresh || !project || project.root !== report.root) {
      projectSel = new Set(report.detections.flatMap((d) => d.targets.filter((t) => t.defaultOn).map((t) => t.id)));
    } else {
      projectSel = new Set([...projectSel].filter((id) => ids.has(id)));
    }
    project = report;
    prefs.lastProject = report.root;
    savePrefs(prefs);
    if (fresh && (report.primary === "android" || report.primary === "flutter")) {
      prefs.backdrop = "android";
      el.androidGroup.open = true;
      renderControls();
      schedulePreview();
    }
    renderProject();
  } catch (e) {
    if (quiet) {
      prefs.lastProject = null;
      savePrefs(prefs);
    } else {
      toast(`Could not read the folder: ${errMsg(e)}`, "error");
    }
  }
}

async function openProject() {
  try {
    const picked = await open({ directory: true, multiple: false, title: "Open a project folder" });
    const dir = Array.isArray(picked) ? picked[0] : picked;
    if (dir) await loadProject(dir, true);
  } catch (e) {
    toast(errMsg(e), "error");
  }
}

async function exportProject() {
  const sel = selected();
  const p = project;
  if (!sel || !p) return;
  const chosen = p.detections.flatMap((d) => d.targets).filter((t) => projectSel.has(t.id));
  if (!chosen.length) return;
  const replacing = chosen.filter((t) => t.exists).length;
  const msg =
    `Write ${chosen.length} icon target${chosen.length === 1 ? "" : "s"} into ${p.name}?\n\n` +
    (replacing
      ? `${replacing} existing ${replacing === 1 ? "one is" : "ones are"} replaced. The originals are copied to a backup folder first.`
      : "Nothing existing is replaced.");
  if (!(await ask(msg, { title: "Export to project", kind: "info", okLabel: "Export", cancelLabel: "Cancel" }))) return;
  projectBusy = true;
  renderProjectButton();
  try {
    const out = await api.exportToProject(sel.id, p.root, chosen.map((t) => t.id), effectiveSettings(sel));
    let line = `Wrote ${out.written.length} files into ${p.name}`;
    if (out.replaced) line += ` (${out.replaced} replaced)`;
    if (out.removed.length) line += `, removed ${out.removed.length} duplicate resource${out.removed.length === 1 ? "" : "s"}`;
    if (out.backupDir) line += `. Originals backed up to ${out.backupDir}`;
    toast(line);
    for (const w of out.warnings) toast(w, "error");
  } catch (e) {
    toast(`Project export failed: ${errMsg(e)}`, "error");
  } finally {
    projectBusy = false;
    await loadProject(p.root, false);
  }
}

pj.open.onclick = () => void openProject();
pj.rescan.onclick = () => {
  if (project) void loadProject(project.root, false);
};
pj.close.onclick = () => {
  project = null;
  projectSel.clear();
  prefs.lastProject = null;
  savePrefs(prefs);
  renderProject();
};
pj.export.onclick = () => void exportProject();
if (prefs.lastProject) void loadProject(prefs.lastProject, false, true);

renderList();
renderControls();
