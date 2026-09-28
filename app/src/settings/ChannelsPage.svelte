<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { CHANNEL_KINDS, CHANNEL_KIND_LABEL, deriveTag, relativeTime, type Pilot, type Settings } from "./model";
  import ChannelRow from "./ChannelRow.svelte";
  import TrackedListSection from "./TrackedListSection.svelte";
  import Dialog from "./Dialog.svelte";

  let { settings, pilotId, pilot, onedit }: { settings: Settings; pilotId: string | null; pilot?: Pilot; onedit: () => void } = $props();

  const knownChannels = $derived(
    pilot
      ? Object.entries(pilot.channels).sort(([, a], [, b]) => b.lastSeen - a.lastSeen)
      : [],
  );

  let removeError = $state("");
  let tagError = $state("");
  let tagTimer: ReturnType<typeof setTimeout> | undefined;
  let removeDialogOpen = $state(false);
  let removeTargetId = $state("");
  let removeTargetName = $state("");

  // Its own tiny debounced autosave, separate from the rest of the page:
  // this is a Pilot-registry field (pilots.json), not a layered Settings one,
  // so it doesn't belong to the parent's save_settings/onedit flow.
  function setTag(value: string) {
    if (!pilot) return;
    const normalized = value.trim();
    pilot.tag = normalized ? normalized.slice(0, 5).toUpperCase() : undefined;
    clearTimeout(tagTimer);
    const id = pilot.id;
    tagTimer = setTimeout(async () => {
      try {
        await invoke("set_pilot_tag", { pilotId: id, tag: value });
        tagError = "";
      } catch (e) {
        tagError = String(e);
      }
    }, 500);
  }

  function askToRemove(channelId: string, name: string) {
    removeTargetId = channelId;
    removeTargetName = name;
    removeDialogOpen = true;
  }

  async function remove() {
    const channelId = removeTargetId;
    if (!pilotId || !pilot || !channelId) return;
    try {
      await invoke("remove_known_channel", { pilotId, channelId });
      removeError = "";
      // `pilot` is the same reactive object the parent holds — mutating it
      // directly updates the view without a full reload, which would risk
      // discarding any unsaved edits elsewhere on the page.
      delete pilot.channels[channelId];
    } catch (e) {
      removeError = String(e);
    }
  }
</script>

{#if pilotId === null}
  <h1>Defaults</h1>
  <p class="lede">Applied to every character unless it overrides that value.</p>
{:else}
  <div class="pilot-head">
    <h1 style="margin:0">{pilot?.name ?? pilotId}</h1>
    {#if pilot}<span class="badge {pilot.live ? 'live' : 'logs'}">{pilot.live ? "Online" : "Seen in logs"}</span>{/if}
  </div>
  <p class="pilot-id">Character ID {pilotId}</p>
  <p class="lede">
    Untouched fields follow Defaults and stay in sync with it. A <span class="pip"></span>marks a field this character overrides, with a
    link to switch it back.
  </p>
  {#if pilot}
    <div class="field tag-field" style="max-width:280px">
      <span class="flabel">{#if pilot.tag}<span class="pip"></span>{/if}Overlay tag</span>
      <input
        class="tag-input"
        maxlength="5"
        value={pilot.tag ?? ""}
        placeholder={deriveTag(pilot.name)}
        oninput={(e) => setTag(e.currentTarget.value)}
      />
      {#if pilot.tag}<button type="button" class="revert" onclick={() => setTag("")}>↺ use {deriveTag(pilot.name)}</button>{/if}
    </div>
    <p class="section-note">
      Shown in the Strip overlay's badge. Useful once two characters end up with the same initials. Leave it blank to use the initials
      derived from the name: {deriveTag(pilot.name)}.
    </p>
    {#if tagError}<p class="section-note" style="color:var(--danger)">{tagError}</p>{/if}
  {/if}
{/if}

<h2>Channels</h2>
{#if pilotId === null}
  <p class="section-note">Individual public channels are set up on each character's own page, since every character discovers different ones.</p>
{/if}
<section class="card">
  {#each CHANNEL_KINDS as kind (kind)}
    <ChannelRow {settings} {pilotId} {kind} label={CHANNEL_KIND_LABEL[kind]} {onedit} />
  {/each}
</section>

<TrackedListSection {settings} {pilotId} {onedit} />

{#if pilotId !== null}
  <h2>Public channels</h2>
  <p class="section-note">
    Newest first, discovered from this character's chat logs. Each channel initially starts out using the Defaults' Public channels
    settings. If you are no longer using a public channel, you can Remove the configuration to clean things up.
  </p>
  {#if removeError}<p class="section-note" style="color:var(--danger)">{removeError}</p>{/if}
  <section class="card">
    {#each knownChannels as [channelId, info] (channelId)}
      <ChannelRow
        {settings}
        {pilotId}
        kind={info.kind}
        {channelId}
        label={info.name}
        meta={`last active ${relativeTime(info.lastSeen)}`}
        {onedit}
        onremove={() => askToRemove(channelId, info.name)}
      />
    {/each}
    {#if knownChannels.length === 0}
      <p class="section-note" style="padding:14px 18px; margin:0">No public channels seen yet — they appear here once this character joins one.</p>
    {/if}
  </section>
{/if}

<Dialog bind:open={removeDialogOpen} title="Remove this channel?" confirmLabel="Remove" danger onconfirm={remove}>
  <p class="dialog-msg">Remove <span class="dialog-highlight">{removeTargetName}</span> from this list? It doesn't affect the channel in-game.</p>
</Dialog>
