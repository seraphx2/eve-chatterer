import "@fontsource/barlow/300.css";
import "@fontsource/barlow/400.css";
import "@fontsource/barlow/500.css";
import "@fontsource/barlow/600.css";
import "./overlay.css";
import { mount } from "svelte";
import Overlay from "./Overlay.svelte";

mount(Overlay, { target: document.getElementById("app")! });
