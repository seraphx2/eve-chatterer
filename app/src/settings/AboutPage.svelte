<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  let { version }: { version: string } = $props();

  /** Mirrors `UpdateStatus` in src-tauri/src/updates.rs. */
  type UpdateStatus = {
    mode: "self" | "unmanaged";
    available: string | null;
    notes: string | null;
    checking: boolean;
    installing: boolean;
    lastChecked: number | null;
    error: string | null;
  };

  let status = $state<UpdateStatus | null>(null);
  let busy = $state(false);

  onMount(() => {
    const refresh = () =>
      invoke<UpdateStatus>("get_update_status")
        .then((s) => (status = s))
        .catch(() => {});
    refresh();
    // The tray app checks on its own timer; keep this page in step with it.
    const t = setInterval(refresh, 5000);
    return () => clearInterval(t);
  });

  async function check() {
    busy = true;
    try {
      status = await invoke<UpdateStatus>("check_for_updates");
    } finally {
      busy = false;
    }
  }

  async function install() {
    busy = true;
    try {
      // On success the app closes and restarts into the new version.
      await invoke("install_update");
    } catch {
      status = await invoke<UpdateStatus>("get_update_status");
    } finally {
      busy = false;
    }
  }

  function when(secs: number): string {
    return new Date(secs * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
  }

  const message = $derived.by(() => {
    if (!status) return "…";
    if (status.mode !== "self") return "This copy doesn't update itself (the portable version, or a development build). New versions are on GitHub.";
    if (status.installing) return `Installing ${status.available}… EVE Chatterer will restart.`;
    if (status.checking || busy) return "Checking…";
    if (status.error) return `Couldn't check for updates: ${status.error}`;
    if (status.available) return `Version ${status.available} is available.`;
    if (status.lastChecked) return `Up to date. Last checked ${when(status.lastChecked)}.`;
    return "Checks for updates automatically.";
  });
</script>

<h1>About</h1>
<section class="card">
  <div class="about-top" style="padding:18px 18px 0">
    <div class="about-icon"><span class="notch"></span></div>
    <div>
      <div style="font-size:17px;font-weight:500">EVE Chatterer</div>
      <div class="pilot-id">Version {version || "…"}</div>
    </div>
  </div>
  <div class="kv">
    <dt>License</dt>
    <dd>MIT</dd>
    <dt>Source</dt>
    <dd>github.com/seraphx2/eve-chatterer</dd>
  </div>
  <div class="update-row">
    <span class="update-status" class:error={!!status?.error}>{message}</span>
    {#if status?.mode === "self"}
      {#if status.available}
        <button type="button" class="btn primary" disabled={busy || status.installing} onclick={install}>Install and restart</button>
      {:else}
        <button type="button" class="btn" disabled={busy || status.checking} onclick={check}>Check for updates</button>
      {/if}
    {/if}
  </div>
  {#if status?.available && status.notes}
    <div class="update-notes">{status.notes}</div>
  {/if}
</section>
