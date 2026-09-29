<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  let autostart = $state(false);
  let error = $state("");

  onMount(() => {
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
