<script lang="ts">
  import { CHANNEL_KINDS, CHANNEL_KIND_LABEL, type ChannelKind, type Settings, type TrackedKind, type TrackedRule, baseLayerToEdit, peekBaseLayer } from "./model";
  import Dialog from "./Dialog.svelte";

  let {
    settings,
    pilotId,
    onedit,
  }: {
    settings: Settings;
    pilotId: string | null;
    onedit: () => void;
  } = $props();

  // A character's entries add to Defaults' (core/src/settings.rs `resolve`),
  // so each page lists and edits only its own layer's entries: Defaults'
  // apply everywhere without being copied in, and there's nothing to
  // override or revert.
  const entries = $derived(peekBaseLayer(settings, pilotId)?.tracked ?? []);

  function scopeLabel(entry: TrackedRule): string {
    return entry.onlyIn && entry.onlyIn.length > 0 ? entry.onlyIn.map((k) => CHANNEL_KIND_LABEL[k]).join(", ") : "";
  }

  function chipTitle(entry: TrackedRule): string {
    const scope = entry.onlyIn && entry.onlyIn.length > 0 ? `Only in: ${scopeLabel(entry)}` : "Applies to every channel";
    const muted = entry.evenWhenMuted ? "Still fires on a muted (“Nothing”) channel" : "Silent on a muted (“Nothing”) channel";
    return `${scope}\n${muted}`;
  }

  function save(rule: TrackedRule, editIndex: number | null) {
    const layer = baseLayerToEdit(settings, pilotId);
    const list = (layer.tracked ??= []);
    if (editIndex !== null) list[editIndex] = rule;
    else list.push(rule);
    onedit();
  }

  function removeAt(index: number) {
    const layer = baseLayerToEdit(settings, pilotId);
    layer.tracked = (layer.tracked ?? []).filter((_, i) => i !== index);
    onedit();
  }

  // ---- Add/edit dialog: a String/Regex toggle, an input decorated with
  // slashes in Regex mode (RegExr-style chrome, never part of the stored
  // text), a channel-scope picker, and the muted-channel exemption. The same
  // dialog handles both adding a new entry and editing an existing chip. ----
  let dialogOpen = $state(false);
  let editIndex = $state<number | null>(null);
  let dialogValue = $state("");
  let dialogKind = $state<TrackedKind>("keyword");
  let dialogScope = $state<Set<ChannelKind>>(new Set());
  let dialogEvenWhenMuted = $state(true);
  let dialogInputEl = $state<HTMLInputElement>();

  function openAdd() {
    editIndex = null;
    dialogValue = "";
    dialogKind = "keyword";
    dialogScope = new Set();
    dialogEvenWhenMuted = false;
    dialogOpen = true;
  }

  function openEdit(index: number, entry: TrackedRule) {
    editIndex = index;
    dialogValue = entry.text;
    dialogKind = entry.kind;
    dialogScope = new Set(entry.onlyIn ?? []);
    dialogEvenWhenMuted = entry.evenWhenMuted;
    dialogOpen = true;
  }

  $effect(() => {
    if (dialogOpen) queueMicrotask(() => dialogInputEl?.focus());
  });

  function toggleScopeKind(k: ChannelKind) {
    const next = new Set(dialogScope);
    if (next.has(k)) next.delete(k);
    else next.add(k);
    dialogScope = next;
  }

  function submitDialog() {
    const text = dialogValue.trim();
    if (!text) return;
    save({ text, kind: dialogKind, onlyIn: [...dialogScope], evenWhenMuted: dialogEvenWhenMuted }, editIndex);
    dialogOpen = false;
  }

  // ---- Remove confirmation ----
  let removeDialogOpen = $state(false);
  let removeIndex = $state(0);
  let removeTargetText = $state("");

  function askToRemove(index: number, entry: TrackedRule) {
    removeIndex = index;
    removeTargetText = entry.text;
    removeDialogOpen = true;
  }
</script>

<h2>Tracked</h2>
<p class="section-note">
  Checked under every mode above, including "Nothing" by default — a muted channel can still show something you're watching for, unless
  that entry turns it off below. Applies everywhere unless scoped to specific channels.
  {#if pilotId !== null}
    Everything tracked in Defaults applies here too; entries added here are on top of those, for this character only.
  {/if}
</p>
<section class="card">
  <div class="kind-row">
    <div class="kind-body" style="gap:14px; margin-top:0">
      {#each entries as entry, i (i)}
        <span class="chip" class:regex-chip={entry.kind === "regex"} title={chipTitle(entry)}>
          <button type="button" class="chip-text" onclick={() => openEdit(i, entry)}>
            {#if entry.kind === "regex"}<span class="regex-delim">/</span>{/if}{entry.text}{#if entry.kind === "regex"}<span
                class="regex-delim">/</span
              >{/if}
          </button>
          {#if scopeLabel(entry)}<span class="chip-scope">· {scopeLabel(entry)}</span>{/if}
          {#if !entry.evenWhenMuted}<span class="chip-scope">· ignores Nothing</span>{/if}
          <button
            type="button"
            onclick={() => askToRemove(i, entry)}
            aria-label={`remove ${entry.text}`}
            style="background:none;border:none;color:var(--danger);cursor:pointer;padding:0 0 0 4px"
          >
            ×
          </button>
        </span>
      {/each}
      <button type="button" class="btn chip-add" onclick={openAdd}>+ Add</button>
    </div>
  </div>
</section>

<Dialog
  bind:open={dialogOpen}
  title={editIndex !== null ? "Edit tracked entry" : "Add to Tracked"}
  confirmLabel={editIndex !== null ? "Save" : "Add"}
  confirmDisabled={!dialogValue.trim()}
  onconfirm={submitDialog}
>
  <div class="segmented" style="margin-bottom:12px">
    <button type="button" class:sel={dialogKind === "keyword"} onclick={() => (dialogKind = "keyword")}>String</button>
    <button type="button" class:sel={dialogKind === "regex"} onclick={() => (dialogKind = "regex")}>Regex</button>
  </div>
  <div class="regex-input-wrap" class:regex={dialogKind === "regex"}>
    {#if dialogKind === "regex"}<span class="regex-delim">/</span>{/if}
    <input
      bind:this={dialogInputEl}
      bind:value={dialogValue}
      placeholder={dialogKind === "regex" ? String.raw`\bgank(ed|ing)?\b` : "Keyword or phrase to watch for"}
      onkeydown={(e) => e.key === "Enter" && submitDialog()}
    />
    {#if dialogKind === "regex"}<span class="regex-delim">/</span><span
        class="regex-flag"
        title="Patterns are always matched case-insensitively">i</span
      >{/if}
  </div>

  <div class="dialog-field-label">Applies to</div>
  <div class="scope-picker">
    <button type="button" class="scope-chip" class:sel={dialogScope.size === 0} onclick={() => (dialogScope = new Set())}>All channels</button>
    {#each CHANNEL_KINDS as k (k)}
      <button type="button" class="scope-chip" class:sel={dialogScope.has(k)} onclick={() => toggleScopeKind(k)}>{CHANNEL_KIND_LABEL[k]}</button>
    {/each}
  </div>

  <label class="check" style="margin-top:14px">
    <input type="checkbox" bind:checked={dialogEvenWhenMuted} />
    Still alert even when the channel is muted ("Nothing")
  </label>
</Dialog>

<Dialog bind:open={removeDialogOpen} title="Remove this entry?" confirmLabel="Remove" danger onconfirm={() => removeAt(removeIndex)}>
  <p class="dialog-msg">Stop tracking <span class="dialog-highlight">{removeTargetText}</span>?</p>
</Dialog>
