<script lang="ts">
  import type { Alert } from "./types";
  let { alert }: { alert: Alert } = $props();
  const initial = $derived(alert.pilot.trim().charAt(0).toUpperCase());
</script>

<div class="alert glass strip">
  <span class="chip">{initial}</span>
  <span class="chan">{alert.channel}</span>
  <span class="msg"><b>{alert.sender}</b>{alert.text}</span>
  {#if alert.count > 1}<span class="badge">{alert.count}</span>{/if}
</div>
