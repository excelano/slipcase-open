<#
.SYNOPSIS
    Render the tinted tray icons from the one piece of artwork.

.DESCRIPTION
    The tray icon is the interface: its colour answers "is my work safe" without
    a click, and the menu is only there to say what the colour means. So there
    is a set of icons rather than one, and they have to be the same drawing --
    a person recognises the shape and reads the colour, and two drawings would
    make them read the shape instead.

    So this tints, and does not redraw. Every pixel that is brand blue has its
    hue replaced and its lightness nudged; the document, its ruled lines and the
    outline are left exactly as they were, because they are what makes it still
    the same icon. Blue is decided by hue rather than by matching two constants,
    which is what keeps the antialiased edges clean.

    The frames are kept as they are found. Six of the nine are 32-bit DIBs and
    are tinted in their own bytes, so the hand-tuned 16, 20, 24, 32, 40 and 48
    pixel drawings survive -- downscaling the 256 to 16 instead would be mush at
    exactly the size the tray uses. The three PNG frames are decoded, tinted and
    re-encoded, and the directory is rebuilt around whatever length they come
    back as.

    Run it when the artwork changes. The output is checked in, because a build
    step that needs System.Drawing is not one the Linux and macOS builds should
    ever have to satisfy.
#>

[CmdletBinding()]
param(
    [string] $Base,
    [string] $OutDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Not in the param defaults: $PSScriptRoot is empty there under -File.
if (-not $Base)   { $Base   = Join-Path $PSScriptRoot 'slipcase-open.ico' }
if (-not $OutDir) { $OutDir = $PSScriptRoot }

Add-Type -AssemblyName System.Drawing

# What each state looks like. Hue in degrees, saturation absolute, lightness as
# a delta on whatever the pixel already had -- so the two blues stay two tones
# and the drawing keeps its shading.
#
# The meanings live in the Rust side; here they are only colours. Red is the one
# that must not be spent on anything routine, which is why there is an orange
# and a yellow at all.
$States = @(
    # A brighter blue, not a new colour: the pulse while a save is going back
    # into its container has to read as the same icon flashing, not as a state.
    @{ Name = 'working'; Hue = 202; Sat = 0.95; Light =  0.16 }
    @{ Name = 'yellow';  Hue =  44; Sat = 0.97; Light =  0.06 }
    @{ Name = 'orange';  Hue =  26; Sat = 0.98; Light =  0.02 }
    @{ Name = 'red';     Hue = 354; Sat = 0.90; Light = -0.02 }
)

# Which pixels are the slipcase. Brand blue sits near 215 degrees; the window is
# wide enough to catch the antialiasing and narrow enough to leave the grey
# ruled lines on the document alone, which have no hue worth the name.
$BlueFrom = 175
$BlueTo   = 285
$MinSat   = 0.10

function ConvertTo-Hsl([double] $r, [double] $g, [double] $b) {
    $max = [Math]::Max($r, [Math]::Max($g, $b))
    $min = [Math]::Min($r, [Math]::Min($g, $b))
    $l = ($max + $min) / 2.0
    $d = $max - $min
    if ($d -lt 1e-9) { return @(0.0, 0.0, $l) }
    $s = if ($l -gt 0.5) { $d / (2.0 - $max - $min) } else { $d / ($max + $min) }
    $h = if ($max -eq $r) {
        60.0 * ((($g - $b) / $d) % 6.0)
    } elseif ($max -eq $g) {
        60.0 * ((($b - $r) / $d) + 2.0)
    } else {
        60.0 * ((($r - $g) / $d) + 4.0)
    }
    if ($h -lt 0) { $h += 360.0 }
    return @($h, $s, $l)
}

function ConvertFrom-Hsl([double] $h, [double] $s, [double] $l) {
    if ($s -lt 1e-9) { return @($l, $l, $l) }
    $q = if ($l -lt 0.5) { $l * (1.0 + $s) } else { $l + $s - $l * $s }
    $p = 2.0 * $l - $q
    $out = @()
    foreach ($shift in @((1.0 / 3.0), 0.0, (-1.0 / 3.0))) {
        $t = $h / 360.0 + $shift
        if ($t -lt 0) { $t += 1.0 }
        if ($t -gt 1) { $t -= 1.0 }
        $v = if ($t -lt 1.0 / 6.0) { $p + ($q - $p) * 6.0 * $t }
             elseif ($t -lt 0.5)   { $q }
             elseif ($t -lt 2.0 / 3.0) { $p + ($q - $p) * (2.0 / 3.0 - $t) * 6.0 }
             else { $p }
        $out += $v
    }
    return $out
}

# One pixel, in place, in a BGRA buffer. Returns nothing; edits the array.
function Set-TintedPixel([byte[]] $buf, [int] $at, [hashtable] $state) {
    if ($buf[$at + 3] -eq 0) { return }   # fully transparent: no colour to move
    $b = $buf[$at]     / 255.0
    $g = $buf[$at + 1] / 255.0
    $r = $buf[$at + 2] / 255.0
    $hsl = ConvertTo-Hsl $r $g $b
    $h = $hsl[0]; $s = $hsl[1]; $l = $hsl[2]
    if ($s -lt $MinSat -or $h -lt $BlueFrom -or $h -gt $BlueTo) { return }
    $nl = [Math]::Max(0.04, [Math]::Min(0.96, $l + $state.Light))
    $rgb = ConvertFrom-Hsl ([double] $state.Hue) ([double] $state.Sat) $nl
    $buf[$at]     = [byte] [Math]::Round([Math]::Max(0.0, [Math]::Min(1.0, $rgb[2])) * 255.0)
    $buf[$at + 1] = [byte] [Math]::Round([Math]::Max(0.0, [Math]::Min(1.0, $rgb[1])) * 255.0)
    $buf[$at + 2] = [byte] [Math]::Round([Math]::Max(0.0, [Math]::Min(1.0, $rgb[0])) * 255.0)
}

# A PNG frame: decode, tint every pixel, hand back PNG bytes.
function Get-TintedPng([byte[]] $png, [hashtable] $state) {
    $in = New-Object System.IO.MemoryStream(,$png)
    $src = [System.Drawing.Image]::FromStream($in)
    $bmp = New-Object System.Drawing.Bitmap($src.Width, $src.Height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $gfx = [System.Drawing.Graphics]::FromImage($bmp)
    $gfx.Clear([System.Drawing.Color]::Transparent)
    $gfx.DrawImage($src, 0, 0, $src.Width, $src.Height)
    $gfx.Dispose()
    $src.Dispose()
    $in.Dispose()

    $rect = New-Object System.Drawing.Rectangle(0, 0, $bmp.Width, $bmp.Height)
    $data = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadWrite, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $count = $data.Stride * $bmp.Height
    $buf = New-Object byte[] $count
    [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $buf, 0, $count)
    for ($at = 0; $at -lt $count; $at += 4) { Set-TintedPixel $buf $at $state }
    [System.Runtime.InteropServices.Marshal]::Copy($buf, 0, $data.Scan0, $count)
    $bmp.UnlockBits($data)

    $out = New-Object System.IO.MemoryStream
    $bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    $bytes = $out.ToArray()
    $out.Dispose()
    return ,$bytes
}

# A 32-bit DIB frame: 40 bytes of header, then BGRA bottom-up, then the AND
# mask. Only the colour rows are touched, so the header and the mask travel
# through untouched and the frame stays exactly the length it was.
function Get-TintedDib([byte[]] $dib, [int] $w, [int] $h, [hashtable] $state) {
    $out = $dib.Clone()
    $header = [BitConverter]::ToUInt32($dib, 0)
    $pixels = $w * $h * 4
    if ($header + $pixels -gt $dib.Length) { return ,$out }
    for ($at = 0; $at -lt $pixels; $at += 4) { Set-TintedPixel $out ($header + $at) $state }
    return ,$out
}

$raw = [System.IO.File]::ReadAllBytes($Base)
$frames = [BitConverter]::ToUInt16($raw, 4)
Write-Host "$([System.IO.Path]::GetFileName($Base)): $frames frames"

foreach ($state in $States) {
    $bodies = @()
    $entries = @()
    for ($i = 0; $i -lt $frames; $i++) {
        $at = 6 + $i * 16
        $entry = New-Object byte[] 16
        [Array]::Copy($raw, $at, $entry, 0, 16)
        $w = if ($entry[0] -eq 0) { 256 } else { [int] $entry[0] }
        $h = if ($entry[1] -eq 0) { 256 } else { [int] $entry[1] }
        $size = [BitConverter]::ToUInt32($raw, $at + 8)
        $offset = [BitConverter]::ToUInt32($raw, $at + 12)
        $body = New-Object byte[] $size
        [Array]::Copy($raw, $offset, $body, 0, $size)

        # A PNG frame announces itself; anything else in a modern icon is a DIB.
        $tinted = if ($body[0] -eq 0x89 -and $body[1] -eq 0x50) {
            Get-TintedPng $body $state
        } else {
            Get-TintedDib $body $w $h $state
        }
        $bodies += ,$tinted
        $entries += ,$entry
    }

    # Rebuild the directory: a re-encoded PNG is rarely the length it arrived as.
    $out = New-Object System.IO.MemoryStream
    $write = New-Object System.IO.BinaryWriter($out)
    $write.Write([uint16] 0)
    $write.Write([uint16] 1)
    $write.Write([uint16] $frames)
    $offset = 6 + $frames * 16
    for ($i = 0; $i -lt $frames; $i++) {
        $entry = $entries[$i]
        [Array]::Copy([BitConverter]::GetBytes([uint32] $bodies[$i].Length), 0, $entry, 8, 4)
        [Array]::Copy([BitConverter]::GetBytes([uint32] $offset), 0, $entry, 12, 4)
        $write.Write($entry)
        $offset += $bodies[$i].Length
    }
    foreach ($body in $bodies) { $write.Write($body) }
    $write.Flush()

    $path = Join-Path $OutDir "slipcase-open-$($state.Name).ico"
    [System.IO.File]::WriteAllBytes($path, $out.ToArray())
    $write.Dispose()
    $out.Dispose()
    Write-Host "  wrote $([System.IO.Path]::GetFileName($path))"
}

exit 0
