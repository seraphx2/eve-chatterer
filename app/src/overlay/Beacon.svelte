<script lang="ts">
  import type { Alert } from "./types";
  let { alert }: { alert: Alert } = $props();
</script>

<article class="alert glass beacon" style="--life: {alert.lifetimeMs}ms">
  <div class="top"><span class="why">{alert.reason}</span><span class="where">{alert.pilot} in {alert.channel}</span></div>
  <p class="sender">{alert.sender}</p>
  <p class="body">{alert.text}</p>
  <div class="foot">
    <div class="life"><i></i><i></i><i></i></div>
    {#if alert.count > 1}<span class="badge">{alert.count}</span>{/if}
  </div>
</article>
