<#
.SYNOPSIS
    Build the MSIX package from a release binary, and optionally sign a copy of
    it so it can be installed here.

.DESCRIPTION
    Concept 15 takes the Microsoft Store for this product, and more strongly
    than it does for the viewer: the audience in concept 2 is the person who was
    sent a container and has nothing that opens it, and a Store listing is how
    that person finds the thing that opens it when Windows offers to search by
    file type.

    TWO PACKAGES OUT OF ONE STAGING TREE, AND WHY THEY MUST NOT BE REBUILT

    The Store signs what it distributes, so the package that goes up is
    unsigned. The signed copy is only for installing on this machine. Both come
    from one staging tree with no rebuild in between, because a rebuild of
    identical source produces a different file — the sibling measured 24 bytes
    of difference, being the COFF timestamp, three debug directory timestamps
    and the CodeView PDB GUID. The artefact uploaded has to be the one that was
    tested, not a fresh build of the same commit.

    ONE ADMINISTRATOR ACTION, WHICH THIS SCRIPT DOES NOT ATTEMPT

    `makeappx`, `New-SelfSignedCertificate`, `signtool` and `Add-AppxPackage` all
    run as an ordinary user, but the test certificate has to reach
    `LocalMachine\TrustedPeople` — the per-user store is not read for this and
    importing there leaves deployment failing 0x800B0109. `-SelfSign` prints the
    two elevated commands rather than trying to run them.

.PARAMETER SelfSign
    Also produce a throwaway-signed copy for installing locally.

.PARAMETER Configuration
    Which cargo profile's binary to package. Release by default; debug is for
    when the thing being tested is the packaging rather than the build.
#>
[CmdletBinding()]
param(
    [switch] $SelfSign,
    [ValidateSet('release', 'debug')]
    [string] $Configuration = 'release'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..\..')).Path

function Refuse([string] $why) {
    Write-Host "build-msix: $why" -ForegroundColor Red
    exit 1
}
function Step([string] $what) { Write-Host "build-msix: $what" -ForegroundColor Cyan }

# --- the identity, which cannot be guessed ---------------------------------
$identityFile = Join-Path $here 'identity.psd1'
if (-not (Test-Path -LiteralPath $identityFile)) {
    Refuse "no identity.psd1. Copy identity.psd1.example beside it and put in the values Partner Center shows under Product management -> Product identity."
}
$identity = Import-PowerShellDataFile -LiteralPath $identityFile
foreach ($k in 'Name', 'Publisher', 'PublisherDisplayName') {
    if (-not $identity.ContainsKey($k) -or -not $identity[$k]) {
        Refuse "identity.psd1 has no $k"
    }
}
# The value most often copied wrong is the one signtool is strictest about: it
# refuses to sign a package whose manifest Publisher and whose certificate
# subject differ, and the display name is not the X.500 string.
if ($identity.Publisher -notmatch '^(CN|O|OU|L|S|C|E)=') {
    Refuse "Publisher is '$($identity.Publisher)', which is not an X.500 string. It is the Package/Identity/Publisher value, not the display name."
}

# --- the version, in the spelling the Store requires -----------------------
$cargo = Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw
if ($cargo -notmatch '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"') {
    Refuse 'could not read version from Cargo.toml'
}
$version = $Matches[1]
# Four parts, and the Store requires the fourth to be 0.
$versionAppx = "$version.0"
Step "version $version -> $versionAppx"

# --- the binary ------------------------------------------------------------
$binary = Join-Path $root "target\$Configuration\slipcase-open.exe"
if (-not (Test-Path -LiteralPath $binary)) {
    Refuse "no binary at $binary. Run 'cargo build --$Configuration' first."
}
Step 'checking what it imports'
$global:LASTEXITCODE = 0
& (Join-Path $here 'check-imports.ps1') -Binary $binary
if ($LASTEXITCODE -ne 0) { Refuse 'the import check refused this binary' }

# --- stage -----------------------------------------------------------------
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist 'stage'
if (Test-Path -LiteralPath $stage) { Remove-Item -Recurse -Force -LiteralPath $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

Copy-Item -LiteralPath $binary -Destination (Join-Path $stage 'slipcase-open.exe')
Copy-Item -Recurse -LiteralPath (Join-Path $here 'assets') -Destination (Join-Path $stage 'Assets')
# Beside the binary rather than under Assets: `present::tray` loads it by path
# at run time, because compiling it in as a resource would need `rc.exe` and
# this project has no build step to put one in.
Copy-Item -LiteralPath (Join-Path $here 'slipcase-open.ico') -Destination (Join-Path $stage 'slipcase-open.ico')

$manifest = Get-Content -LiteralPath (Join-Path $here 'AppxManifest.xml.in') -Raw
$manifest = $manifest.Replace('@IDENTITY_NAME@', $identity.Name)
$manifest = $manifest.Replace('@PUBLISHER@', $identity.Publisher)
$manifest = $manifest.Replace('@PUBLISHER_DISPLAY_NAME@', $identity.PublisherDisplayName)
$manifest = $manifest.Replace('@VERSION_APPX@', $versionAppx)
# A placeholder that survived substitution is a package that will be rejected at
# upload with nothing here to say why, so it is refused now while the name of
# the one that got through can still be printed.
if ($manifest -match '@[A-Z_]+@') {
    Refuse "a placeholder survived substitution: $($Matches[0])"
}
Set-Content -LiteralPath (Join-Path $stage 'AppxManifest.xml') -Value $manifest -Encoding utf8

# --- pack ------------------------------------------------------------------
$kit = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin' -Recurse -Filter 'makeappx.exe' -ErrorAction SilentlyContinue |
       Where-Object { $_.FullName -match '\\x64\\' } | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $kit) { Refuse 'makeappx.exe not found. Install the Windows SDK.' }
$makeappx = $kit.FullName
$signtool = Join-Path (Split-Path -Parent $makeappx) 'signtool.exe'

$unsigned = Join-Path $dist "slipcase-open-$version.msix"
Step "packing $unsigned"
& $makeappx pack /d $stage /p $unsigned /o | Out-Host
if ($LASTEXITCODE -ne 0) { Refuse 'makeappx refused the package' }
Write-Host "build-msix: unsigned package (this is the Store upload)" -ForegroundColor Green
Write-Host "  $unsigned"

if (-not $SelfSign) { exit 0 }

# --- a throwaway signature, for installing here ----------------------------
$signed = Join-Path $dist "slipcase-open-$version-selfsigned.msix"
Copy-Item -LiteralPath $unsigned -Destination $signed -Force

# The subject is built from the manifest's Publisher rather than typed a second
# time, because the two differing is exactly what signtool refuses on.
$existing = Get-ChildItem Cert:\CurrentUser\My -ErrorAction SilentlyContinue |
            Where-Object { $_.Subject -eq $identity.Publisher } | Select-Object -First 1
if ($existing) {
    Step "reusing certificate $($existing.Thumbprint)"
    $cert = $existing
} else {
    Step "making a throwaway certificate for $($identity.Publisher)"
    $cert = New-SelfSignedCertificate -Type Custom -Subject $identity.Publisher `
        -KeyUsage DigitalSignature -FriendlyName 'Slipcase Open test signing' `
        -CertStoreLocation 'Cert:\CurrentUser\My' `
        -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
}

Step 'signing the copy'
& $signtool sign /fd SHA256 /sha1 $cert.Thumbprint $signed | Out-Host
if ($LASTEXITCODE -ne 0) { Refuse 'signtool refused to sign' }

$cer = Join-Path $dist 'slipcase-open-test.cer'
Export-Certificate -Cert $cert -FilePath $cer -Force | Out-Null

Write-Host ''
Write-Host "build-msix: signed copy (local install only)" -ForegroundColor Green
Write-Host "  $signed"
Write-Host ''
Write-Host 'One administrator action is needed before this will install. The per-user'
Write-Host 'certificate store is not read for package deployment, so importing there'
Write-Host 'leaves Add-AppxPackage failing 0x800B0109. In an ELEVATED PowerShell:'
Write-Host ''
Write-Host "  Import-Certificate -FilePath '$cer' -CertStoreLocation Cert:\LocalMachine\TrustedPeople" -ForegroundColor Yellow
Write-Host ''
Write-Host 'Then, as yourself:'
Write-Host ''
Write-Host "  Add-AppxPackage -Path '$signed'" -ForegroundColor Yellow
Write-Host ''
