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
document.documentElement.dataset.meter = new URLSearchParams(location.search).get("meter") ?? "stepped";

mount(Overlay, { target: document.getElementById("app")! });
