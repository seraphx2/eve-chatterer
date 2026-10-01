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
  // Saved, but some tracked patterns don't compile and are skipped.
  let saveWarning = $state("");
  let dirty = $state(false); // an edit exists that the last save doesn't reflect yet
  let saving = $state(false); // a save_settings call is in flight right now

  const SAVE_DEBOUNCE_MS = 600;
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let saveAgain = false; // another edit landed while a save was already in flight

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

  // No save button: every edit schedules an autosave a moment later, so a
  // burst of changes (dragging a slider, typing a keyword) collapses into
  // one write instead of one per keystroke.
  function onedit() {
    dirty = true;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(save, SAVE_DEBOUNCE_MS);
  }

  async function save() {
    if (!data) return;
    if (saving) {
      // A save is already in flight for a slightly older snapshot; this
      // edit will be picked up when it finishes, not lost.
      saveAgain = true;
      return;
    }
    saving = true;
    saveError = "";
    try {
      const bad = await invoke<string[]>("save_settings", { settings: data.settings });
      saveWarning = bad.length > 0 ? `Saved. ${bad.length === 1 ? "This pattern doesn't" : "These patterns don't"} work and ${bad.length === 1 ? "is" : "are"} skipped until fixed: ${bad.join(", ")}` : "";
      dirty = false;
    } catch (e) {
      saveError = String(e);
    } finally {
      saving = false;
      if (saveAgain) {
        saveAgain = false;
        save();
      }
    }
  }

  // If the window is about to be hidden (its close button, per lib.rs's
  // hide-not-destroy), send a pending edit immediately rather than leaving
  // it to a debounce timer that a backgrounded page may delay.
  function flushIfDirty() {
    if (dirty && !saving) {
      clearTimeout(saveTimer);
      save();
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
    document.addEventListener("visibilitychange", flushIfDirty);
    return () => {
      clearInterval(t);
      clearTimeout(saveTimer);
      document.removeEventListener("visibilitychange", flushIfDirty);
    };
  });

  const sortedPilots = $derived(
    data
      ? [...data.pilots]
          .filter((p) => p.name.toLowerCase().includes(search.toLowerCase()))
          .sort((a, b) => Number(isOnline(b.id)) - Number(isOnline(a.id)) || a.name.localeCompare(b.name))
      : [],
  );
  const onlineIds = $derived(new Set(data?.online ?? []));
  function isOnline(id: string) {
    return onlineIds.has(id);
  }
  // Narrowed once here (svelte-check doesn't reliably narrow a discriminated
  // union read back out of a template attribute expression).
  const currentPilotId = $derived(page.kind === "pilot" ? page.id : null);
  const currentPilot = $derived(data?.pilots.find((p) => p.id === currentPilotId));
</script>

<div class="app">
  <nav class="sidebar">
    <div class="search"><input placeholder="Search characters…" bind:value={search} /></div>
    <div class="top-items lead">
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
    </div>
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
              <span class="dot {isOnline(p.id) ? 'on' : 'off'}" title={isOnline(p.id) ? "Online now" : "Not running"}></span>
              <span class="label">{p.name}</span>
              {#if p.muted}<span class="sub">muted</span>{:else if !p.live}<span class="sub">logs only</span>{/if}
            </button>
          {/each}
        </div>
      </div>
    </div>
    <div class="top-items">
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
          <ChannelsPage settings={data.settings} pilotId={currentPilotId} pilot={currentPilot} online={currentPilotId !== null && isOnline(currentPilotId)} {onedit} />
        {:else if page.kind === "audio"}
          <AudioPage settings={data.settings} pilots={data.pilots} {onedit} />
        {:else if page.kind === "general"}
          <GeneralPage settings={data.settings} />
        {:else if page.kind === "about"}
          <AboutPage {version} />
        {/if}
      </div>
    </main>
    {#if data}
      <div class="savebar">
        <span class="msg" class:error={!!saveError || (!!saveWarning && !dirty && !saving)}
          >{saveError || (saving ? "Saving…" : dirty ? "Unsaved changes" : saveWarning || "All changes saved")}</span
        >
      </div>
    {/if}
  </div>
</div>
