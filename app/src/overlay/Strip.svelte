<script lang="ts">
  import type { Alert } from "./types";
  let { alert }: { alert: Alert } = $props();
</script>

<div class="alert glass strip">
  <span class="chip">{alert.tag}</span>
  <span class="chan">{alert.channel}</span>
  <span class="msg"><b>{alert.sender}</b>{alert.text}</span>
  {#if alert.count > 1}<span class="badge">{alert.count}</span>{/if}
</div>
