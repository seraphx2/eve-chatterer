<script lang="ts">
  let { version }: { version: string } = $props();
</script>

<h1>About</h1>
<section class="card">
  <div class="about-top" style="padding:18px 18px 0">
    <div class="about-icon"><span class="notch"></span></div>
    <div>
      <div style="font-size:17px;font-weight:500">EVE Chatterer</div>
      <div class="pilot-id">Version {version || "…"}</div>
    </div>
  </div>
  <div class="kv">
    <dt>License</dt>
    <dd>MIT</dd>
    <dt>Source</dt>
    <dd>github.com/seraphx2/eve-chatterer</dd>
  </div>
  <div class="update-row">
    <span class="update-status">Update checking isn't wired up yet.</span>
    <button type="button" class="btn" disabled>Check for updates</button>
  </div>
</section>
