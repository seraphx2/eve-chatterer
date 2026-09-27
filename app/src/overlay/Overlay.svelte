<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
  import Beacon from "./Beacon.svelte";
  import Panel from "./Panel.svelte";
  import Strip from "./Strip.svelte";
  import type { Alert, Fold, Style, Tone } from "./types";

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
    const target = [...alerts].filter((a) => a.pilot === f.pilot && a.channel === f.channel).sort((x, y) => y.id - x.id)[0];
    if (target) target.count += 1;
  }

  onMount(() => {
    const unlisten: Array<() => void> = [];
    let disposed = false;
    (async () => {
      const un1 = await listen<Alert>("overlay:alert", (e) => add(e.payload));
      const un2 = await listen<Fold>("overlay:fold", (e) => fold(e.payload));
      if (disposed) {
        un1();
        un2();
        return;
      }
      unlisten.push(un1, un2);
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

<div class="stack">
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
