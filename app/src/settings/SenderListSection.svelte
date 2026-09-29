<script lang="ts">
  import { type Settings, baseLayerToEdit, peekBaseLayer } from "./model";
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

  // The lists merge by name (core/src/settings.rs `resolve`): Defaults decide
  // every name, and a character only overrides the names it lists itself,
  // including "normal" to undo one of Defaults' entries. So a name added to
  // Defaults later still reaches every character.
  type Verdict = "always" | "ignore" | "normal";
  type Entry = { name: string; inherited?: Verdict; own?: Verdict };

  const FIELD = { always: "alwaysSenders", ignore: "ignoreSenders", normal: "normalSenders" } as const;
  const same = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: "base" }) === 0;

  const isPilot = $derived(pilotId !== null);

  /** Name -> verdict for one layer; within a layer, ignore beats always beats normal (as in Rust). */
  function verdicts(layer: { alwaysSenders?: string[]; ignoreSenders?: string[]; normalSenders?: string[] } | undefined) {
    const out = new Map<string, { name: string; v: Verdict }>();
    for (const v of ["normal", "always", "ignore"] as const) {
      for (const name of layer?.[FIELD[v]] ?? []) out.set(name.toLowerCase(), { name, v });
    }
    return out;
  }

  const entries = $derived.by<Entry[]>(() => {
    const defaults = verdicts(settings.global);
    const own = isPilot ? verdicts(peekBaseLayer(settings, pilotId)) : new Map();
    const all = new Map<string, Entry>();
    if (!isPilot) {
      // Defaults' own page: its entries are its own.
      for (const [k, { name, v }] of defaults) all.set(k, { name, own: v });
    } else {
      for (const [k, { name, v }] of defaults) all.set(k, { name, inherited: v });
      for (const [k, { name, v }] of own) all.set(k, { ...(all.get(k) ?? { name }), own: v });
    }
    return [...all.values()].sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
  });

  const effective = (e: Entry): Verdict => e.own ?? e.inherited ?? "normal";
  const hasOverrides = $derived(isPilot && entries.some((e) => e.own !== undefined));

  /** Sets this page's verdict for `name`, or clears it (`undefined`) so the inherited one applies again. */
  function setOwn(name: string, v: Verdict | undefined) {
    const layer = baseLayerToEdit(settings, pilotId);
    for (const f of Object.values(FIELD)) {
      const kept = (layer[f] ?? []).filter((n) => !same(n, name));
      if (kept.length) layer[f] = kept;
      else delete layer[f];
    }
    if (v !== undefined) (layer[FIELD[v]] ??= []).push(name);
    onedit();
  }

  function save(name: string, v: Verdict, editing: Entry | null) {
    if (editing && !same(editing.name, name)) setOwn(editing.name, undefined);
    const inherited = entries.find((e) => same(e.name, name))?.inherited;
    // Choosing what Defaults already says is just following Defaults.
    if (isPilot && (v === inherited || (v === "normal" && inherited === undefined))) setOwn(name, undefined);
    else setOwn(name, v);
  }

  function revertAll() {
    const layer = baseLayerToEdit(settings, pilotId);
    for (const f of Object.values(FIELD)) delete layer[f];
    onedit();
  }

  function chipTitle(e: Entry): string {
    const word = { always: "Always alert", ignore: "Ignored", normal: "Normal (neither)" };
    if (!isPilot) return word[effective(e)];
    if (e.own === undefined) return `${word[e.inherited!]}, from Defaults`;
    if (e.inherited === undefined) return `${word[e.own]}, for this character only`;
    return `${word[e.own]} for this character (Defaults: ${word[e.inherited].toLowerCase()})`;
  }

  // ---- Add/edit dialog ----
  let dialogOpen = $state(false);
  let editing = $state<Entry | null>(null);
  let dialogValue = $state("");
  let dialogVerdict = $state<Verdict>("always");
  let dialogInputEl = $state<HTMLInputElement>();

  function openAdd() {
    editing = null;
    dialogValue = "";
    dialogVerdict = "always";
    dialogOpen = true;
  }

  function openEdit(entry: Entry) {
    editing = entry;
    dialogValue = entry.name;
    dialogVerdict = effective(entry);
    dialogOpen = true;
  }

  $effect(() => {
    if (dialogOpen) queueMicrotask(() => dialogInputEl?.focus());
  });

  function submitDialog() {
    const name = dialogValue.trim();
    if (!name) return;
    save(name, dialogVerdict, editing);
    dialogOpen = false;
  }

  // ---- Remove confirmation: on Defaults it removes the entry; on a
  // character it removes that character's own setting for the name. ----
  let removeDialogOpen = $state(false);
  let removeTarget = $state<Entry>({ name: "" });

  function askToRemove(entry: Entry) {
    removeTarget = entry;
    removeDialogOpen = true;
  }
</script>

<h2>Always: Alert / Ignore</h2>
<p class="section-note">
  A personal allow-list and block-list, by exact character name. Both override every other setting above, including a channel muted to
  "Nothing" - that's the entire point of "always."
  {#if isPilot}
    Defaults' entries apply here too; set a name here to decide it differently for this character, including "Normal" to undo one of
    Defaults' entries.
    {#if hasOverrides}<button type="button" class="revert" onclick={revertAll}>↺ use default for all</button>{/if}
  {/if}
</p>
<section class="card">
  <div class="kind-row">
    <div class="kind-body" style="gap:14px; margin-top:0">
      {#each entries as entry (entry.name.toLowerCase())}
        {@const v = effective(entry)}
        <span class="chip" class:own={isPilot && entry.own !== undefined} class:normal-chip={v === "normal"} title={chipTitle(entry)}>
          <span class="sender-glyph {v}">{v === "always" ? "✓" : v === "ignore" ? "⦸" : "○"}</span>
          <button type="button" class="chip-text" onclick={() => openEdit(entry)}>{entry.name}</button>
          {#if isPilot && entry.own === undefined}
            <span class="chip-scope">· Defaults</span>
          {:else}
            <button
              type="button"
              onclick={() => askToRemove(entry)}
              aria-label={isPilot && entry.inherited !== undefined ? `use Defaults for ${entry.name}` : `remove ${entry.name}`}
              title={isPilot && entry.inherited !== undefined ? "Use Defaults' setting for this name" : "Remove"}
              style="background:none;border:none;color:var(--danger);cursor:pointer;padding:0 0 0 4px"
            >
              {isPilot && entry.inherited !== undefined ? "↺" : "×"}
            </button>
          {/if}
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
    <button type="button" class:sel={dialogVerdict === "always"} onclick={() => (dialogVerdict = "always")}>Alert</button>
    <button type="button" class:sel={dialogVerdict === "ignore"} onclick={() => (dialogVerdict = "ignore")}>Ignore</button>
    {#if isPilot}<button type="button" class:sel={dialogVerdict === "normal"} onclick={() => (dialogVerdict = "normal")} title="Neither: treat them like anyone else, whatever Defaults says"
        >Normal</button
      >{/if}
  </div>
  <input
    bind:this={dialogInputEl}
    class="tag-input dialog-input"
    bind:value={dialogValue}
    placeholder="Character's exact name"
    onkeydown={(e) => e.key === "Enter" && submitDialog()}
  />
</Dialog>

<Dialog
  bind:open={removeDialogOpen}
  title={isPilot && removeTarget.inherited !== undefined ? "Use Defaults for this name?" : "Remove this entry?"}
  confirmLabel={isPilot && removeTarget.inherited !== undefined ? "Use Defaults" : "Remove"}
  danger={!(isPilot && removeTarget.inherited !== undefined)}
  onconfirm={() => setOwn(removeTarget.name, undefined)}
>
  <p class="dialog-msg">
    {#if isPilot && removeTarget.inherited !== undefined}
      Go back to Defaults' setting for <span class="dialog-highlight">{removeTarget.name}</span> ({removeTarget.inherited === "always"
        ? "always alert"
        : "ignored"})?
    {:else}
      Stop {effective(removeTarget) === "always" ? "always alerting on" : "ignoring"} <span class="dialog-highlight">{removeTarget.name}</span>?
    {/if}
  </p>
</Dialog>
