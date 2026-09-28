<script lang="ts">
  import { type Settings, baseLayerToEdit, peekBaseLayer, resolveBaseField } from "./model";
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

  type Kind = "always" | "ignore";
  type Entry = { name: string; kind: Kind };

  const resolvedAlways = $derived(resolveBaseField(settings, pilotId, "alwaysSenders", [] as string[]));
  const resolvedIgnore = $derived(resolveBaseField(settings, pilotId, "ignoreSenders", [] as string[]));
  const entries = $derived<Entry[]>(
    [
      ...resolvedAlways.value.map((name) => ({ name, kind: "always" as const })),
      ...resolvedIgnore.value.map((name) => ({ name, kind: "ignore" as const })),
    ].sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" })),
  );

  const ownLayer = $derived(peekBaseLayer(settings, pilotId));
  // Either list having its own value counts as "this character has customized
  // sender rules" - always/ignore are one feature to the user, even though
  // they're two independent Layer fields underneath.
  const isOwn = $derived(ownLayer?.alwaysSenders !== undefined || ownLayer?.ignoreSenders !== undefined);
  const showDeviation = $derived(pilotId !== null && isOwn);

  function fieldFor(kind: Kind) {
    return kind === "always" ? ("alwaysSenders" as const) : ("ignoreSenders" as const);
  }

  function startOwnLists() {
    const layer = baseLayerToEdit(settings, pilotId);
    if (layer.alwaysSenders === undefined) layer.alwaysSenders = [...resolvedAlways.value];
    if (layer.ignoreSenders === undefined) layer.ignoreSenders = [...resolvedIgnore.value];
  }

  function save(name: string, kind: Kind, editing: Entry | null) {
    if (!isOwn) startOwnLists();
    const layer = baseLayerToEdit(settings, pilotId);
    if (editing) {
      const oldField = fieldFor(editing.kind);
      layer[oldField] = (layer[oldField] ?? []).filter((n) => n !== editing.name);
    }
    const field = fieldFor(kind);
    const list = (layer[field] ??= []);
    if (!list.includes(name)) list.push(name);
    onedit();
  }

  function removeEntry(entry: Entry) {
    if (!isOwn) return; // nothing of its own to remove from; adding/editing first makes sense
    const layer = baseLayerToEdit(settings, pilotId);
    const field = fieldFor(entry.kind);
    layer[field] = (layer[field] ?? []).filter((n) => n !== entry.name);
    onedit();
  }

  function revertAll() {
    const layer = baseLayerToEdit(settings, pilotId);
    delete layer.alwaysSenders;
    delete layer.ignoreSenders;
    onedit();
  }

  // ---- Add/edit dialog ----
  let dialogOpen = $state(false);
  let editing = $state<Entry | null>(null);
  let dialogValue = $state("");
  let dialogKind = $state<Kind>("always");
  let dialogInputEl = $state<HTMLInputElement>();

  function openAdd() {
    editing = null;
    dialogValue = "";
    dialogKind = "always";
    dialogOpen = true;
  }

  function openEdit(entry: Entry) {
    editing = entry;
    dialogValue = entry.name;
    dialogKind = entry.kind;
    dialogOpen = true;
  }

  $effect(() => {
    if (dialogOpen) queueMicrotask(() => dialogInputEl?.focus());
  });

  function submitDialog() {
    const name = dialogValue.trim();
    if (!name) return;
    save(name, dialogKind, editing);
    dialogOpen = false;
  }

  // ---- Remove confirmation ----
  let removeDialogOpen = $state(false);
  let removeTarget = $state<Entry>({ name: "", kind: "always" });

  function askToRemove(entry: Entry) {
    if (!isOwn) return;
    removeTarget = entry;
    removeDialogOpen = true;
  }
</script>

<h2>Always: Alert / Ignore</h2>
<p class="section-note">
  A personal allow-list and block-list, by exact character name. Both override every other setting above, including a channel muted to
  "Nothing" - that's the entire point of "always."
  {#if pilotId !== null}
    {#if isOwn}This character has its own list. <button type="button" class="revert" onclick={revertAll}>↺ use default</button
      >{:else}Follows Defaults.{/if}
  {/if}
</p>
<section class="card">
  <div class="kind-row">
    <div class="kind-body" style="gap:14px; margin-top:0">
      {#each entries as entry (entry.kind + ":" + entry.name)}
        <span class="chip" class:own={showDeviation}>
          <span class="sender-glyph {entry.kind}">{entry.kind === "always" ? "✓" : "⦸"}</span>
          <button type="button" class="chip-text" onclick={() => openEdit(entry)}>{entry.name}</button>
          <button
            type="button"
            onclick={() => askToRemove(entry)}
            aria-label={`remove ${entry.name}`}
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
  title={editing ? "Edit sender" : "Add a sender"}
  confirmLabel={editing ? "Save" : "Add"}
  confirmDisabled={!dialogValue.trim()}
  onconfirm={submitDialog}
>
  <div class="dialog-field-label">Always</div>
  <div class="segmented" style="margin-bottom:12px">
    <button type="button" class:sel={dialogKind === "always"} onclick={() => (dialogKind = "always")}>Alert</button>
    <button type="button" class:sel={dialogKind === "ignore"} onclick={() => (dialogKind = "ignore")}>Ignore</button>
  </div>
  <input
    bind:this={dialogInputEl}
    class="tag-input dialog-input"
    bind:value={dialogValue}
    placeholder="Character's exact name"
    onkeydown={(e) => e.key === "Enter" && submitDialog()}
  />
</Dialog>

<Dialog bind:open={removeDialogOpen} title="Remove this entry?" confirmLabel="Remove" danger onconfirm={() => removeEntry(removeTarget)}>
  <p class="dialog-msg">Stop {removeTarget.kind === "always" ? "always alerting on" : "ignoring"}
    <span class="dialog-highlight">{removeTarget.name}</span>?</p>
</Dialog>
