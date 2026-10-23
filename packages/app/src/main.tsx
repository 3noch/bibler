import { render } from "solid-js/web";
import { App } from "./App.tsx";
import "./style.css";

render(() => <App />, document.getElementById("root")!);

// Installable app + offline reading (src/sw.js). Not in dev: a cache would fight hot reload.
if (import.meta.env.PROD && "serviceWorker" in navigator) {
  addEventListener("load", () => {
    // The texts it keeps for offline reading are named by `keepOffline` (src/text.ts) once one is shown.
    void navigator.serviceWorker.register(`${import.meta.env.BASE_URL}sw.js`);
  });
}
