<script lang="ts">
  import { type Settings, baseLayerToEdit, peekBaseLayer, resolveBaseField } from "./model";

  let { settings, pilotId, onedit }: { settings: Settings; pilotId: string | null; onedit: () => void } = $props();

  const resolved = $derived(resolveBaseField(settings, pilotId, "keywords", []));
  const ownList = $derived(peekBaseLayer(settings, pilotId)?.keywords);
  const isOwn = $derived(ownList !== undefined);
  // Defaults isn't inheriting from anything the user should think about — it
  // *is* what everyone else inherits from — so it never shows deviation
  // chrome, even on the (currently impossible, but not worth relying on)
  // chance a future default seeds global.keywords directly.
  const showDeviation = $derived(pilotId !== null && isOwn);

  function startOwnList(initial: string[]) {
    baseLayerToEdit(settings, pilotId).keywords = initial;
    onedit();
  }

  function add() {
    const word = prompt("Keyword or phrase to watch for:")?.trim();
    if (!word) return;
    if (isOwn) {
      if (!ownList!.includes(word)) ownList!.push(word);
    } else {
      startOwnList([...resolved.value, word]);
    }
    onedit();
  }

  function remove(word: string) {
    if (!isOwn) return; // nothing of its own to remove from; adding first makes sense
    const layer = baseLayerToEdit(settings, pilotId);
    layer.keywords = (layer.keywords ?? []).filter((w) => w !== word);
    onedit();
  }

  function revertAll() {
    delete baseLayerToEdit(settings, pilotId).keywords;
    onedit();
  }
</script>

<h2>{#if showDeviation}<span class="pip"></span>{/if}Tracked keywords</h2>
<p class="section-note">
  Checked under every mode above, "Nothing" included — muting a channel for chatter never silences something you asked to hear about.
  {#if pilotId !== null}
    {#if isOwn}This character has its own list instead of Defaults'. <button type="button" class="revert" onclick={revertAll}>↺ Reset to Defaults' list</button
      >{:else}Following Defaults' list.{/if}
  {/if}
</p>
<section class="card">
  <div class="kind-row">
    <div class="kind-body" style="gap:14px">
      {#each resolved.value as word (word)}
        <span class="chip" class:own={showDeviation}>
          {word}
          <button type="button" onclick={() => remove(word)} aria-label={`remove ${word}`} style="background:none;border:none;color:inherit;cursor:pointer;padding:0 0 0 4px">×</button>
        </span>
      {/each}
      <button type="button" class="btn chip-add" onclick={add}>+ Add{pilotId !== null && !isOwn ? " (starts this character's own list)" : ""}</button>
    </div>
  </div>
</section>
