import { render } from "solid-js/web";
import { App } from "./App.tsx";
import "./style.css";

render(() => <App />, document.getElementById("root")!);

// Installable app + offline reading (src/sw.js). Not in dev: a cache would fight hot reload.
if (import.meta.env.PROD && "serviceWorker" in navigator) {
  addEventListener("load", () => {
    void navigator.serviceWorker.register(`${import.meta.env.BASE_URL}sw.js`);
    // Then download the whole text for offline reading, in the background (see sw.js).
    void navigator.serviceWorker.ready.then((r) => r.active?.postMessage("precache-text"));
  });
}
