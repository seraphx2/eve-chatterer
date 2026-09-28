<script lang="ts">
  import type { Snippet } from "svelte";

  // A themed replacement for the browser's native prompt()/confirm() popups,
  // which look and feel nothing like the rest of the app. Two modes: "prompt"
  // asks for one line of text, "confirm" just asks yes/no.
  let {
    open = $bindable(false),
    title,
    message,
    children,
    mode = "confirm",
    initialValue = "",
    placeholder = "",
    confirmLabel = "OK",
    danger = false,
    confirmDisabled = false,
    onconfirm,
  }: {
    open?: boolean;
    title: string;
    /** Plain-language question or explanation shown above the input (if any). */
    message?: string;
    /** Rich alternative to `message` — e.g. custom body content (bolded text,
     * a toggle, a differently-decorated input). The caller owns its own
     * wrapping/markup entirely; Dialog just renders it as-is between the
     * title and the action buttons. Wins over `message` if both are given. */
    children?: Snippet;
    mode?: "confirm" | "prompt";
    initialValue?: string;
    placeholder?: string;
    confirmLabel?: string;
    /** Styles the confirm button as destructive (e.g. removing something). */
    danger?: boolean;
    /** Disables the confirm button — e.g. while a caller-owned input (via `children`) is empty. */
    confirmDisabled?: boolean;
    /** Called with the trimmed input text in "prompt" mode, or no argument in "confirm" mode. Not called on cancel. */
    onconfirm: (value?: string) => void;
  } = $props();

  let value = $state("");
  let inputEl = $state<HTMLInputElement>();
  let dialogEl = $state<HTMLDivElement>();

  // Reset the field and refocus every time the dialog is (re)opened, not just
  // once at mount — the same Dialog instance is reused for every open. In
  // "confirm" mode there's no input, so focus the dialog itself instead, or
  // Escape/Enter would have nothing inside the modal to bubble through.
  $effect(() => {
    if (open) {
      value = initialValue;
      // Neither element is in the DOM until this render commits.
      queueMicrotask(() => (inputEl ?? dialogEl)?.focus());
    }
  });

  function submit() {
    const trimmed = value.trim();
    if (mode === "prompt" && !trimmed) return;
    open = false;
    onconfirm(mode === "prompt" ? trimmed : undefined);
  }
  function cancel() {
    open = false;
  }
  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") cancel();
    else if (e.key === "Enter" && mode === "prompt") submit();
  }
</script>

{#if open}
  <div class="dialog-backdrop" onclick={cancel} role="presentation">
    <div
      class="dialog"
      bind:this={dialogEl}
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      onkeydown={onkeydown}
      role="dialog"
      aria-modal="true"
      aria-label={title}
    >
      <h3>{title}</h3>
      {#if children}
        {@render children()}
      {:else if message}
        <p class="dialog-msg">{message}</p>
      {/if}
      {#if mode === "prompt"}
        <input bind:this={inputEl} class="tag-input dialog-input" {placeholder} bind:value />
      {/if}
      <div class="dialog-actions">
        <button type="button" class="btn" onclick={cancel}>Cancel</button>
        <button type="button" class="btn primary" class:danger disabled={confirmDisabled} onclick={submit}>{confirmLabel}</button>
      </div>
    </div>
  </div>
{/if}
