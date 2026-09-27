export type Style = "panel" | "strip" | "beacon";
export type Tone = "mention" | "keyword" | "always";

/** Mirrors `OverlayAlert` in src-tauri/src/overlay.rs. */
export interface Alert {
  id: number;
  style: Style;
  pilot: string;
  /** The pilot's accent color (CSS). */
  accent: string;
  channel: string;
  sender: string;
  text: string;
  /** Plain-language reason, e.g. "Mentioned you". */
  reason: string;
  tone: Tone;
  lifetimeMs: number;
  /** Lines folded into this alert; the badge shows when above 1. */
  count: number;
}

export interface Fold {
  pilot: string;
  channel: string;
}
