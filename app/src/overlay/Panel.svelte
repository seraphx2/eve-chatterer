<script lang="ts">
  import type { Alert } from "./types";
  let { alert }: { alert: Alert } = $props();
</script>

<article class="alert glass panel" style="--life: {alert.lifetimeMs}ms">
  <span class="notch"></span>
  <header><span class="pilot">{alert.pilot}</span><span>{alert.channel}</span></header>
  <p class="text"><b>{alert.sender}</b>{alert.text}</p>
  <p class="why">{alert.reason}</p>
  <footer>
    <div class="life"><i></i><i></i><i></i></div>
    {#if alert.count > 1}<span class="badge">{alert.count}</span>{/if}
  </footer>
</article>
