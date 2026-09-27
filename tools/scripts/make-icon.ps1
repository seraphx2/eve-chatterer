# Draws the EVE Chatterer app icon (1024x1024 PNG) from the overlay design language:
# a steel-glass tile, the reason-colored notch on its top edge, and a chat bubble
# carrying the three-segment lifetime meter, frozen part-way through its life.
#
#   pwsh tools/scripts/make-icon.ps1 [-Out docs/design/icon-source.png]
#   cd app; npm run tauri -- icon ../docs/design/icon-source.png   # regenerates src-tauri/icons
param([string]$Out = (Join-Path $PSScriptRoot '..\..\docs\design\icon-source.png'))

Add-Type -AssemblyName System.Drawing
$S = 1024
$bmp = New-Object System.Drawing.Bitmap $S, $S
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = 'AntiAlias'
$g.InterpolationMode = 'HighQualityBicubic'
$g.PixelOffsetMode = 'HighQuality'
$g.Clear([System.Drawing.Color]::Transparent)

function Hex($h, $a = 255) { $c = [System.Drawing.ColorTranslator]::FromHtml($h); [System.Drawing.Color]::FromArgb($a, $c.R, $c.G, $c.B) }
function RoundRect($x, $y, $w, $h, $r) {
  $p = New-Object System.Drawing.Drawing2D.GraphicsPath
  $d = 2 * $r
  $p.AddArc($x, $y, $d, $d, 180, 90); $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
  $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90); $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
  $p.CloseFigure(); $p
}

# The glass tile.
$tile = RoundRect 40 40 944 944 210
$glass = New-Object System.Drawing.Drawing2D.LinearGradientBrush ([System.Drawing.Point]::new(0, 0)), ([System.Drawing.Point]::new($S, $S)), (Hex '#52707f'), (Hex '#14242c')
$g.FillPath($glass, $tile)
$g.DrawPath((New-Object System.Drawing.Pen (Hex '#dcf0f6' 70), 8), $tile)

# The reason-colored notch on the top edge.
$notch = RoundRect 392 40 240 20 10
$g.FillPath((New-Object System.Drawing.SolidBrush (Hex '#55c4d6')), $notch)

# The chat bubble with its tail.
$ink = New-Object System.Drawing.SolidBrush (Hex '#dbe5ea')
$g.FillPath($ink, (RoundRect 190 250 644 430 96))
$tail = [System.Drawing.Point[]]@([System.Drawing.Point]::new(250, 660), [System.Drawing.Point]::new(250, 800), [System.Drawing.Point]::new(400, 668))
$g.FillPolygon($ink, $tail)

# The three-segment meter inside it: two full, one half drained.
$dark = New-Object System.Drawing.SolidBrush (Hex '#24404d')
$segW = 150; $segH = 46; $gap = 24
$total = 3 * $segW + 2 * $gap
$x0 = 190 + (644 - $total) / 2
$y0 = 250 + (430 - $segH) / 2
for ($i = 0; $i -lt 3; $i++) {
  $w = if ($i -eq 2) { $segW * 0.45 } else { $segW }
  $g.FillRectangle($dark, [float]($x0 + $i * ($segW + $gap)), [float]$y0, [float]$w, [float]$segH)
}

$g.Dispose()
$dir = Split-Path -Parent $Out
if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
"wrote $Out"
