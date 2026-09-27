<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { CHANNEL_KINDS, CHANNEL_KIND_LABEL, relativeTime, type Pilot, type Settings } from "./model";
  import ChannelRow from "./ChannelRow.svelte";
  import KeywordsSection from "./KeywordsSection.svelte";

  let { settings, pilotId, pilot, onedit }: { settings: Settings; pilotId: string | null; pilot?: Pilot; onedit: () => void } = $props();

  const knownChannels = $derived(
    pilot
      ? Object.entries(pilot.channels).sort(([, a], [, b]) => b.lastSeen - a.lastSeen)
      : [],
  );

  let removeError = $state("");

  async function remove(channelId: string) {
    if (!pilotId || !pilot) return;
    if (!confirm("Remove this channel from the list? It doesn't affect the channel in-game, only this list.")) return;
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
  <p class="lede">
    Applied to every character unless a character overrides it. Change something here and any character who hasn't set their own value
    follows it immediately — including characters you already have.
  </p>
{:else}
  <div class="pilot-head">
    <h1 style="margin:0">{pilot?.name ?? pilotId}</h1>
    {#if pilot}<span class="badge {pilot.live ? 'live' : 'logs'}">{pilot.live ? "Online" : "Seen in logs"}</span>{/if}
  </div>
  <p class="pilot-id">Character ID {pilotId}</p>
  <p class="lede">
    Fields left alone follow Defaults and update automatically if you change them there. A <span class="pip"></span>marker next to a field
    means this character has its own value there, with a link to reset it.
  </p>
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

<KeywordsSection {settings} {pilotId} {onedit} />

{#if pilotId !== null}
  <h2>Public channels</h2>
  <p class="section-note">
    Seen live in this character's logs, newest first. Same fields as any other channel; a new one starts out following Defaults' "Public
    channels" row. A channel with nothing configured can be removed from this list — it doesn't touch the channel itself, only stops it
    cluttering this page.
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
        onremove={() => remove(channelId)}
      />
    {/each}
    {#if knownChannels.length === 0}
      <p class="section-note" style="padding:14px 18px; margin:0">No public channels seen yet — they appear here once this character joins one.</p>
    {/if}
  </section>
{/if}
