import { render } from "solid-js/web";
import { App } from "./App";
import { registerServiceWorker } from "./pwa";
import "./styles.css";
import { initSystemBars } from "./systemBars";
import { initTheme } from "./theme";

const root = document.getElementById("root");
if (!root) {
  throw new Error("missing #root element");
}
initSystemBars();
initTheme();
render(() => <App />, root);
registerServiceWorker();
