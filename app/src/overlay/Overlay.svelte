<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
  import Beacon from "./Beacon.svelte";
  import Panel from "./Panel.svelte";
  import Strip from "./Strip.svelte";
  import type { Alert, Fold, RepositionInfo, Style, Tone } from "./types";

  /** How many alerts may be on screen at once in this window. */
  const MAX_VISIBLE = 5;
  /** A Beacon takes the top of the stack, Strips sit beneath. */
  const RANK: Record<Style, number> = { beacon: 0, panel: 1, strip: 2 };
  const TONE: Record<Tone, string> = {
    mention: "var(--mention)",
    keyword: "var(--keyword)",
    always: "var(--always)",
  };

  let alerts = $state<Alert[]>([]);
  const timers = new Map<number, ReturnType<typeof setTimeout>>();

  // Reposition mode ("Overlay reposition & resize", docs/DESIGN.md):
  // real alerts keep queuing in `alerts` in the background (nothing is
  // lost), but the stack is hidden in favor of a draggable/resizable
  // placeholder while this is set.
  let reposition = $state<RepositionInfo | null>(null);

  function remove(id: number) {
    clearTimeout(timers.get(id));
    timers.delete(id);
    alerts = alerts.filter((a) => a.id !== id);
  }

  function add(a: Alert) {
    alerts = [...alerts, a].sort((x, y) => RANK[x.style] - RANK[y.style] || y.id - x.id);
    timers.set(a.id, setTimeout(() => remove(a.id), a.lifetimeMs));
    // Over the limit: drop the oldest of the least important style.
    while (alerts.length > MAX_VISIBLE) {
      const victim = [...alerts].sort((x, y) => RANK[y.style] - RANK[x.style] || x.id - y.id)[0];
      remove(victim.id);
    }
  }

  /** A line past the pilot's rate cap: bump the count of the latest matching alert. */
  function fold(f: Fold) {
    const target = [...alerts].filter((a) => a.pilot === f.pilot && a.channelId === f.channelId).sort((x, y) => y.id - x.id)[0];
    if (target) target.count += 1;
  }

  /** A stand-in alert, so the placeholder previews exactly what a real one will look like at this width. */
  function sample(r: RepositionInfo): Alert {
    return {
      id: 0,
      style: "panel",
      pilot: r.name,
      tag: r.tag,
      accent: r.accent,
      channel: "Local",
      channelId: "local",
      sender: "Example Pilot",
      text: "This is where alerts for this character will appear.",
      reason: "Preview",
      tone: "keyword",
      lifetimeMs: 1e9,
      count: 1,
      stackUp: false,
    };
  }

  // Drag/resize: report pointer deltas (screen coordinates, so they stay
  // stable while the window itself moves under the pointer) and let the host
  // move the window, clamped inside the game. At most one call per frame.
  let gesture: { kind: "move" | "resize"; sx: number; sy: number } | null = null;
  let pending: { dx: number; dy: number } | null = null;
  const label = getCurrentWebviewWindow().label;

  function begin(kind: "move" | "resize", e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    gesture = { kind, sx: e.screenX, sy: e.screenY };
    invoke("reposition_gesture_start", { label });
  }

  function track(e: PointerEvent) {
    if (!gesture) return;
    const first = pending === null;
    pending = { dx: e.screenX - gesture.sx, dy: e.screenY - gesture.sy };
    if (first) requestAnimationFrame(flush);
  }

  function flush() {
    if (!gesture || !pending) return;
    const { dx, dy } = pending;
    pending = null;
    if (gesture.kind === "move") invoke("reposition_move", { label, dx, dy });
    else invoke("reposition_resize", { label, dx });
  }

  function end(e: PointerEvent) {
    if (!gesture) return;
    track(e);
    flush();
    gesture = null;
  }

  // The box's height follows its content (the sample text re-wraps as it is
  // resized), so measure it and have the host size the window to fit,
  // instead of cropping it to a fixed height.
  let box = $state<HTMLElement>();
  $effect(() => {
    if (!box) return;
    const el = box;
    const report = () => invoke("reposition_box_height", { label, height: Math.ceil(el.getBoundingClientRect().height) + 12 });
    const ro = new ResizeObserver(report);
    ro.observe(el);
    report();
    return () => ro.disconnect();
  });

  /** The newest alert decides the direction: the host sets it per placement. */
  const stackUp = $derived(alerts.length > 0 && alerts.reduce((a, b) => (b.id > a.id ? b : a)).stackUp);

  onMount(() => {
    const unlisten: Array<() => void> = [];
    let disposed = false;
    (async () => {
      const un1 = await listen<Alert>("overlay:alert", (e) => add(e.payload));
      const un2 = await listen<Fold>("overlay:fold", (e) => fold(e.payload));
      const un3 = await listen<RepositionInfo>("overlay:reposition-enter", (e) => (reposition = e.payload));
      const un4 = await listen("overlay:reposition-exit", () => (reposition = null));
      if (disposed) {
        un1();
        un2();
        un3();
        un4();
        return;
      }
      unlisten.push(un1, un2, un3, un4);
      // Tell the host we can receive alerts; it flushes anything it queued.
      await invoke("overlay_ready", { label: getCurrentWebviewWindow().label });
    })();
    return () => {
      disposed = true;
      unlisten.forEach((f) => f());
      timers.forEach((t) => clearTimeout(t));
    };
  });
</script>

{#if reposition}
  <div
    bind:this={box}
    class="stack reposition"
    style="--pilot: {reposition.accent}; --accent: var(--keyword)"
    role="application"
    aria-label="Position this character's alerts"
    onpointerdown={(e) => begin("move", e)}
    onpointermove={track}
    onpointerup={end}
    onpointercancel={end}
  >
    <p class="reposition-hint">
      <span class="reposition-tag">{reposition.tag}</span>
      {reposition.name} · drag to move, drag the right edge to resize · Ctrl+Alt+O when done
    </p>
    <div class="reposition-preview">
      <Panel alert={sample(reposition)} />
    </div>
    <button
      type="button"
      class="reposition-grip"
      aria-label="Resize this character's overlay width"
      onpointerdown={(e) => begin("resize", e)}
      onpointermove={track}
      onpointerup={end}
      onpointercancel={end}
      title="Drag to resize"
    ></button>
  </div>
{:else}
  <div class="stack" class:up={stackUp}>
    {#each alerts as a (a.id)}
      <div style="--accent: {TONE[a.tone]}; --pilot: {a.accent}; display: contents">
        {#if a.style === "beacon"}
          <Beacon alert={a} />
        {:else if a.style === "panel"}
          <Panel alert={a} />
        {:else}
          <Strip alert={a} />
        {/if}
      </div>
    {/each}
  </div>
{/if}
