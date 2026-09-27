<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  interface Pilot {
    id: string;
    name: string;
    live: boolean;
  }
  interface Status {
    logFolder: string | null;
    running: boolean;
    pilots: Pilot[];
    alertsShown: number;
    overlayWindows: number;
  }

  let status = $state<Status | null>(null);
  let error = $state("");

  async function refresh() {
    try {
      status = await invoke<Status>("get_status");
      error = "";
    } catch (e) {
      error = String(e);
    }
  }

  async function test(kind: string) {
    try {
      await invoke("send_test", { kind });
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    refresh();
    const t = setInterval(refresh, 2000);
    return () => clearInterval(t);
  });
</script>

<main>
  <h1>EVE Chatterer</h1>
  <p class="dim">Running in the tray. Alerts appear over your EVE clients.</p>

  <h2>Status</h2>
  <section>
    {#if status}
      <p>
        {#if status.running}Watching{:else}Not watching{/if}
        {#if status.logFolder}<span class="dim"> {status.logFolder}</span>{/if}
      </p>
      <p class="dim">
        {status.alertsShown} alerts shown this session. Overlay windows open: {status.overlayWindows}
        (they close on their own after a while without alerts).
      </p>
    {:else}
      <p class="dim">Loading…</p>
    {/if}
    {#if error}<p>{error}</p>{/if}
  </section>

  <h2>Characters</h2>
  <section>
    {#if status && status.pilots.length}
      <table>
        <thead><tr><th>Name</th><th>Character ID</th><th>State</th></tr></thead>
        <tbody>
          {#each status.pilots as p (p.id)}
            <tr>
              <td>{p.name}</td>
              <td class="dim">{p.id}</td>
              <td class={p.live ? "live" : "dim"}>{p.live ? "Playing" : "In old logs"}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="dim">No characters found yet. Log in with chat logging turned on.</p>
    {/if}
  </section>

  <h2>Try an overlay</h2>
  <section>
    <div class="row">
      <button type="button" onclick={() => test("panel")}>Panel</button>
      <button type="button" onclick={() => test("strip")}>Strip</button>
      <button type="button" onclick={() => test("beacon")}>Beacon</button>
      <button type="button" onclick={() => test("burst")}>Burst of 12</button>
    </div>
    <p class="dim" style="margin-top: 10px">Shows on every monitor, without taking focus from your game.</p>
  </section>
</main>
