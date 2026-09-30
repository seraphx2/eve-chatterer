// Mirrors core/src/{prefs,settings,pilots,channel}.rs. Field names match the
// JSON the backend sends (camelCase; see the #[serde(rename_all)] attributes
// added there). Keep this in sync by hand when the Rust types change.

export type Mode = "nothing" | "mentions" | "everything";
export type DeliveryMode = "auto" | "overlay" | "toast" | "both" | "sound_only";
export type Suppression = "focused_only" | "visible_on_screen" | "allow_all";
export type OverlayStyle = "panel" | "strip" | "beacon";
export type OverCap = "drop" | "fold";
export type ChannelKind = "local" | "corp" | "alliance" | "fleet" | "private" | "public" | "unknown";

export const CHANNEL_KINDS: ChannelKind[] = ["local", "corp", "fleet", "private", "alliance", "public"];

export const CHANNEL_KIND_LABEL: Record<ChannelKind, string> = {
  private: "Private messages",
  fleet: "Fleet",
  corp: "Corp",
  alliance: "Alliance",
  local: "Local",
  public: "Public channels",
  unknown: "Unrecognized channels",
};

/** Fleet and private conversation ids are new every time; only these kinds can be configured by a specific channel id. */
export function hasStableId(kind: ChannelKind): boolean {
  return kind !== "fleet" && kind !== "private";
}

export interface RateCap {
  perMinute: number;
  over: OverCap;
}

export type TrackedKind = "keyword" | "regex";

/** A tracked keyword or regex. Mirrors `core::settings::TrackedRule` — stored
 * once per character (or once for Defaults), not once per channel layer;
 * which channels it applies to is a property of the entry (`onlyIn`), not of
 * where the list lives (owner decision 2026-09-27, see docs/DESIGN.md). */
export interface TrackedRule {
  text: string;
  kind: TrackedKind;
  /** Empty or absent means every channel kind. */
  onlyIn?: ChannelKind[];
  evenWhenMuted: boolean;
}

/** One settings layer. Every field is optional: unset means "inherit". */
export interface Layer {
  mode?: Mode;
  ownName?: boolean;
  /** Only ever consulted on the global layer or a character's own base layer — see `TrackedRule`. */
  tracked?: TrackedRule[];
  ignoreOwnMessages?: boolean;
  ignoreSystem?: boolean;
  systemSenders?: string[];
  ignoreSenders?: string[];
  alwaysSenders?: string[];
  /** Names this layer puts back to normal, undoing an entry inherited from Defaults (sender lists merge by name). */
  normalSenders?: string[];
  delivery?: DeliveryMode;
  suppression?: Suppression;
  style?: OverlayStyle;
  sound?: boolean;
  rateCap?: RateCap;
}

export function emptyLayer(): Layer {
  return {};
}

export interface PilotSettings {
  base: Layer;
  kinds: Partial<Record<ChannelKind, Layer>>;
  channels: Record<string, Layer>;
}

export function emptyPilotSettings(): PilotSettings {
  return { base: {}, kinds: {}, channels: {} };
}

export interface Settings {
  global: Layer;
  kinds: Partial<Record<ChannelKind, Layer>>;
  channels: Record<string, Layer>;
  pilots: Record<string, PilotSettings>;
  /** Which sound plays (core/src/audio.rs). Always present: Rust fills in its defaults. */
  audio: AudioSettings;
  /** App-wide General page options (core/src/settings.rs `GeneralSettings`). */
  general: GeneralSettings;
}

export interface GeneralSettings {
  /** Toggles overlay reposition mode. Changed only through `set_reposition_hotkey`, which registers it first. */
  repositionHotkey: string;
}

export type AudioMode = "off" | "shared" | "per_character";

export interface AudioSettings {
  mode: AudioMode;
  /** Absent means the built-in sound. */
  sharedFile?: string;
  /** By character id. */
  pilotFiles?: Record<string, string>;
  /** 0 to 100. */
  volume: number;
  /** Quiet time after a sound; mentions ignore it. */
  cooldownSecs: number;
}

export interface KnownChannel {
  name: string;
  kind: ChannelKind;
  /** Unix seconds; the entry itself is otherwise kept forever, so this is the only signal for "abandoned". */
  lastSeen: number;
}

/** A short "last active" caption, e.g. "today", "3 days ago". */
export function relativeTime(unixSeconds: number): string {
  const days = Math.floor((Date.now() / 1000 - unixSeconds) / 86400);
  if (days <= 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 30) return `${days} days ago`;
  const months = Math.floor(days / 30);
  if (months < 12) return `${months} month${months === 1 ? "" : "s"} ago`;
  const years = Math.floor(months / 12);
  return `${years} year${years === 1 ? "" : "s"} ago`;
}

/** This character's saved overlay position/width, set by dragging it in reposition mode (the reposition hotkey in-game, Ctrl+Alt+O by default). */
export interface OverlayPlacement {
  monitorLeft: number;
  monitorTop: number;
  x: number;
  y: number;
  width: number;
}

export interface Pilot {
  id: string;
  name: string;
  live: boolean;
  firstSeen: number;
  channels: Record<string, KnownChannel>;
  /** This character's own Strip-badge tag, if it has set one; absent means "derive one from the name". */
  tag?: string;
  /** Absent means "use the default centered placement". */
  placement?: OverlayPlacement;
  /** As named in the latest Corp / Alliance logs (core/src/pilots.rs `Membership`). */
  corp?: Membership;
  alliance?: Membership;
}

export interface Membership {
  name: string;
  /** Unix seconds the log session naming it began (a new one each login). */
  session: number;
}

/**
 * The character's corp and alliance. EVE simply stops writing an Alliance log
 * once a corp leaves its alliance, so the stored alliance only counts if it
 * came from the same login as the corp (both logs start within moments).
 */
export function affiliation(p: Pilot): { corp?: string; alliance?: string } {
  const corp = p.corp?.name;
  const current = p.alliance && (!p.corp || p.alliance.session >= p.corp.session - 600);
  return { corp, alliance: current ? p.alliance!.name : undefined };
}

export interface SettingsData {
  settings: Settings;
  pilots: Pilot[];
  /** Ids of characters with an EVE client running right now. (`Pilot.live` only means "has ever played".) */
  online: string[];
}

const MAX_TAG_LEN = 5;

/** Mirrors `eve_chatterer_core::pilots::tag_from_name`, for a live preview before the round trip to Rust. */
export function deriveTag(name: string): string {
  return name
    .split(/\s+/)
    .filter((w) => w.length > 0)
    .map((w) => w[0])
    .join("")
    .slice(0, MAX_TAG_LEN)
    .toUpperCase();
}

// ---------------------------------------------------------------------------
// Resolution: mirrors Settings::resolve() in core/src/settings.rs.
// ---------------------------------------------------------------------------

/** Identifies which settings layer a resolved value (or override) came from. */
export type LayerRef =
  | { at: "global" }
  | { at: "kind"; kind: ChannelKind }
  | { at: "channel"; channelId: string }
  | { at: "pilot" }
  | { at: "pilotKind"; kind: ChannelKind }
  | { at: "pilotChannel"; channelId: string };

export function layerRefEquals(a: LayerRef, b: LayerRef): boolean {
  if (a.at !== b.at) return false;
  if (a.at === "kind" && b.at === "kind") return a.kind === b.kind;
  if (a.at === "channel" && b.at === "channel") return a.channelId === b.channelId;
  if (a.at === "pilotKind" && b.at === "pilotKind") return a.kind === b.kind;
  if (a.at === "pilotChannel" && b.at === "pilotChannel") return a.channelId === b.channelId;
  return true;
}

/** A resolved value plus where it came from; `layer: null` means no layer set it — the built-in default. */
export interface Resolved<T> {
  value: T;
  layer: LayerRef | null;
}

export interface ResolvedPrefs {
  mode: Resolved<Mode>;
  ownName: Resolved<boolean>;
  ignoreOwnMessages: Resolved<boolean>;
  ignoreSystem: Resolved<boolean>;
  systemSenders: Resolved<string[]>;
  ignoreSenders: Resolved<string[]>;
  alwaysSenders: Resolved<string[]>;
  delivery: Resolved<DeliveryMode>;
  suppression: Resolved<Suppression>;
  /** `null` lets the overlay's reason pick the style; still "resolved", just to no forced value. */
  style: Resolved<OverlayStyle | null>;
  sound: Resolved<boolean>;
  /** Every layer that sets a cap, in order — all of them apply (core/src/governor.rs), not just the last. */
  caps: { layer: LayerRef; cap: RateCap }[];
}

const RULESET_DEFAULT = {
  ownName: true,
  ignoreOwnMessages: true,
  ignoreSystem: true,
  systemSenders: ["EVE System"],
  ignoreSenders: [] as string[],
  alwaysSenders: [] as string[],
};

/** The layers that apply to (pilotId, kind, channelId), least to most specific — same order as Rust's `layers()`. */
export function layersFor(s: Settings, pilotId: string | null, kind: ChannelKind, channelId: string): [LayerRef, Layer][] {
  const byId = hasStableId(kind) && channelId !== "";
  const out: [LayerRef, Layer][] = [[{ at: "global" }, s.global]];
  const k = s.kinds[kind];
  if (k) out.push([{ at: "kind", kind }, k]);
  if (byId) {
    const c = s.channels[channelId];
    if (c) out.push([{ at: "channel", channelId }, c]);
  }
  const p = pilotId ? s.pilots[pilotId] : undefined;
  if (p) {
    out.push([{ at: "pilot" }, p.base]);
    const pk = p.kinds[kind];
    if (pk) out.push([{ at: "pilotKind", kind }, pk]);
    if (byId) {
      const pc = p.channels[channelId];
      if (pc) out.push([{ at: "pilotChannel", channelId }, pc]);
    }
  }
  return out;
}

function pick<K extends keyof Layer>(
  layers: [LayerRef, Layer][],
  field: K,
  fallback: NonNullable<Layer[K]>,
): Resolved<NonNullable<Layer[K]>> {
  let result: Resolved<NonNullable<Layer[K]>> = { value: fallback, layer: null };
  for (const [ref, layer] of layers) {
    const v = layer[field];
    if (v !== undefined) result = { value: v as NonNullable<Layer[K]>, layer: ref };
  }
  return result;
}

/** Style resolves to `null` ("Default" in the UI) rather than some forced
 * value when nothing sets it anywhere, so it can't go through `pick`'s NonNullable fallback. */
function pickStyle(layers: [LayerRef, Layer][]): Resolved<OverlayStyle | null> {
  let result: Resolved<OverlayStyle | null> = { value: null, layer: null };
  for (const [ref, layer] of layers) {
    if (layer.style !== undefined) result = { value: layer.style, layer: ref };
  }
  return result;
}

export function resolve(s: Settings, pilotId: string | null, kind: ChannelKind, channelId: string): ResolvedPrefs {
  const layers = layersFor(s, pilotId, kind, channelId);
  const caps = layers.filter(([, l]) => l.rateCap !== undefined).map(([ref, l]) => ({ layer: ref, cap: l.rateCap! }));
  return {
    mode: pick(layers, "mode", "mentions"),
    ownName: pick(layers, "ownName", RULESET_DEFAULT.ownName),
    ignoreOwnMessages: pick(layers, "ignoreOwnMessages", RULESET_DEFAULT.ignoreOwnMessages),
    ignoreSystem: pick(layers, "ignoreSystem", RULESET_DEFAULT.ignoreSystem),
    systemSenders: pick(layers, "systemSenders", RULESET_DEFAULT.systemSenders),
    ignoreSenders: pick(layers, "ignoreSenders", RULESET_DEFAULT.ignoreSenders),
    alwaysSenders: pick(layers, "alwaysSenders", RULESET_DEFAULT.alwaysSenders),
    delivery: pick(layers, "delivery", "auto"),
    suppression: pick(layers, "suppression", "focused_only"),
    style: pickStyle(layers),
    sound: pick(layers, "sound", false),
    caps,
  };
}

/** Read-only: the layer this scope's edits live in, if it already exists.
 * Pure — never creates anything, safe to call from a `$derived`. */
export function peekEditLayer(s: Settings, pilotId: string | null, kind: ChannelKind, channelId: string): Layer | undefined {
  const byId = hasStableId(kind) && channelId !== "";
  if (pilotId === null) return byId ? s.channels[channelId] : s.kinds[kind];
  const p = s.pilots[pilotId];
  if (!p) return undefined;
  return byId ? p.channels[channelId] : p.kinds[kind];
}

// ---------------------------------------------------------------------------
// The kind-independent "base" layer: global, or one character's own base
// layer. Tracked keywords/patterns are edited here, not per channel kind —
// "watch for this phrase" is a character-level concern, with each entry
// carrying its own channel scope instead (`TrackedRule.onlyIn`).
// ---------------------------------------------------------------------------

export function peekBaseLayer(s: Settings, pilotId: string | null): Layer | undefined {
  return pilotId === null ? s.global : s.pilots[pilotId]?.base;
}

export function baseLayerToEdit(s: Settings, pilotId: string | null): Layer {
  return pilotId === null ? s.global : (s.pilots[pilotId] ??= emptyPilotSettings()).base;
}

export function resolveBaseField<K extends keyof Layer>(
  s: Settings,
  pilotId: string | null,
  field: K,
  fallback: NonNullable<Layer[K]>,
): Resolved<NonNullable<Layer[K]>> {
  let result: Resolved<NonNullable<Layer[K]>> = { value: fallback, layer: null };
  const g = s.global[field];
  if (g !== undefined) result = { value: g as NonNullable<Layer[K]>, layer: { at: "global" } };
  const own = pilotId ? s.pilots[pilotId]?.base[field] : undefined;
  if (own !== undefined) result = { value: own as NonNullable<Layer[K]>, layer: { at: "pilot" } };
  return result;
}

/** The layer object to write an override into for (pilotId, kind, channelId)
 * — creating any missing parents. Mutates `s` in place; call only from an
 * actual edit (e.g. inside an event handler), never from a `$derived`. */
export function layerToEdit(s: Settings, pilotId: string | null, kind: ChannelKind, channelId: string): Layer {
  const byId = hasStableId(kind) && channelId !== "";
  if (pilotId === null) {
    if (byId) return (s.channels[channelId] ??= {});
    return (s.kinds[kind] ??= {});
  }
  const p = (s.pilots[pilotId] ??= emptyPilotSettings());
  if (byId) return (p.channels[channelId] ??= {});
  return (p.kinds[kind] ??= {});
}
