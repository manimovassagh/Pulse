// Pulse dev client, injected into pages served by `pulse dev`.
//
// Listens for rebuild events from the dev server and hot-swaps the
// WebAssembly module in place (global signal state is preserved by the
// loader), or just the stylesheet when only CSS changed. Shows compile errors
// in an overlay until they are fixed.
import { start } from "/pulse.js";

const OVERLAY_ID = "__pulse_error_overlay";

function showError(message) {
  let el = document.getElementById(OVERLAY_ID);
  if (!el) {
    el = document.createElement("div");
    el.id = OVERLAY_ID;
    el.setAttribute("role", "alert");
    el.style.cssText = [
      "position:fixed", "inset:0", "z-index:2147483647", "overflow:auto",
      "background:rgba(15,23,42,.94)", "color:#f8fafc", "padding:2rem",
      "font:14px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace",
    ].join(";");
    document.body.appendChild(el);
  }
  el.innerHTML = "";
  const title = document.createElement("div");
  title.textContent = "Pulse: build failed";
  title.style.cssText = "color:#f87171;font-weight:700;font-size:16px;margin-bottom:1rem";
  const pre = document.createElement("pre");
  pre.textContent = message;
  pre.style.cssText = "white-space:pre-wrap;margin:0";
  const hint = document.createElement("div");
  hint.textContent = "Fix the error and save. This page updates automatically.";
  hint.style.cssText = "color:#94a3b8;margin-top:1rem";
  el.append(title, pre, hint);
}

function hideError() {
  document.getElementById(OVERLAY_ID)?.remove();
}

/** Points the managed stylesheet link at the latest `app.css`. */
function refreshCss(version) {
  let link = document.querySelector("link[data-pulse-css]");
  if (!link) {
    link = document.createElement("link");
    link.rel = "stylesheet";
    link.setAttribute("data-pulse-css", "");
    document.head.appendChild(link);
  }
  link.href = `/app.css?v=${version}`;
}

const events = new EventSource("/__pulse/events");

events.addEventListener("css-update", (e) => {
  const { version } = JSON.parse(e.data);
  hideError();
  refreshCss(version);
  console.info(`[pulse] styles updated (build ${version})`);
});

events.addEventListener("reload", async (e) => {
  const { version, css } = JSON.parse(e.data);
  hideError();
  if (css) refreshCss(version);
  try {
    await start(`/app.wasm?v=${version}`);
    console.info(`[pulse] hot reloaded (build ${version})`);
  } catch (err) {
    console.error("[pulse] hot reload failed, reloading the page", err);
    location.reload();
  }
});

events.addEventListener("full-reload", () => location.reload());

events.addEventListener("build-error", (e) => {
  const { message } = JSON.parse(e.data);
  console.error(message);
  showError(message);
});

// After the server restarts, reload to pick up its current build.
let lost = false;
events.onerror = () => {
  lost = true;
};
events.onopen = () => {
  if (lost) location.reload();
};
