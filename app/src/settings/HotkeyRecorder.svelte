<script lang="ts">
  // Records a key combination for a global hotkey. Adapted from dev-prompt's
  // HotkeyRecorder: combinations Windows reserves are refused, ones other
  // apps commonly use ask for confirmation (hotkeys.ts).
  import { classifyHotkey } from "./hotkeys";

  let {
    value,
    busy = false,
    onsave,
  }: {
    /** The combination in use, for display. */
    value: string;
    busy?: boolean;
    /** Called with a checked combination such as "Ctrl+Alt+O". */
    onsave: (accel: string) => void | Promise<void>;
  } = $props();

  let capturing = $state(false);
  let hint = $state("");
  let pending = $state("");
  let pendingReason = $state("");

  function codeToKey(code: string, fallback: string): string {
    if (code.startsWith("Key")) return code.slice(3); // KeyA -> A
    if (code.startsWith("Digit")) return code.slice(5); // Digit1 -> 1
    return code || fallback; // Space, F5, Comma, ArrowUp, Minus, ...
  }

  function start() {
    hint = "";
    pending = "";
    pendingReason = "";
    capturing = true;
  }

  function apply(accel: string) {
    capturing = false;
    const v = classifyHotkey(accel);
    if (v.level === "block") {
      hint = `${v.reason} Pick another.`;
      return;
    }
    if (v.level === "warn") {
      hint = "";
      pending = accel;
      pendingReason = v.reason ?? "";
      return;
    }
    hint = "";
    void onsave(accel);
  }

  function onKey(e: KeyboardEvent) {
    if (!capturing) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      capturing = false;
      hint = "";
      return;
    }
    if (["Control", "Alt", "Shift", "Meta", "OS"].includes(e.key)) {
      hint = "…keep going";
      return;
    }
    const mods: string[] = [];
    if (e.ctrlKey) mods.push("Ctrl");
    if (e.altKey) mods.push("Alt");
    if (e.shiftKey) mods.push("Shift");
    if (e.metaKey) mods.push("Super");
    apply([...mods, codeToKey(e.code, e.key)].join("+"));
  }

  function confirmPending() {
    const p = pending;
    pending = "";
    pendingReason = "";
    void onsave(p);
  }
</script>

<button type="button" class="hotkey-box" class:capturing disabled={busy} onclick={start} onkeydown={onKey} onblur={() => (capturing = false)}>
  <span class="hotkey-value">{capturing ? "Press a combination…" : value}</span>
  <span class="hotkey-hint">{capturing ? "Esc cancels" : "click to change"}</span>
</button>

{#if pending}
  <div class="hotkey-warn">
    <div><b>{pending}</b>: {pendingReason}</div>
    <div class="hotkey-warn-actions">
      <button type="button" class="btn" disabled={busy} onclick={confirmPending}>Use it anyway</button>
      <button type="button" class="btn" onclick={start}>Pick another</button>
    </div>
  </div>
{/if}
{#if hint}<p class="section-note error" style="margin:6px 0 0">{hint}</p>{/if}
