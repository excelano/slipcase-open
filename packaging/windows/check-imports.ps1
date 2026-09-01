<#
.SYNOPSIS
    Refuse a binary that imports a DLL Windows does not ship, and report what
    the PE header says about it.

.DESCRIPTION
    The same check as `slipcase-desktop/packaging/windows/check-imports.ps1`,
    and it is here rather than shared because the allowlist is the point and the
    two products' lists differ: that one is a GUI application and imports the
    window and rendering stack, this one opens a pipe and calls the shell.

    Why it exists at all is borrowed evidence, and worth repeating because it is
    the kind that is expensive to learn twice. Slipcase 0.1.1 failed Microsoft
    Store certification on 2026-08-29 under policy 10.2.4.1: the package
    installed on a tester's clean machine and would not start, because
    `VCRUNTIME140.dll` ships in the Visual C++ Redistributable and not in
    Windows. Nothing that project ran could have caught it, because every
    machine it built on had Visual Studio and so had already hidden the fault.
    The defect is invisible from inside the toolchain that causes it, so the
    check has to be about the artefact rather than about whether it runs here.

    It parses the PE import table itself rather than shelling out to dumpbin,
    because dumpbin comes with Visual C++ and a check that needs the toolchain
    cannot run where the toolchain is absent.

.PARAMETER Binary
    The executable to read. Defaults to the release build.
#>
[CmdletBinding()]
param(
    [string] $Binary
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Resolved here rather than as a parameter default: `$PSScriptRoot` is empty
# while a default is being evaluated under `-File`, and the failure names
# Join-Path rather than the cause.
if (-not $Binary) {
    $here = Split-Path -Parent $MyInvocation.MyCommand.Path
    $Binary = Join-Path $here '..\..\target\release\slipcase-open.exe'
}

function Refuse([string] $why) {
    Write-Host "check-imports: $why" -ForegroundColor Red
    exit 1
}

# Measured from this binary on 2026-09-01 and then written down, rather than
# copied from the sibling and trimmed. Each is here because something in the
# code asks for it:
#
#   advapi32          the process token, its user SID, and the SDDL that
#                     becomes the named pipe's descriptor  (endpoint::pipe)
#   ole32             CoInitializeEx, because ShellExecuteEx hands work to
#                     shell extensions that want an apartment  (platform::shell)
#   shell32           ShellExecuteExW itself
#   kernel32, ntdll   the pipe, the file information call, the rest of std
#   bcryptprimitives  hashing inside the standard library
#
# The list is deliberately explicit: a dependency this project has never seen
# before should stop a build and be looked at by a person, which is the step
# that was missing when VCRUNTIME140.dll arrived.
$InBox = @(
    'advapi32.dll', 'bcryptprimitives.dll', 'kernel32.dll',
    'ntdll.dll', 'ole32.dll', 'shell32.dll'
)

# API set contracts are resolved by the loader from the schema inside Windows
# itself, so there is no file to be missing. `api-ms-win-crt-*` is the Universal
# C Runtime, which is a Windows component from Windows 10 onward — it is the
# *Visual C++* runtime beside it that is not.
$InBoxPrefixes = @('api-ms-win-', 'ext-ms-win-')

if (-not (Test-Path -LiteralPath $Binary)) {
    Refuse "no such binary: $Binary. Run 'cargo build --release' first."
}
$Binary = (Resolve-Path -LiteralPath $Binary).Path
$bytes = [System.IO.File]::ReadAllBytes($Binary)

function U16([int] $at) { [BitConverter]::ToUInt16($bytes, $at) }
function U32([int] $at) { [BitConverter]::ToUInt32($bytes, $at) }

$pe = U32 0x3c
if ([System.Text.Encoding]::ASCII.GetString($bytes, $pe, 4) -ne "PE`0`0") {
    Refuse 'not a PE image'
}
$machine    = U16 ($pe + 4)
$sections   = U16 ($pe + 6)
$optSize    = U16 ($pe + 20)
$magic      = U16 ($pe + 24)
$subsystem  = U16 ($pe + 24 + 68)

# The header facts a package depends on, reported whether or not they refuse.
$machineName = switch ($machine) { 0x8664 { 'x64' } 0x014c { 'x86' } 0xaa64 { 'arm64' } default { "0x{0:x4}" -f $machine } }
$subsystemName = switch ($subsystem) { 2 { 'WINDOWS_GUI' } 3 { 'WINDOWS_CUI (console)' } default { "$subsystem" } }
Write-Host "check-imports: $Binary"
Write-Host "  machine   : $machineName"
Write-Host "  subsystem : $subsystemName"

if ($machine -ne 0x8664) {
    Refuse "the manifest declares x64 and this is $machineName"
}

# The subsystem is reported and not refused, which is where this differs from
# the sibling's copy. That one is a windowed application and a console
# subsystem would be a mistake; this one is a command line by design — concept
# 9 keeps it as the floor beneath the notifications — and what a console
# subsystem costs a packaged double-click is a measurement nobody has taken
# yet. When it has been taken, this is where the answer goes.

$dataDir = $pe + 24 + $(if ($magic -eq 0x20b) { 112 } else { 96 })
$importRva = U32 ($dataDir + 8)
if ($importRva -eq 0) { Refuse 'no import table' }

$secTable = $pe + 24 + $optSize
$map = @()
for ($i = 0; $i -lt $sections; $i++) {
    $o = $secTable + 40 * $i
    $map += [pscustomobject]@{ Va = U32 ($o + 12); Size = U32 ($o + 16); Raw = U32 ($o + 20) }
}
function ToOffset([uint32] $rva) {
    foreach ($s in $map) {
        if ($rva -ge $s.Va -and $rva -lt ($s.Va + $s.Size)) { return [int]($s.Raw + ($rva - $s.Va)) }
    }
    return -1
}

$names = @()
$at = ToOffset $importRva
while ($true) {
    $nameRva = U32 ($at + 12)
    if ($nameRva -eq 0) { break }
    $o = ToOffset $nameRva
    if ($o -lt 0) { break }
    $end = $o
    while ($bytes[$end] -ne 0) { $end++ }
    $names += [System.Text.Encoding]::ASCII.GetString($bytes, $o, $end - $o).ToLowerInvariant()
    $at += 20
}

$names = $names | Sort-Object -Unique
$strangers = @()
foreach ($n in $names) {
    $known = ($InBox -contains $n)
    if (-not $known) {
        foreach ($p in $InBoxPrefixes) { if ($n.StartsWith($p)) { $known = $true; break } }
    }
    Write-Host ("  {0} {1}" -f $(if ($known) { ' ' } else { '!' }), $n)
    if (-not $known) { $strangers += $n }
}

Write-Host "  $($names.Count) distinct imports, $($strangers.Count) not known to ship with Windows"
if ($strangers.Count -gt 0) {
    Refuse "not shipped with Windows: $($strangers -join ', '). If one of these really is in-box, add it to `$InBox with a note saying how that was confirmed."
}
Write-Host 'check-imports: every import ships with Windows' -ForegroundColor Green
# Explicit, so a caller can read `$LASTEXITCODE`: a script that returns
# without calling `exit` sets it not at all, and under `Set-StrictMode` reading
# it is then an error rather than a zero.
exit 0
