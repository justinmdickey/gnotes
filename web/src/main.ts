import { mount } from "svelte";
import * as loro from "loro-crdt/web";
import "./app.css";
import App from "./App.svelte";

// Some Android WebViews (Hermit lite apps) report a status-bar inset while laying
// the page out below the status bar, leaving an empty band on top. Only keep the
// inset when the page really covers the screen top: then little of the screen height
// is outside the viewport.
function fitSafeTop() {
  const probe = document.createElement("div");
  probe.style.cssText = "position:fixed;visibility:hidden;padding-top:env(safe-area-inset-top)";
  document.body.append(probe);
  const inset = parseFloat(getComputedStyle(probe).paddingTop) || 0;
  probe.remove();
  const outside = screen.height - window.innerHeight;
  document.documentElement.style.setProperty("--safe-top", inset && outside >= inset ? "0px" : `${inset}px`);
}
fitSafeTop();
screen.orientation?.addEventListener("change", () => setTimeout(fitSafeTop, 300));

// Loro's WebAssembly loads with fetch, which the service worker answers from its cache, so the app
// opens with no network. (Loro's sync-XHR browser entry can't: service workers don't see sync XHR.)
// Its types leave out the init function the web entry exports as default.
const initLoro = (loro as unknown as { default: () => Promise<unknown> }).default;
export default initLoro().then(() => mount(App, { target: document.getElementById("app")! }));
