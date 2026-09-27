<script lang="ts">
  import { type ChannelKind, type Layer, type OverlayStyle, type Settings, layerToEdit, peekEditLayer, resolve } from "./model";

  let {
    settings,
    pilotId,
    kind,
    channelId = "",
    label,
    meta,
    onedit,
    onremove,
  }: {
    settings: Settings;
    pilotId: string | null;
    kind: ChannelKind;
    channelId?: string;
    label: string;
    /** A short caption next to the name — used for "last active N days ago" on a known public channel. */
    meta?: string;
    onedit: () => void;
    /** Only meaningful for a specific known channel, not a fixed kind row. */
    onremove?: () => void;
  } = $props();

  // Both pure: safe to read reactively. layerToEdit (which creates missing
  // parent objects) is only ever called from set(), i.e. on an actual edit.
  const resolved = $derived(resolve(settings, pilotId, kind, channelId));
  const editLayer = $derived(peekEditLayer(settings, pilotId, kind, channelId));

  function own<K extends keyof Layer>(field: K): boolean {
    return editLayer?.[field] !== undefined;
  }

  // The Defaults page isn't inheriting from anything a user should think
  // about — it *is* the source every character inherits from — so it never
  // shows a deviation marker or a "use default" link, even though a field it
  // edits (settings.kinds[kind]) is very often already set by
  // Settings::with_defaults(). own() above still answers "does this field
  // have a value at all", which is real and needed for checkbox/input state
  // on every page, Defaults included; this only hides the decoration.
  const showDeviation = $derived(pilotId !== null);

  function set<K extends keyof Layer>(field: K, value: Layer[K] | undefined) {
    const layer = layerToEdit(settings, pilotId, kind, channelId);
    if (value === undefined) delete layer[field];
    else layer[field] = value;
    onedit();
  }
</script>

<div class="kind-row">
  <div class="kind-head">
    <span class="kind-name">{label}{#if meta}<span class="hint">{meta}</span>{/if}</span>
    {#if onremove}<button type="button" class="revert" onclick={onremove}>Remove</button>{/if}
  </div>
  <div class="kind-body">
    <div class="field">
      <span class="flabel">{#if showDeviation && own("mode")}<span class="pip"></span>{/if}Mode</span>
      <div class="segmented">
        <button type="button" class:sel={resolved.mode.value === "nothing"} data-mode={resolved.mode.value === "nothing" ? "nothing" : undefined} onclick={() => set("mode", "nothing")}>Nothing</button>
        <button type="button" class:sel={resolved.mode.value === "mentions"} data-mode={resolved.mode.value === "mentions" ? "mentions" : undefined} onclick={() => set("mode", "mentions")}>Mentions</button>
        <button type="button" class:sel={resolved.mode.value === "everything"} data-mode={resolved.mode.value === "everything" ? "everything" : undefined} onclick={() => set("mode", "everything")}>Everything</button>
      </div>
      {#if showDeviation && own("mode")}<button type="button" class="revert" onclick={() => set("mode", undefined)}>↺ use default</button>{/if}
    </div>

    <div class="field">
      <span class="flabel">{#if showDeviation && own("style")}<span class="pip"></span>{/if}Style</span>
      <select
        value={resolved.style.value ?? "auto"}
        onchange={(e) => set("style", e.currentTarget.value === "auto" ? undefined : (e.currentTarget.value as OverlayStyle))}
      >
        <option value="auto">Auto</option>
        <option value="beacon">Beacon</option>
        <option value="panel">Panel</option>
        <option value="strip">Strip</option>
      </select>
      {#if showDeviation && own("style")}<button type="button" class="revert" onclick={() => set("style", undefined)}>↺ use default</button>{/if}
    </div>

    <div class="field">
      <span class="flabel">{#if showDeviation && own("rateCap")}<span class="pip"></span>{/if}Rate cap</span>
      <label class="check">
        <input type="checkbox" checked={own("rateCap")} onchange={(e) => set("rateCap", e.currentTarget.checked ? { perMinute: 6, over: "fold" } : undefined)} />
        {#if own("rateCap")}
          <input
            class="num"
            type="number"
            min="1"
            value={editLayer?.rateCap?.perMinute ?? 6}
            oninput={(e) => set("rateCap", { perMinute: Math.max(1, Number(e.currentTarget.value) || 1), over: "fold" })}
          />/min
        {:else if resolved.caps.length > 0}
          default: {resolved.caps[0].cap.perMinute}/min
        {:else}
          uncapped
        {/if}
      </label>
    </div>

    <div class="field">
      <span class="flabel">{#if showDeviation && own("sound")}<span class="pip"></span>{/if}Sound</span>
      <label class="check">
        <input type="checkbox" checked={resolved.sound.value} onchange={(e) => set("sound", e.currentTarget.checked)} />
        On
      </label>
      {#if showDeviation && own("sound")}<button type="button" class="revert" onclick={() => set("sound", undefined)}>↺ use default</button>{/if}
    </div>
  </div>
</div>
