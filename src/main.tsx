import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { PendingPreview } from "./Pending";
import { PlanPreview } from "./PlanPreview";
import { VoicePreview } from "./VoicePreview";
import { PlannerWindow } from "./Planner";
import { PopoverPreview, PopoverWindow } from "./Popover";
import { inApp } from "./bridge";
import "./styles.css";

const popover = location.search.includes("popover");

// The popover's window is see-through around the panel
if (popover) {
  document.documentElement.classList.add("popover-page");
}

// In a plain browser there's no native window around the page, so the stylesheet draws one (see `html.browser`)
if (!inApp) {
  document.documentElement.classList.add("browser");
}

// How wide a scroll bar beside the content is here: zero where they float over it. The page's right padding gives
// that width back.
{
  const probe = document.createElement("div");
  probe.style.cssText = "position:absolute;top:-999px;width:100px;height:100px;overflow:scroll";
  document.body.appendChild(probe);
  document.documentElement.style.setProperty("--scroll-bar-width", `${probe.offsetWidth - probe.clientWidth}px`);
  probe.remove();
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {location.search.includes("voice") && !inApp ? (
      <VoicePreview />
    ) : location.search.includes("planner") ? (
      <PlannerWindow />
    ) : location.search.includes("plan") && !inApp ? (
      <PlanPreview />
    ) : location.search.includes("pending") && !inApp ? (
      <PendingPreview />
    ) : !popover ? (
      <App />
    ) : inApp ? (
      <PopoverWindow />
    ) : (
      <PopoverPreview />
    )}
  </React.StrictMode>,
);
