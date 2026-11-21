// Palettes resolved from the bundled themes/*.toml in the Nevi repo
// ([ui] background/foreground, [syntax] groups, statusline normal mode).
// `ok` (success green) defaults to the string color; github-light's strings
// are navy, so it uses that theme's own success_fg instead.
const THEMES = {
  "onedark": { bg: "#282c34", fg: "#abb2bf", kw: "#c678dd", fn: "#61afef", ty: "#e5c07b", st: "#98c379", cm: "#5c6370", num: "#d19a66", ln: "#5c6370", bar: "#21252b", mode: "#61afef" },
  "onedark-darker": { bg: "#1f2329", fg: "#a0a8b7", kw: "#bf68d9", fn: "#4fa6ed", ty: "#e2b86b", st: "#8ebd6b", cm: "#535965", num: "#cc9057", ln: "#535965", bar: "#181b20", mode: "#4fa6ed" },
  "dracula": { bg: "#282a36", fg: "#f8f8f2", kw: "#ff79c6", fn: "#50fa7b", ty: "#8be9fd", st: "#f1fa8c", cm: "#6272a4", num: "#bd93f9", ln: "#6272a4", bar: "#21222c", mode: "#bd93f9" },
  "gruvbox": { bg: "#282828", fg: "#ebdbb2", kw: "#fb4934", fn: "#b8bb26", ty: "#fabd2f", st: "#b8bb26", cm: "#928374", num: "#d3869b", ln: "#928374", bar: "#1d2021", mode: "#83a598" },
  "nord": { bg: "#2e3440", fg: "#d8dee9", kw: "#81a1c1", fn: "#88c0d0", ty: "#8fbcbb", st: "#a3be8c", cm: "#4c566a", num: "#b48ead", ln: "#4c566a", bar: "#3b4252", mode: "#88c0d0" },
  "tokyonight": { bg: "#1a1b26", fg: "#c0caf5", kw: "#bb9af7", fn: "#7aa2f7", ty: "#7dcfff", st: "#9ece6a", cm: "#565f89", num: "#ff9e64", ln: "#3b4261", bar: "#16161e", mode: "#7aa2f7" },
  "catppuccin-mocha": { bg: "#1e1e2e", fg: "#cdd6f4", kw: "#cba6f7", fn: "#89b4fa", ty: "#f9e2af", st: "#a6e3a1", cm: "#6c7086", num: "#fab387", ln: "#6c7086", bar: "#181825", mode: "#89b4fa" },
  "rose-pine": { bg: "#191724", fg: "#e0def4", kw: "#31748f", fn: "#ebbcba", ty: "#9ccfd8", st: "#f6c177", cm: "#6e6a86", num: "#f6c177", ln: "#6e6a86", bar: "#1f1d2e", mode: "#ebbcba" },
  "solarized-dark": { bg: "#002b36", fg: "#839496", kw: "#859900", fn: "#268bd2", ty: "#b58900", st: "#2aa198", cm: "#586e75", num: "#d33682", ln: "#586e75", bar: "#073642", mode: "#268bd2" },
  "kanagawa": { bg: "#1f1f28", fg: "#dcd7ba", kw: "#957fb8", fn: "#7e9cd8", ty: "#e6c384", st: "#98bb6c", cm: "#727169", num: "#d27e99", ln: "#54546d", bar: "#16161d", mode: "#7e9cd8" },
  "monokai": { bg: "#272822", fg: "#f8f8f2", kw: "#f92672", fn: "#a6e22e", ty: "#66d9ef", st: "#e6db74", cm: "#75715e", num: "#ae81ff", ln: "#75715e", bar: "#1e1f1c", mode: "#66d9ef" },
  "everforest": { bg: "#2d353b", fg: "#d3c6aa", kw: "#e67e80", fn: "#a7c080", ty: "#dbbc7f", st: "#83c092", cm: "#859289", num: "#d699b6", ln: "#7a8478", bar: "#232a2e", mode: "#83c092" },
  "github-dark": { bg: "#0d1117", fg: "#e6edf3", kw: "#ff7b72", fn: "#d2a8ff", ty: "#79c0ff", st: "#a5d6ff", cm: "#8b949e", num: "#79c0ff", ln: "#6e7681", bar: "#161b22", mode: "#2f81f7" },
  "github-light": { bg: "#ffffff", fg: "#1f2328", kw: "#cf222e", fn: "#8250df", ty: "#0550ae", st: "#0a3069", cm: "#6e7781", num: "#0550ae", ln: "#6e7781", bar: "#f6f8fa", mode: "#0969da", ok: "#1a7f37", light: true },
  "ayu-dark": { bg: "#0b0e14", fg: "#bfbdb6", kw: "#ff8f40", fn: "#ffb454", ty: "#39bae6", st: "#aad94c", cm: "#626a73", num: "#ff8f40", ln: "#6c7380", bar: "#0d1017", mode: "#39bae6" },
  "palenight": { bg: "#292d3e", fg: "#bfc7d5", kw: "#c792ea", fn: "#82b1ff", ty: "#ffcb6b", st: "#c3e88d", cm: "#697098", num: "#f78c6c", ln: "#697098", bar: "#1e222d", mode: "#82b1ff" },
  "nightfox": { bg: "#192330", fg: "#cdcecf", kw: "#9d79d6", fn: "#719cd6", ty: "#dbc074", st: "#81b29a", cm: "#738091", num: "#f4a261", ln: "#71839b", bar: "#131a24", mode: "#719cd6" },
};
const DEFAULT_THEME = "onedark";
const STORAGE_KEY = "nevi-site-theme";

/* ---------- themes ---------- */
const root = document.documentElement;
const themeList = document.querySelector("[data-theme-list]");
const themeName = document.querySelector("[data-theme-name]");
const themeMeta = document.querySelector('meta[name="theme-color"]');

/* Several themes keep comments, line numbers or accents below 4.5:1 on purpose
   (they are meant to recede in an editor). On a page people read, every text
   color is nudged toward white (or black on light themes) just enough to reach
   4.5:1 on all the surfaces it sits on. The swatches still show the raw theme. */
const MIN_CONTRAST = 4.6; // a hair above AA so color-mix rounding can't dip below it
const toRgb = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
const toHex = (rgb) => "#" + rgb.map((v) => Math.round(v).toString(16).padStart(2, "0")).join("");
const mix = (a, b, t) => a.map((v, i) => v + (b[i] - v) * t);
function luminance(rgb) {
  const [r, g, b] = rgb.map((v) => {
    v /= 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}
function legible(color, surfaces, light) {
  const start = typeof color === "string" ? toRgb(color) : color;
  const target = light ? [0, 0, 0] : [255, 255, 255];
  for (let step = 0; step <= 20; step++) {
    const c = mix(start, target, step / 20);
    if (surfaces.every((s) => contrast(c, s) >= MIN_CONTRAST)) return toHex(c);
  }
  return toHex(target);
}

function applyTheme(name) {
  const t = THEMES[name] || THEMES[DEFAULT_THEME];
  const bg = toRgb(t.bg);
  const fg = toRgb(t.fg);
  // bg, statusline/code panels, kbd chips, and the tinted Nevi row of the table.
  const surfaces = [bg, toRgb(t.bar), mix(bg, fg, 0.06), mix(bg, toRgb(t.mode), 0.13)];
  root.style.setProperty("--bg", t.bg);
  root.style.setProperty("--bar", t.bar);
  for (const key of ["fg", "kw", "fn", "ty", "st", "cm", "num", "ln", "mode"]) {
    root.style.setProperty(`--${key}`, legible(t[key], surfaces, t.light));
  }
  root.style.setProperty("--ok", legible(t.ok || t.st, surfaces, t.light));
  root.style.setProperty("--muted", legible(mix(fg, bg, 0.22), surfaces, t.light));
  root.toggleAttribute("data-light", Boolean(t.light));
  themeMeta.content = t.bg;
  themeName.textContent = name;
  for (const btn of themeList.querySelectorAll("button")) {
    btn.setAttribute("aria-pressed", String(btn.dataset.theme === name));
  }
}

for (const [name, t] of Object.entries(THEMES)) {
  const li = document.createElement("li");
  const btn = document.createElement("button");
  btn.type = "button";
  btn.dataset.theme = name;
  btn.style.setProperty("--t-bg", t.bg);
  btn.style.setProperty("--t-fg", t.fg);
  const dots = [t.kw, t.fn, t.ty, t.st].map((c) => `<i style="background:${c}"></i>`).join("");
  btn.innerHTML = `<span>${name}</span><span class="dots" aria-hidden="true">${dots}</span>`;
  btn.addEventListener("click", () => {
    applyTheme(name);
    try { localStorage.setItem(STORAGE_KEY, name); } catch {}
  });
  li.append(btn);
  themeList.append(li);
}

const markThemeEnd = () => themeList.toggleAttribute("data-end", themeList.scrollLeft + themeList.clientWidth >= themeList.scrollWidth - 2);
themeList.addEventListener("scroll", markThemeEnd, { passive: true });
addEventListener("resize", markThemeEnd);
markThemeEnd();

let saved = null;
try { saved = localStorage.getItem(STORAGE_KEY); } catch {}
applyTheme(saved in THEMES ? saved : DEFAULT_THEME);

/* ---------- install tabs + copy ---------- */
const heroPanel = document.querySelector("#hero-cmd");
const heroSource = heroPanel.querySelector("[data-copy-source]");
const platformLabel = document.querySelector("[data-platform-label]");
const tabs = [...document.querySelectorAll('.tabs [role="tab"]')];

function selectTab(tab) {
  for (const t of tabs) {
    const on = t === tab;
    t.setAttribute("aria-selected", String(on));
    t.tabIndex = on ? 0 : -1;
  }
  // Allow line breaks after a slash (but not inside "//") so long URLs wrap
  // on phones at a path boundary instead of mid-word.
  heroSource.innerHTML = tab.dataset.cmd.replace(/\/(?!\/)/g, "/<wbr>");
  platformLabel.textContent = tab.dataset.platform;
  const copyBtn = heroPanel.querySelector("[data-copy]");
  clearTimeout(copyBtn._t);
  copyBtn.textContent = "Copy";
  copyBtn.dataset.state = "";
  heroPanel.setAttribute("aria-labelledby", tab.id);
}

tabs.forEach((tab, i) => {
  tab.addEventListener("click", () => selectTab(tab));
  // Standard tablist keys: arrows move between tabs, Home/End jump to the ends.
  tab.addEventListener("keydown", (e) => {
    const next = { ArrowRight: i + 1, ArrowLeft: i - 1, Home: 0, End: tabs.length - 1 }[e.key];
    if (next === undefined) return;
    const target = tabs[(next + tabs.length) % tabs.length];
    selectTab(target);
    target.focus();
    e.preventDefault();
    e.stopPropagation();
  });
});

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // Clipboard API is blocked on plain http and in some embeds.
    const ta = Object.assign(document.createElement("textarea"), { value: text });
    ta.style.cssText = "position:fixed;opacity:0";
    document.body.append(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    return ok;
  }
}

for (const btn of document.querySelectorAll("[data-copy]")) {
  btn.addEventListener("click", async () => {
    const text = btn.closest(".cmd").querySelector("[data-copy-source]").textContent;
    const ok = await copyText(text);
    btn.textContent = ok ? "Copied" : "Select";
    btn.dataset.state = ok ? "done" : "";
    clearTimeout(btn._t);
    btn._t = setTimeout(() => { btn.textContent = "Copy"; btn.dataset.state = ""; }, 1600);
  });
}

/* ---------- statusline ---------- */
const sections = [...document.querySelectorAll("main > section")];
const statusSection = document.querySelector("[data-status-section]");
const statusPos = document.querySelector("[data-status-pos]");
const modeEl = document.querySelector(".statusline .mode");

function currentSectionIndex() {
  const line = window.innerHeight * 0.35;
  let idx = 0;
  sections.forEach((s, i) => { if (s.getBoundingClientRect().top <= line) idx = i; });
  return idx;
}

function updateStatus() {
  statusSection.textContent = sections[currentSectionIndex()].dataset.name;
  const max = document.documentElement.scrollHeight - window.innerHeight;
  const y = window.scrollY;
  statusPos.textContent = y <= 2 ? "Top" : y >= max - 2 ? "Bot" : `${Math.round((y / max) * 100)}%`;
}

// Phones: the top bar slides away while scrolling down and comes back on any
// scroll up, so the two-row bar doesn't eat a sixth of the screen while reading.
const topbar = document.querySelector(".topbar");
const phone = matchMedia("(max-width: 720px)");
let lastY = window.scrollY;
let jumpUntil = 0;
function markJump() { jumpUntil = performance.now() + 700; }
document.addEventListener("click", (e) => { if (e.target.closest('a[href^="#"]')) markJump(); });
function updateTopbar() {
  const y = window.scrollY;
  if (phone.matches && performance.now() < jumpUntil && y > topbar.offsetHeight) {
    topbar.setAttribute("data-hidden", "");
    lastY = y;
    return;
  }
  const hide = phone.matches && y > lastY && y > topbar.offsetHeight;
  if (hide) topbar.setAttribute("data-hidden", "");
  else if (y < lastY || !phone.matches) topbar.removeAttribute("data-hidden");
  lastY = y;
}
// Keyboard focus inside the bar always brings it back.
topbar.addEventListener("focusin", () => topbar.removeAttribute("data-hidden"));

let ticking = false;
addEventListener("scroll", () => {
  if (ticking) return;
  ticking = true;
  requestAnimationFrame(() => { updateStatus(); updateTopbar(); ticking = false; });
}, { passive: true });
updateStatus();

/* ---------- vim keys on the page ---------- */
// j/k, Ctrl-d/Ctrl-u, gg/G, ]] / [[ like the editor: instant jumps, no easing.
// Two-key sequences wait 800ms for the second key.
let pending = "";
let pendingTimer;
const smooth = "instant";

function setPending(keys) {
  pending = keys;
  if (keys) modeEl.dataset.pending = keys; else delete modeEl.dataset.pending;
  clearTimeout(pendingTimer);
  if (keys) pendingTimer = setTimeout(() => setPending(""), 800);
}

// Jumps land on each section's heading, the same place the nav links go.
const headings = sections.map((s) => s.querySelector("h1, h2"));

function jumpSection(dir) {
  const i = currentSectionIndex();
  const offset = parseFloat(getComputedStyle(root).scrollPaddingTop) || 0;
  // Like Vim: [[ from the middle of a section goes to its start first.
  const pastStart = headings[i].getBoundingClientRect().top < offset - 4;
  const target = dir > 0 ? i + 1 : pastStart ? i : i - 1;
  const t = Math.max(0, Math.min(sections.length - 1, target));
  if (t === 0) scrollTo({ top: 0, behavior: smooth });
  else headings[t].scrollIntoView({ behavior: smooth });
}

addEventListener("keydown", (e) => {
  const el = e.target;
  if (e.altKey || e.metaKey || el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)) return;

  const half = window.innerHeight / 2;
  if (e.ctrlKey) {
    if (e.key === "d" || e.key === "u") markJump();
    if (e.key === "d") { scrollBy({ top: half, behavior: smooth }); e.preventDefault(); }
    if (e.key === "u") { scrollBy({ top: -half, behavior: smooth }); e.preventDefault(); }
    return;
  }

  const seq = pending + e.key;
  let handled = true;
  if (["G", "gg", "]]", "[["].includes(seq)) markJump();
  if (seq === "j") scrollBy({ top: 64 });
  else if (seq === "k") scrollBy({ top: -64 });
  else if (seq === "G") scrollTo({ top: document.documentElement.scrollHeight, behavior: smooth });
  else if (seq === "gg") scrollTo({ top: 0, behavior: smooth });
  else if (seq === "]]") jumpSection(1);
  else if (seq === "[[") jumpSection(-1);
  else if (seq === "g" || seq === "]" || seq === "[") { setPending(seq); e.preventDefault(); return; }
  else handled = false;

  setPending("");
  if (handled) e.preventDefault();
});
