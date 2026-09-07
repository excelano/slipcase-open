# Photograph what there is to photograph, at a size the Microsoft Store accepts.
#
# The sibling's version of this file points a camera at the application's own
# window. This product has no window, which is the whole design: what a person
# meets is an entry in Explorer's context menu, an icon by the clock with a
# menu, and — when something is wrong — a message box. Those are the three
# things `packaging/store-listing.md` lists, and they are three different
# capture problems.
#
#   -Window <title>   a real window, sized and captured by its frame. The
#                     refusal is one of these.
#   -Region           the whole desktop after a delay, cropped to the size
#                     asked for. A menu is not a window anybody can size, and a
#                     popup closes the moment something else takes the focus, so
#                     the only way to photograph one is on a timer while a
#                     person holds it open.
#   -Refusal          opens a container whose payload is a program under a
#                     document's name, waits for the message box, captures it.
#                     The one shot that can be taken without a person.
#
# Examples:
#   ...\screenshot.ps1 -Refusal -Container ~\Desktop\slipcase-open-demo\invoice.slpc `
#       -Out shots\03-refusal.png
#   ...\screenshot.ps1 -Region -Delay 8 -Out shots\02-tray-menu.png
#
# WHAT THIS WILL NOT DO
#
# Decide whether a screenshot is any good, or choose the container. Which
# container appears in a listing is an editorial decision and is recorded in
# `packaging/store-listing.md` rather than here.
#
# TWO THINGS MEASURED IN THE SIBLING AND INHERITED HERE, BOTH OF THEM PIXELS
#
# `SetWindowPos` sizes the *window rect*, which on Windows 10 carries an
# invisible resize border outside the visible frame. The visible frame is
# `DWMWA_EXTENDED_FRAME_BOUNDS`. And that frame's top edge is one pixel above
# what is actually drawn, so a capture at exactly the frame rect picks up a
# sliver of whatever is behind it.
#
# Author: David M. Anderson
# Built with AI assistance (Claude, Anthropic)

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string] $Out,
    # Capture a window by its title, which is a substring match.
    [string] $Window,
    # Capture the desktop after $Delay seconds instead.
    [switch] $Region,
    # Open a container that must be refused, and capture the refusal.
    [switch] $Refusal,
    [string] $Container,
    # Narrow -Window to one process, which is how a title that anybody's editor
    # or browser might also carry stops being ambiguous.
    [string] $Owner,
    [int] $Delay = 8,
    # The Store's minimum for a desktop screenshot, and the default because
    # anything larger is mostly empty desktop.
    [int] $Width = 1366,
    [int] $Height = 768
)

$ErrorActionPreference = 'Stop'

function Refuse([string] $why) {
    Write-Host "screenshot: $why" -ForegroundColor Red
    exit 1
}
function Step([string] $what) { Write-Host "screenshot: $what" -ForegroundColor Cyan }

if (-not ($Window -or $Region -or $Refusal)) {
    Refuse 'give one of -Window, -Region or -Refusal'
}

Add-Type -AssemblyName System.Drawing
Add-Type -Namespace Shot -Name Win -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
[DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
[DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
public struct RECT { public int Left, Top, Right, Bottom; }
'@

# The frame Windows actually draws, which is not the window rect.
function Get-Frame([IntPtr] $h) {
    $r = New-Object Shot.Win+RECT
    # 9 is DWMWA_EXTENDED_FRAME_BOUNDS.
    $ok = [Shot.Win]::DwmGetWindowAttribute($h, 9, [ref] $r, 16)
    if ($ok -ne 0) { [void][Shot.Win]::GetWindowRect($h, [ref] $r) }
    return $r
}

function Save-Bitmap($bmp, [string] $path) {
    $dir = Split-Path -Parent $path
    if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    Step "wrote $path ($($bmp.Width) x $($bmp.Height))"
    # The Store's floor, checked rather than assumed. A screenshot rejected at
    # upload costs a round trip through a form nobody enjoys.
    if ($bmp.Width -lt 1366 -or $bmp.Height -lt 768) {
        Write-Host "screenshot: WARNING $($bmp.Width)x$($bmp.Height) is under the Store's 1366x768 minimum" -ForegroundColor Yellow
    }
}

function Capture-Screen([int] $w, [int] $h) {
    $screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $x = [Math]::Max(0, [int](($screen.Width - $w) / 2))
    $y = [Math]::Max(0, [int](($screen.Height - $h) / 2))
    $w = [Math]::Min($w, $screen.Width)
    $h = [Math]::Min($h, $screen.Height)
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($x, $y, 0, 0, (New-Object System.Drawing.Size $w, $h))
    $g.Dispose()
    return $bmp
}

Add-Type -AssemblyName System.Windows.Forms

# --- a window, by the process that owns it ----------------------------------
#
# **Matched on the owning process and not on the title**, which is a correction
# rather than a preference. Matching `*Slipcase*` against every window title
# found a terminal that happened to have the word in its tab, and it would have
# photographed that: any window belonging to the person taking the screenshot --
# an editor with the repository open, a browser on the Store listing -- can
# carry the product's name. The process is what cannot be coincidence.
function Shoot-Window([string] $title, [string] $owner) {
    $deadline = (Get-Date).AddSeconds(30)
    $target = $null
    while ((Get-Date) -lt $deadline) {
        $candidates = Get-Process -ErrorAction SilentlyContinue |
            Where-Object { $_.MainWindowHandle -ne 0 -and $_.MainWindowTitle }
        if ($owner) { $candidates = $candidates | Where-Object { $_.ProcessName -eq $owner } }
        if ($title) { $candidates = $candidates | Where-Object { $_.MainWindowTitle -like "*$title*" } }
        $target = $candidates | Select-Object -First 1
        if ($target) { break }
        Start-Sleep -Milliseconds 300
    }
    if (-not $target) {
        $what = if ($owner) { "owned by $owner" } else { "whose title contains '$title'" }
        Refuse "no window $what appeared"
    }
    Step "found '$($target.MainWindowTitle)'"
    [void][Shot.Win]::SetForegroundWindow($target.MainWindowHandle)
    Start-Sleep -Milliseconds 600
    $r = Get-Frame $target.MainWindowHandle
    $w = $r.Right - $r.Left
    $h = $r.Bottom - $r.Top
    if ($w -le 0 -or $h -le 0) { Refuse 'the window reported no size' }
    # Two rows taller than needed, then the top two cropped: the frame's top
    # edge sits one pixel above what is drawn.
    $bmp = New-Object System.Drawing.Bitmap $w, ($h + 2)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object System.Drawing.Size $w, ($h + 2)))
    $g.Dispose()
    $cropped = $bmp.Clone(
        (New-Object System.Drawing.Rectangle 0, 2, $w, $h),
        $bmp.PixelFormat)
    return $cropped
}

if ($Refusal) {
    if (-not $Container) { Refuse '-Refusal needs -Container, pointing at one whose payload is a program' }
    if (-not (Test-Path $Container)) { Refuse "no container at $Container" }
    $Container = (Resolve-Path $Container).Path
    Step 'opening it through the shell, from a launcher with no console'
    # **Through `wscript` and not from here, and that is not fussiness.**
    # `attach_console` joins the console of whatever started this product, and
    # a process started from a console is a command line -- which is the floor
    # concept 9 puts beneath the tray, where there is no icon and no dialog to
    # wait for. So calling the verb straight out of this script photographs an
    # invocation nobody makes: measured 2026-09-06, the box did not appear at
    # all, and the same container opened through the launcher below showed the
    # box and the icon every time.
    #
    # `wscript.exe` is a GUI-subsystem host with no console, which is what
    # Explorer looks like to the process it starts.
    #
    # The verb rather than a double-click: with the viewer installed as well and
    # no UserChoice set, a double-click raises the picker, which cannot be
    # scripted. `packaging/windows/README.md` has both measurements.
    $shell = New-Object -ComObject Shell.Application
    $item = $shell.Namespace((Split-Path -Parent $Container)).ParseName((Split-Path -Leaf $Container))
    $verb = $item.Verbs() | Where-Object { ($_.Name -replace '&', '') -eq 'Open payload' } | Select-Object -First 1
    if (-not $verb) { Refuse "no 'Open payload' verb on that container - is the package installed?" }
    if (-not (Get-Command wscript.exe -ErrorAction SilentlyContinue)) {
        Refuse 'no wscript.exe, and calling the verb from this console would photograph the wrong invocation'
    }
    $launcher = Join-Path ([System.IO.Path]::GetTempPath()) 'slipcase-open-verb.vbs'
    @'
Set shell = CreateObject("Shell.Application")
path = WScript.Arguments(0)
Set folder = shell.Namespace(Left(path, InStrRev(path, "\") - 1))
Set item = folder.ParseName(Mid(path, InStrRev(path, "\") + 1))
For Each v In item.Verbs()
    If Replace(v.Name, "&", "") = "Open payload" Then
        v.DoIt()
        Exit For
    End If
Next
'@ | Set-Content -LiteralPath $launcher -Encoding ascii
    Start-Process wscript.exe -ArgumentList "`"$launcher`"", "`"$Container`""
    # **The window is waited for and the desktop is what gets photographed.**
    # The box is about 370x210, and the Store's floor is 1366x768, so a capture
    # of its frame alone is rejected at upload -- measured 2026-09-06, when this
    # wrote an 8KB image the form would not take. Waiting for the window first
    # is still what proves the refusal happened rather than the desktop being
    # photographed hopefully; `Shoot-Window`'s bitmap is discarded and only its
    # having found something is kept.
    #
    # What is behind the box is editorial and belongs to whoever runs this:
    # tidy the desktop first, because everything on it goes in the listing.
    $found = Shoot-Window '' 'slipcase-open'
    $found.Dispose()
    Start-Sleep -Milliseconds 400
    Save-Bitmap (Capture-Screen $Width $Height) $Out
    Get-Process slipcase-open -ErrorAction SilentlyContinue |
        ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
    exit 0
}

if ($Window) {
    Save-Bitmap (Shoot-Window $Window $Owner) $Out
    exit 0
}

# --- the desktop, on a timer ------------------------------------------------
Step "capturing the desktop in $Delay seconds - open the menu you want photographed and leave it open"
for ($i = $Delay; $i -gt 0; $i--) {
    Write-Host "  $i" -NoNewline
    Write-Host "`r" -NoNewline
    Start-Sleep -Seconds 1
}
Save-Bitmap (Capture-Screen $Width $Height) $Out
