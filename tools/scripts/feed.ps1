param(
  [string]$Dir,
  [int]$Seconds = 60,
  [int]$Every = 4
)
# Synthetic EVE-style logs for two characters in one channel, appended on a timer.
# Writes only into $Dir (a temp folder), never into the real Chatlogs folder.
$sep = '-' * 63
$utf16 = [Text.Encoding]::Unicode
function Header($listener, $now) {
  "`r`n$sep`r`n  Channel ID:      local`r`n  Channel Name:    Local`r`n  Listener:        $listener`r`n  Session started: $($now.ToString('yyyy.MM.dd HH:mm:ss'))`r`n$sep`r`n"
}
New-Item -ItemType Directory -Force $Dir | Out-Null
$now = [DateTime]::UtcNow
$chars = @(@{ Name = 'Jarna'; Id = '111' }, @{ Name = 'Psianna Archeia'; Id = '222' })
$streams = @()
foreach ($c in $chars) {
  $path = Join-Path $Dir ("Local_{0}_{1}.txt" -f $now.ToString('yyyyMMdd_HHmmss'), $c.Id)
  $fs = [IO.File]::Open($path, 'Create', 'Write', 'ReadWrite')
  $bytes = [byte[]](0xFF, 0xFE) + $utf16.GetBytes((Header $c.Name $now))
  $fs.Write($bytes, 0, $bytes.Length); $fs.Flush()
  $streams += $fs
}
$n = 0
$end = [DateTime]::UtcNow.AddSeconds($Seconds)
while ([DateTime]::UtcNow -lt $end) {
  Start-Sleep -Seconds $Every
  $n++
  for ($i = 0; $i -lt $streams.Count; $i++) {
    # The second character's copy is stamped one second later, as seen in real logs.
    $t = [DateTime]::UtcNow.AddSeconds($i)
    $line = "$([char]0xFEFF)[ $($t.ToString('yyyy.MM.dd HH:mm:ss')) ] Tester > chatterer-test $n`r`n"
    $b = $utf16.GetBytes($line)
    $streams[$i].Write($b, 0, $b.Length); $streams[$i].Flush()
    Start-Sleep -Milliseconds 300
  }
}
$streams | ForEach-Object { $_.Dispose() }
