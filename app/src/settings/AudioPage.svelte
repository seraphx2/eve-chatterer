<h1>Audio</h1>
<p class="lede">
  This page only decides which file plays. Whether a channel makes any sound at all is a separate setting — each channel's own Sound
  toggle.
</p>
<p class="section-note">Not wired up yet — the layout is settled, but choosing a file doesn't do anything yet. Every channel's Sound toggle above is already real.</p>
<section class="card">
  <label class="audio-choice">
    <input type="radio" name="audio-mode" checked disabled />
    <div class="ac-body"><div class="ac-title">No sound</div><div class="ac-sub">Alerts stay silent everywhere.</div></div>
  </label>
  <label class="audio-choice">
    <input type="radio" name="audio-mode" disabled />
    <div class="ac-body">
      <div class="ac-title">Use one sound for everyone</div>
      <div class="ac-sub">eve-chatterer-default.wav <button type="button" class="revert" disabled>Browse…</button></div>
    </div>
  </label>
  <label class="audio-choice">
    <input type="radio" name="audio-mode" disabled />
    <div class="ac-body"><div class="ac-title">A different sound per character</div><div class="ac-sub">A character with nothing chosen uses the sound above.</div></div>
  </label>
</section>
