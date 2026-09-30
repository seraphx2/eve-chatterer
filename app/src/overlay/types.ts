export type Style = "panel" | "strip" | "beacon";
export type Tone = "mention" | "keyword" | "always";

/** Mirrors `OverlayAlert` in src-tauri/src/overlay.rs. */
export interface Alert {
  id: number;
  style: Style;
  pilot: string;
  /** The Strip style's badge text: the pilot's own tag, or one derived from its name. */
  tag: string;
  /** The pilot's accent color (CSS). */
  accent: string;
  channel: string;
  /** The log's channel id; folds match on it since labels can repeat. */
  channelId: string;
  sender: string;
  text: string;
  /** Plain-language reason, e.g. "Mentioned you". */
  reason: string;
  tone: Tone;
  lifetimeMs: number;
  /** Lines folded into this alert; the badge shows when above 1. */
  count: number;
  /** This window's alerts grow upward (its box sits in the lower half of the game). */
  stackUp: boolean;
}

export interface Fold {
  pilot: string;
  channelId: string;
}

/** Mirrors `RepositionInfo` in src-tauri/src/overlay.rs. */
export interface RepositionInfo {
  name: string;
  tag: string;
  accent: string;
}
