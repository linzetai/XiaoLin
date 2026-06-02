import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { QuickActionBar } from "./components/QuickActionBar";
import "./index.css";

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
