import "@fontsource/barlow/300.css";
import "@fontsource/barlow/400.css";
import "@fontsource/barlow/500.css";
import "@fontsource/barlow/600.css";
import "./settings.css";
import { mount } from "svelte";
import Settings from "./Settings.svelte";

mount(Settings, { target: document.getElementById("app")! });
