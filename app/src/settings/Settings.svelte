<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getVersion } from "@tauri-apps/api/app";
  import type { SettingsData } from "./model";
  import ChannelsPage from "./ChannelsPage.svelte";
  import AudioPage from "./AudioPage.svelte";
  import GeneralPage from "./GeneralPage.svelte";
  import AboutPage from "./AboutPage.svelte";

  type Page = { kind: "defaults" } | { kind: "pilot"; id: string } | { kind: "audio" } | { kind: "general" } | { kind: "about" };

  let data = $state<SettingsData | null>(null);
  let loadError = $state("");
  let saveError = $state("");
  let dirty = $state(false);
  let saving = $state(false);
  let page = $state<Page>({ kind: "defaults" });
  let pilotsCollapsed = $state(false);
  let version = $state("");
  let search = $state("");

  async function load() {
    try {
      data = await invoke<SettingsData>("get_settings_data");
      loadError = "";
    } catch (e) {
      loadError = String(e);
    }
  }

  function onedit() {
    dirty = true;
  }

  async function save() {
    if (!data) return;
    saving = true;
    saveError = "";
    try {
      await invoke("save_settings", { settings: data.settings });
      dirty = false;
    } catch (e) {
      saveError = String(e);
    } finally {
      saving = false;
    }
  }

  onMount(() => {
    load();
    getVersion()
      .then((v) => (version = v))
      .catch(() => {});
    // Picks up newly detected characters and channels; skipped while there
    // are unsaved edits so it never clobbers something mid-change.
    const t = setInterval(() => {
      if (!dirty) load();
    }, 4000);
    return () => clearInterval(t);
  });

  const sortedPilots = $derived(
    data
      ? [...data.pilots]
          .filter((p) => p.name.toLowerCase().includes(search.toLowerCase()))
          .sort((a, b) => Number(b.live) - Number(a.live) || a.name.localeCompare(b.name))
      : [],
  );
  // Narrowed once here (svelte-check doesn't reliably narrow a discriminated
  // union read back out of a template attribute expression).
  const currentPilotId = $derived(page.kind === "pilot" ? page.id : null);
  const currentPilot = $derived(data?.pilots.find((p) => p.id === currentPilotId));
</script>

<div class="app">
  <nav class="sidebar">
    <div class="search"><input placeholder="Search characters…" bind:value={search} /></div>
    <div class="tree">
      <div class="group" class:collapsed={pilotsCollapsed}>
        <button type="button" class="row" style="width:100%;text-align:left" onclick={() => (pilotsCollapsed = !pilotsCollapsed)}>
          <span class="caret">▾</span>
          <svg class="icon" viewBox="0 0 16 16" fill="none"
            ><path d="M2 13c0-2.8 2.7-5 6-5s6 2.2 6 5" stroke="#7e939f" stroke-width="1.3" /><circle cx="8" cy="5" r="2.6" stroke="#7e939f" stroke-width="1.3" /></svg
          >
          <span class="label">Characters</span>
        </button>
        <div class="children">
          <button type="button" class="row" class:active={page.kind === "defaults"} style="width:100%;text-align:left" onclick={() => (page = { kind: "defaults" })}>
            <span class="caret"></span>
            <svg class="icon" viewBox="0 0 16 16" fill="none"
              ><rect x="2.5" y="2.5" width="11" height="11" stroke="#7e939f" stroke-width="1.3" /><path d="M2.5 6.5h11M6.5 2.5v11" stroke="#7e939f" stroke-width="1.1" /></svg
            >
            <span class="label">Defaults</span>
          </button>
          {#each sortedPilots as p (p.id)}
            <button type="button" class="row" class:active={currentPilotId === p.id} style="width:100%;text-align:left" onclick={() => (page = { kind: "pilot", id: p.id })}>
              <span class="caret"></span>
              <span class="dot {p.live ? 'on' : 'off'}"></span>
              <span class="label">{p.name}</span>
              {#if !p.live}<span class="sub">logs only</span>{/if}
            </button>
          {/each}
        </div>
      </div>
    </div>
    <div class="top-items">
      <button type="button" class="row" class:active={page.kind === "audio"} style="width:100%;text-align:left" onclick={() => (page = { kind: "audio" })}>
        <span class="caret"></span>
        <svg class="icon" viewBox="0 0 16 16" fill="none"
          ><path d="M2 6.5h2.4L8 3.6v8.8L4.4 9.5H2z" stroke="#7e939f" stroke-width="1.2" stroke-linejoin="round" /><path
            d="M10.6 5.8c1 .8 1 3.6 0 4.4M12.4 4.2c1.9 1.7 1.9 6 0 7.7"
            stroke="#7e939f"
            stroke-width="1.2"
            stroke-linecap="round"
          /></svg
        >
        <span class="label">Audio</span>
      </button>
      <button type="button" class="row" class:active={page.kind === "general"} style="width:100%;text-align:left" onclick={() => (page = { kind: "general" })}>
        <span class="caret"></span>
        <svg class="icon" viewBox="0 0 16 16" fill="none"
          ><circle cx="8" cy="8" r="2.3" stroke="#7e939f" stroke-width="1.3" /><path
            d="M8 2v1.6M8 12.4V14M14 8h-1.6M3.6 8H2M12.1 3.9l-1.1 1.1M5 10l-1.1 1.1M12.1 12.1L11 11M5 6l-1.1-1.1"
            stroke="#7e939f"
            stroke-width="1.2"
            stroke-linecap="round"
          /></svg
        >
        <span class="label">General</span>
      </button>
      <button type="button" class="row" class:active={page.kind === "about"} style="width:100%;text-align:left" onclick={() => (page = { kind: "about" })}>
        <span class="caret"></span>
        <svg class="icon" viewBox="0 0 16 16" fill="none"
          ><circle cx="8" cy="8" r="5.6" stroke="#7e939f" stroke-width="1.3" /><path d="M8 7.2v4M8 5.3v.05" stroke="#7e939f" stroke-width="1.3" stroke-linecap="round" /></svg
        >
        <span class="label">About</span>
      </button>
    </div>
    <footer><span>Watching {data ? data.pilots.length : 0} characters</span></footer>
  </nav>

  <div style="display:flex; flex-direction:column; min-width:0; min-height:0">
    <main class="content">
      <div class="page">
        {#if loadError}
          <p>{loadError}</p>
        {:else if !data}
          <p class="lede">Loading…</p>
        {:else if page.kind === "defaults"}
          <ChannelsPage settings={data.settings} pilotId={null} {onedit} />
        {:else if page.kind === "pilot"}
          <ChannelsPage settings={data.settings} pilotId={currentPilotId} pilot={currentPilot} {onedit} />
        {:else if page.kind === "audio"}
          <AudioPage />
        {:else if page.kind === "general"}
          <GeneralPage />
        {:else if page.kind === "about"}
          <AboutPage {version} />
        {/if}
      </div>
    </main>
    {#if data}
      <div class="savebar">
        <span class="msg" class:error={!!saveError}>{saveError || (dirty ? "Unsaved changes" : "All changes saved")}</span>
        <button type="button" class="btn primary" disabled={!dirty || saving} onclick={save}>{saving ? "Saving…" : "Save"}</button>
      </div>
    {/if}
  </div>
</div>
