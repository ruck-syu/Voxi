import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/bricolage-grotesque";
import "@fontsource/atkinson-hyperlegible/400.css";
import "@fontsource/atkinson-hyperlegible/700.css";
import "./styles.css";
import App from "./App";

// Inside the app the window is frosted glass; in a browser there is no
// desktop behind it, so the preview paints a sample one.
if ("__TAURI_INTERNALS__" in window) document.documentElement.dataset.native = "";

// The website embeds the browser build as a demo that it types into itself.
// On a touch screen a focused text field opens the on-screen keyboard, so the
// embedded demo never takes focus there. The site sets the text directly.
else if (window.self !== window.top && matchMedia("(pointer: coarse)").matches) {
  HTMLElement.prototype.focus = () => {};
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
