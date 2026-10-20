Add-Type -AssemblyName System.Drawing

# The toolbar icon: a filled disc with a question mark punched out of it.
#
# A "?" and not a lens or a dot: at 16 pixels the question mark is the only shape
# that survives, and it is what the feature actually does. Its colour carries the
# state -- the accent while assist is on, grey while it is off -- so the icon is
# the switch's readout and needs no badge to say whether anything is happening.
$onColour = [System.Drawing.Color]::FromArgb(178, 107, 24)
$offColour = [System.Drawing.Color]::FromArgb(138, 138, 147)

function New-Icon {
    param([int]$size, [System.Drawing.Color]$colour, [string]$path)
    $bitmap = New-Object System.Drawing.Bitmap($size, $size)
    $bitmap.SetResolution(96, 96)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $graphics.Clear([System.Drawing.Color]::Transparent)

    $inset = [Math]::Max(1, [int]($size * 0.06))
    $brush = New-Object System.Drawing.SolidBrush($colour)
    $graphics.FillEllipse($brush, $inset, $inset, $size - 2 * $inset, $size - 2 * $inset)

    # The glyph is drawn white, sized to the disc rather than to the canvas, so the
    # mark and the disc stay in proportion at every size the toolbar asks for.
    $fontSize = [single]($size * 0.66)
    $font = New-Object System.Drawing.Font("Segoe UI", $fontSize, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
    $format = New-Object System.Drawing.StringFormat
    $format.Alignment = [System.Drawing.StringAlignment]::Center
    $format.LineAlignment = [System.Drawing.StringAlignment]::Center
    $white = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)
    $box = New-Object System.Drawing.RectangleF(0, [single]($size * 0.02), $size, $size)
    $graphics.DrawString("?", $font, $white, $box, $format)

    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose()
    $bitmap.Dispose()
    $font.Dispose()
    $format.Dispose()
    $brush.Dispose()
    $white.Dispose()
}

# The icons belong beside the manifest, not beside this script: `$PSScriptRoot` is the
# tools folder, so the extension root is its parent.
$directory = Join-Path (Split-Path $PSScriptRoot -Parent) "icons"
New-Item -ItemType Directory -Force -Path $directory | Out-Null

foreach ($size in 16, 32, 48, 128) {
    New-Icon -size $size -colour $onColour -path (Join-Path $directory "on-$size.png")
    New-Icon -size $size -colour $offColour -path (Join-Path $directory "off-$size.png")
}

Get-ChildItem $directory | Select-Object -ExpandProperty Name
