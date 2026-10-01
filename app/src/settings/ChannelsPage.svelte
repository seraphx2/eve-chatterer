<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { CHANNEL_KINDS, CHANNEL_KIND_LABEL, affiliation, deriveTag, relativeTime, type Pilot, type Settings } from "./model";
  import ChannelRow from "./ChannelRow.svelte";
  import TrackedListSection from "./TrackedListSection.svelte";
  import SenderListSection from "./SenderListSection.svelte";
  import Dialog from "./Dialog.svelte";

  let {
    settings,
    pilotId,
    pilot,
    online = false,
    onedit,
  }: { settings: Settings; pilotId: string | null; pilot?: Pilot; online?: boolean; onedit: () => void } = $props();

  // From the Corp / Alliance logs; unknown until the character has logged in
  // with chat logging on.
  const org = $derived(pilot ? affiliation(pilot) : {});
  const orgOf = (kind: string) => (kind === "corp" ? org.corp : kind === "alliance" ? org.alliance : undefined);

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
  let placementError = $state("");

  async function resetPlacement() {
    if (!pilot) return;
    try {
      await invoke("clear_pilot_placement", { pilotId: pilot.id });
      placementError = "";
      pilot.placement = undefined;
    } catch (e) {
      placementError = String(e);
    }
  }

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

  let muteBusy = $state(false);
  let muteError = $state("");

  // A pilots.json field, like the tag: saved at once, not through onedit.
  async function setMuted(muted: boolean) {
    if (!pilot) return;
    muteBusy = true;
    try {
      await invoke("set_pilot_muted", { pilotId: pilot.id, muted });
      pilot.muted = muted;
      muteError = "";
    } catch (e) {
      muteError = String(e);
    } finally {
      muteBusy = false;
    }
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
    <!-- Online = a client is running now. `live` only means "has ever played",
         which is what tells Offline apart from a character only ever seen in old logs. -->
    {#if pilot}<span class="badge {online ? 'live' : 'logs'}">{online ? "Online" : pilot.live ? "Offline" : "Seen in logs"}</span>{/if}
  </div>
  {#if org.corp || org.alliance}
    <p class="pilot-org">{[org.corp, org.alliance].filter(Boolean).join(" · ")}</p>
  {/if}
  <p class="pilot-id">Character ID {pilotId}</p>
  {#if pilot}
    <label class="check mute-toggle">
      <input type="checkbox" checked={!!pilot.muted} disabled={muteBusy} onchange={(e) => setMuted(e.currentTarget.checked)} />
      <span><b class="mute-word">Mute</b> no alerts for this character, on any client</span>
    </label>
    {#if pilot.muted}
      <p class="section-note warning">Nothing from {pilot.name}'s chats will alert you, despite what is set below. The settings will be restored when it's unmuted.</p>
    {/if}
    {#if muteError}<p class="section-note error">{muteError}</p>{/if}
  {/if}
{/if}

<!-- Muted: everything below is kept but can't be changed. -->
<fieldset class="mute-scope" disabled={!!pilot?.muted}>
{#if pilotId !== null}
  <p class="lede">
    Untouched fields follow <b class="defaults-ref">Defaults</b> and stay in sync with it. A <span class="pip"></span>marks a field this character overrides, with a
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
    <p class="section-note">
      Overlay position: {#if pilot.placement}custom ({Math.round(pilot.placement.width)}px wide) <button
          type="button"
          class="revert"
          onclick={resetPlacement}>↺ reset to default</button
        >{:else}
        default (centered on whichever screen the alert should draw attention to){/if}. Press <b>{settings.general.repositionHotkey}</b> in game to drag and resize
      it.
    </p>
    {#if placementError}<p class="section-note" style="color:var(--danger)">{placementError}</p>{/if}
  {/if}
{/if}

<h2>Channels</h2>
{#if pilotId === null}
  <p class="section-note">Individual public channels are set up on each character's own page, since every character discovers different ones.</p>
{/if}
<section class="card">
  {#each CHANNEL_KINDS as kind (kind)}
    <ChannelRow {settings} {pilotId} {kind} label={CHANNEL_KIND_LABEL[kind]} meta={orgOf(kind)} {onedit} />
  {/each}
</section>

<TrackedListSection {settings} {pilotId} {onedit} />

<SenderListSection {settings} {pilotId} {onedit} />

{#if pilotId !== null}
  <h2>Public channels</h2>
  <p class="section-note">
    Newest first, discovered from this character's chat logs. Each channel initially starts out using the <b class="defaults-ref">Defaults</b>' Public channels
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

</fieldset>

<Dialog bind:open={removeDialogOpen} title="Remove this channel?" confirmLabel="Remove" danger onconfirm={remove}>
  <p class="dialog-msg">Remove <span class="dialog-highlight">{removeTargetName}</span> from this list? It doesn't affect the channel in-game.</p>
</Dialog>
