import "@fontsource/barlow/300.css";
import "@fontsource/barlow/400.css";
import "@fontsource/barlow/500.css";
import "@fontsource/barlow/600.css";
import "./overlay.css";
import { mount } from "svelte";
import Overlay from "./Overlay.svelte";

// How the lifetime meter animates: "smooth" (every frame), "stepped" (a few
// updates per second, far less work for the desktop compositor over a game),
// or "off" (static). Set by the host via the page URL.
const params = new URLSearchParams(location.search);
document.documentElement.dataset.meter = params.get("meter") ?? "smooth";
// Measurement only: effects to switch off, e.g. "noshadow noarrive nopulse".
document.documentElement.dataset.fx = params.get("fx") ?? "";

mount(Overlay, { target: document.getElementById("app")! });
