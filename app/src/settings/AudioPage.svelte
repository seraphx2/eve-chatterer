<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { AudioMode, Pilot, Settings } from "./model";

  let { settings, pilots, onedit }: { settings: Settings; pilots: Pilot[]; onedit: () => void } = $props();

  const audio = $derived(settings.audio);
  const sortedPilots = $derived([...pilots].sort((a, b) => a.name.localeCompare(b.name)));
  let error = $state("");

  const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

  function setMode(mode: AudioMode) {
    settings.audio.mode = mode;
    onedit();
  }

  /** Browse for a file: the shared sound (`pilotId` null) or one character's. */
  async function browse(pilotId: string | null) {
    error = "";
    try {
      const picked = await invoke<string | null>("pick_sound_file");
      if (!picked) return;
      if (pilotId === null) settings.audio.sharedFile = picked;
      else (settings.audio.pilotFiles ??= {})[pilotId] = picked;
      onedit();
    } catch (e) {
      error = String(e);
    }
  }

  function clear(pilotId: string | null) {
    if (pilotId === null) delete settings.audio.sharedFile;
    else if (settings.audio.pilotFiles) {
      delete settings.audio.pilotFiles[pilotId];
      if (Object.keys(settings.audio.pilotFiles).length === 0) delete settings.audio.pilotFiles;
    }
    onedit();
  }

  function play(file: string | undefined) {
    invoke("preview_sound", { file: file ?? null, volume: audio.volume }).catch((e) => (error = String(e)));
  }

  function setVolume(v: number) {
    settings.audio.volume = Math.max(0, Math.min(100, Math.round(v)));
    onedit();
  }

  function setCooldown(v: number) {
    settings.audio.cooldownSecs = Math.max(0, Math.min(600, Math.round(v) || 0));
    onedit();
  }
</script>

<h1>Audio</h1>
<p class="lede">
  Which sound plays, and how often. Whether a channel makes a sound at all is that channel's own Sound setting on each character's page.
</p>

<section class="card">
  <label class="audio-choice">
    <input type="radio" name="audio-mode" checked={audio.mode === "off"} onchange={() => setMode("off")} />
    <div class="ac-body"><div class="ac-title">No sound</div><div class="ac-sub">Alerts stay silent everywhere, Windows notifications included.</div></div>
  </label>
  <label class="audio-choice">
    <input type="radio" name="audio-mode" checked={audio.mode === "shared"} onchange={() => setMode("shared")} />
    <div class="ac-body"><div class="ac-title">One sound for every character</div></div>
  </label>
  <label class="audio-choice">
    <input type="radio" name="audio-mode" checked={audio.mode === "per_character"} onchange={() => setMode("per_character")} />
    <div class="ac-body">
      <div class="ac-title">A different sound per character</div>
      <div class="ac-sub">A character with no sound of its own uses the shared one.</div>
    </div>
  </label>
</section>

<div class:muted={audio.mode === "off"}>
  <h2>Playback</h2>
  <section class="card sound-list">
    <div class="sound-row">
      <span class="sound-who">Volume</span>
      <input class="volume" type="range" min="0" max="100" step="1" value={audio.volume} oninput={(e) => setVolume(Number(e.currentTarget.value))} />
      <span class="sound-value">{audio.volume}%</span>
    </div>
    <div class="sound-row">
      <span class="sound-who">Quiet time</span>
      <span class="sound-file">
        <input class="num" type="number" min="0" max="600" value={audio.cooldownSecs} oninput={(e) => setCooldown(Number(e.currentTarget.value))} /> seconds after a
        sound before the next one can play
      </span>
    </div>
    <p class="section-note" style="margin:8px 0">
      Only one sound plays at a time. A mention always plays: it ignores the quiet time and cuts off any sound already playing. Chosen files
      play for at most 15 seconds.
    </p>
  </section>

  <h2>{audio.mode === "per_character" ? "Shared sound" : "Sound"}</h2>
  <section class="card sound-list">
    <div class="sound-row">
      <span class="sound-who">{audio.mode === "per_character" ? "Shared" : "Every character"}</span>
      <span class="sound-file" title={audio.sharedFile ?? ""}>{audio.sharedFile ? fileName(audio.sharedFile) : "Built-in alert tone"}</span>
      <button type="button" class="btn play" onclick={() => play(audio.sharedFile)} aria-label="Play the shared sound" title="Play">▶</button>
      <button type="button" class="btn" onclick={() => browse(null)}>Browse…</button>
      {#if audio.sharedFile}<button type="button" class="revert" onclick={() => clear(null)}>↺ use built-in</button>{/if}
    </div>
  </section>

  {#if audio.mode === "per_character"}
    <h2>Per character</h2>
    <section class="card sound-list">
      {#each sortedPilots as p (p.id)}
        {@const own = audio.pilotFiles?.[p.id]}
        <div class="sound-row">
          <span class="sound-who">{p.name}</span>
          <span class="sound-file" class:inherited={!own} title={own ?? ""}>{own ? fileName(own) : "Uses the shared sound"}</span>
          <button type="button" class="btn play" onclick={() => play(own ?? audio.sharedFile)} aria-label="Play {p.name}'s sound" title="Play">▶</button>
          <button type="button" class="btn" onclick={() => browse(p.id)}>Browse…</button>
          {#if own}<button type="button" class="revert" onclick={() => clear(p.id)}>↺ use shared</button>{/if}
        </div>
      {:else}
        <p class="section-note" style="margin:0">No characters yet.</p>
      {/each}
    </section>
  {/if}

</div>

{#if error}<p class="section-note error">{error}</p>{/if}
