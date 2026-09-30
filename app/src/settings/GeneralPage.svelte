<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  /** Mirrors `Locations` in src-tauri/src/lib.rs. */
  type Locations = {
    storage: { portable: boolean; config: string; cache: string; problem: string | null };
    chatLogs: string | null;
    program: string | null;
  };
  type Folder = "chat_logs" | "settings" | "program";

  let autostart = $state(false);
  let locations = $state<Locations | null>(null);
  let error = $state("");
  let folderError = $state("");

  onMount(() => {
    invoke<Locations>("get_locations")
      .then((l) => (locations = l))
      .catch(() => {});
    invoke<boolean>("get_autostart")
      .then((v) => (autostart = v))
      .catch(() => {});
  });

  async function toggleAutostart(enabled: boolean) {
    error = "";
    try {
      await invoke("set_autostart", { enabled });
      autostart = enabled;
    } catch (e) {
      error = `Couldn't change this: ${e}`;
      autostart = !enabled;
    }
  }

  async function open(which: Folder) {
    folderError = "";
    try {
      await invoke("open_folder", { which });
    } catch (e) {
      folderError = String(e);
    }
  }

  const rows = $derived<{ which: Folder; label: string; path: string | null; missing?: string }[]>(
    locations
      ? [
          {
            which: "chat_logs",
            label: "Open chat logs",
            path: locations.chatLogs,
            missing: "Not found yet. Turn on \"Log chat to file\" in EVE's settings and log in.",
          },
          { which: "settings", label: "Open settings folder", path: locations.storage.config },
          { which: "program", label: "Open program folder", path: locations.program },
        ]
      : [],
  );
</script>

<h1>General</h1>
<p class="lede">App-wide behavior, not tied to any one character.</p>
<section class="card">
  <div class="kv" style="grid-template-columns: 1fr; row-gap: 14px">
    <label class="check" style="font-size:13.5px"
      ><input type="checkbox" checked={autostart} onchange={(e) => toggleAutostart(e.currentTarget.checked)} />Start EVE Chatterer when Windows
      starts</label
    >
    <label class="check" style="font-size:13.5px;opacity:.5" title="Not built yet"
      ><input type="checkbox" disabled />Treat input idle for <input class="num" value="5" style="width:34px;margin:0 4px" disabled />minutes as "away"
      (alerts switch to notifications) — coming later</label
    >
  </div>
</section>
{#if error}<p class="section-note error">{error}</p>{/if}

<h2>File locations</h2>
<p class="section-note">
  Where EVE Chatterer reads EVE's chat logs from (it never changes them), and where it keeps its own settings.
</p>
<section class="card locations">
  {#if locations}
    {#each rows as row (row.which)}
      <div class="location-row">
        <button type="button" class="btn" disabled={!row.path} onclick={() => open(row.which)}>{row.label}</button>
        <span class="storage-path" class:missing={!row.path} title={row.path ?? ""}>{row.path ?? row.missing ?? "Unknown"}</span>
      </div>
    {/each}
    <p class="section-note" style="margin:8px 0 0">
      {#if locations.storage.portable}
        This is a portable copy: settings live in the <b>data</b> folder next to the program, so the whole folder can be copied to another PC,
        and deleting it removes everything but Windows' own record that the app may show notifications.
      {:else}
        Settings are in your Windows user's app data, so they survive reinstalling. To make a copy portable instead, put an empty file named
        <b>portable</b> next to the program (the portable download already has one).
      {/if}
    </p>
    {#if locations.storage.problem}<p class="section-note error" style="margin:4px 0 0">{locations.storage.problem}</p>{/if}
  {:else}
    <p class="section-note" style="margin:0">…</p>
  {/if}
</section>
{#if folderError}<p class="section-note error">{folderError}</p>{/if}
