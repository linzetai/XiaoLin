import React from "react";
import ReactDOM from "react-dom/client";
import "./i18n";
import App from "./App";
import { QuickActionBar } from "./components/QuickActionBar";
import "./index.css";

// Suppress ResizeObserver loop errors — a benign browser behavior that
// Vite's HMR client treats as an unhandled error, crashing the dev server.
const RESIZE_OBSERVER_MSG = "ResizeObserver loop";
window.addEventListener("error", (e) => {
  if (e.message?.includes(RESIZE_OBSERVER_MSG)) {
    e.stopImmediatePropagation();
  }
});

function Root() {
  const path = window.location.pathname;
  if (path === "/quick-action") {
    return <QuickActionBar />;
  }
  return <App />;
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);
