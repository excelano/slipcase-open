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

    THE CERTIFICATION KIT, AND WHY ITS GATE IS A LIST RATHER THAN A COUNT

    `-Certify` runs the Windows App Certification Kit over the signed copy and
    refuses on anything the kit says that is not already in `$KNOWN_FINDINGS`.
    It needs `-SelfSign`, because the kit installs what it tests, and it needs
    an elevated session, which it checks for rather than discovering halfway
    through.

    Refusing on *anything not PASS* would be the obvious gate and the wrong one:
    the sibling has a finding that fails on every run it will ever do, so that
    gate would be red always, and a check whose red is the normal state
    announces nothing. This one is quiet when the kit says what it said last
    time and loud when it says anything else — including when a known test
    changes its verdict, or stops being reported at all.

    `-ReadReport <path>` applies the same gate to a report that already exists
    and does nothing else. That is what makes the gate testable: it builds
    nothing, needs no elevation, and lets somebody break `$KNOWN_FINDINGS` on
    purpose and watch this refuse.

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

.PARAMETER NoBuild
    Package whatever binary is already there, without building.

    Only for packaging a binary that came from somewhere else. This script used
    to behave this way always -- it checked that the binary existed and said
    nothing about how old it was -- and on 2026-09-03 that shipped a package
    built from a binary eight hours stale, whose refusal of an executable
    payload was simply absent from it. The hash check that was supposed to catch
    this compared the staged file against the installed one, which is two copies
    of the same stale binary agreeing with each other. Building here is what
    makes the package a statement about the source.
#>
[CmdletBinding()]
param(
    [switch] $SelfSign,
    # Where the packages are written. `dist` unless asked otherwise; the store
    # submission asks for `dist\submit`, which is where fenster's submit.ps1
    # looks for the package it uploads.
    [string] $OutDir,
    [ValidateSet('release', 'debug')]
    [string] $Configuration = 'release',
    [switch] $NoBuild,
    # Run the Windows App Certification Kit over the package and fail on it.
    # Needs elevation, and needs the package to be installable, so it needs
    # -SelfSign as well.
    [switch] $Certify,
    # Apply the -Certify gate to a report that already exists, and do nothing
    # else. Needs no elevation and builds nothing, which is what makes the gate
    # checkable at all: breaking KNOWN_FINDINGS on purpose and watching this
    # refuse is the only way to know it still bites, and a kit run costs an
    # elevated session and several minutes.
    [string] $ReadReport
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

# What the Windows App Certification Kit says about this package every time, so
# that `-Certify` can be quiet about those and loud about anything else.
#
# **This is a record of what is known, not a claim that it is acceptable.**
# Recording a finding here does not take the decision to ship with it;
# `RELEASE.md` carries that, with the tracing behind it.
#
# It started empty on purpose -- a baseline written before the first run is a
# list of things somebody assumed -- and the first run, 2026-09-06, refused with
# two findings. One is below. The other was fixed instead, and the re-run came
# back PASS with only the entry below reported.
#
# Shrink this list when a finding goes away; the run says so when one does.
#
#   Blocked executables   `ShellExecuteExW`, which is concept 5 step 7 and the
#                         thing this product is for: hand the payload to the
#                         desktop, deliberately without the two flags that would
#                         switch off the Mark-of-the-Web check. Removing the
#                         reference would remove the application. Read out of
#                         our own report on 2026-09-06 rather than borrowed:
#                         `APP_TYPE="Centennial"`, and the kit marks this task
#                         optional for Centennial packages -- which is why
#                         `OVERALL_RESULT` reads WARNING over a test reading
#                         FAIL. `RELEASE.md` carries the decision to submit with
#                         it outstanding.
#
# `DPIAwarenessValidation` was the second finding on that run. It never entered
# this list: it was fixed instead -- there was no PE application manifest for the
# kit to read, and `build.rs` embeds one now. The re-run on the same day reported
# it not at all and took the overall verdict from WARNING to PASS.
$KNOWN_FINDINGS = @{
    'Blocked executables' = 'FAIL'
}

# Read a certification report and apply the gate. A function, so that it can be
# run against a report on its own -- `-ReadReport` -- which is the only way to
# check that the gate bites without an elevated session and a fresh kit run.
function Test-CertificationReport([string] $report) {
    # The verdict is read out of the report and never out of an exit code. A kit
    # that ran and failed and a kit that never ran are different things, and a
    # missing verdict is a refusal too.
    [xml] $xml = Get-Content -LiteralPath $report
    $reportNode = $xml.SelectSingleNode('/REPORT')
    if (-not $reportNode) { Refuse "no REPORT element in $report - read it rather than trusting this script" }
    $overall = $reportNode.GetAttribute('OVERALL_RESULT')
    if (-not $overall) {
        Refuse "the report at $report has no OVERALL_RESULT - read it rather than trusting this script"
    }

    # Read out of `<TEST><RESULT>` and not out of the overall attribute. Only the
    # report element carries that attribute, and the kit does not escalate a
    # failing test into it -- a test can read FAIL under an overall of WARNING --
    # so a gate that looked only at the overall would pass a failing test in
    # silence. Whatever this refuses on, it says which test and why.
    $unexpected = @()
    $seen = @{}
    foreach ($test in $xml.SelectNodes('//TEST')) {
        $node = $test.SelectSingleNode('RESULT')
        if (-not $node) { continue }
        $verdict = $node.InnerText.Trim()
        if ($verdict -eq 'PASS') { continue }
        $name = $test.GetAttribute('NAME')
        $seen[$name] = $verdict
        # ContainsKey rather than indexing: `Set-StrictMode -Version Latest` is
        # on in this script, and a missing key is the ordinary case here.
        if ($KNOWN_FINDINGS.ContainsKey($name) -and $KNOWN_FINDINGS[$name] -eq $verdict) {
            Write-Host "$verdict  $name  (known - see RELEASE.md)"
        } else {
            $unexpected += "$verdict $name"
            Write-Host "$verdict  $name  ** NOT IN THE KNOWN LIST **" -ForegroundColor Yellow
        }
        foreach ($message in $test.SelectNodes('.//MESSAGE')) {
            $text = $message.GetAttribute('TEXT')
            if ($text) { Write-Host "        $text" }
        }
    }

    # A known finding that stopped being reported is good news rather than a
    # refusal, and it is said out loud: a baseline nobody ever shrinks becomes a
    # list of things that used to be true.
    foreach ($name in $KNOWN_FINDINGS.Keys) {
        if (-not $seen.ContainsKey($name)) {
            Write-Host "gone   $name is no longer reported - take it out of KNOWN_FINDINGS"
        }
    }
    Write-Host "certification kit: $overall  ($report)"

    # **The gate is the comparison against the list, not the count of things
    # that are not PASS.** A check whose red is the normal state announces
    # nothing -- which is `CLAUDE.md`'s rule about a green-looking gate, in the
    # other direction. This one is quiet when the kit says what it said last
    # time and loud when it says anything else.
    if ($unexpected) {
        Refuse "the certification kit reported $($unexpected.Count) finding(s) not in the known list: $($unexpected -join '; ') - certification runs this too, so it comes back"
    }
    # An overall of FAIL is still a refusal on its own: the kit does not
    # escalate a failing test into it, so if it ever says FAIL it has decided
    # something the per-test list does not cover.
    if ($overall -eq 'FAIL') {
        Refuse 'the Windows App Certification Kit says FAIL overall'
    }
}

# Nothing below this has run yet, which is the point: a report is read on its
# own, without building, packing or signing anything.
if ($ReadReport) {
    if (-not (Test-Path -LiteralPath $ReadReport)) { Refuse "no report at $ReadReport" }
    Test-CertificationReport (Resolve-Path -LiteralPath $ReadReport).Path
    exit 0
}

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

# --- the version, from the one parser ---------------------------------------

# `packaging/version.sh` is the only thing that reads Cargo.toml's version, and
# it is asked here rather than copied. This script used to carry its own regex
# over Cargo.toml, which was a second parser of the same number and disagreed
# with the fleet about how to spell it.
#
# It is POSIX sh, so it needs a shell, and Git for Windows ships one. Not
# `bash` off PATH: on a machine with WSL that name resolves to
# C:\Windows\System32\bash.exe, which runs inside a Linux distribution where
# this checkout is at a different path, so version.sh would read a Cargo.toml
# that is not this one - or nothing at all.
$git = Get-Command git -ErrorAction SilentlyContinue
if (-not $git) { Refuse 'git is not on PATH, and version.sh needs the shell Git for Windows ships' }
$gitRoot = Split-Path -Parent (Split-Path -Parent $git.Source)
$sh = Join-Path $gitRoot 'bin\bash.exe'
if (-not (Test-Path $sh)) { $sh = Join-Path $gitRoot 'usr\bin\sh.exe' }
if (-not (Test-Path $sh)) {
    Refuse "no shell found beside $($git.Source) - version.sh is POSIX sh and needs the one Git for Windows installs"
}
$versionScript = (Join-Path $here '..\version.sh').Replace('\', '/')
$version = & $sh $versionScript
if ($LASTEXITCODE -ne 0 -or -not $version) { Refuse 'version.sh would not answer' }
$version = ($version | Select-Object -First 1).Trim()
$versionAppx = & $sh $versionScript --appx
if ($LASTEXITCODE -ne 0 -or -not $versionAppx) { Refuse 'version.sh --appx would not answer' }
$versionAppx = ($versionAppx | Select-Object -First 1).Trim()
# The Store requires four parts with the fourth 0, and version.sh says so too.
# This is the check that shelling out produced what was asked for rather than a
# message on standard output.
if ($versionAppx -notmatch '^\d+\.\d+\.\d+\.0$') {
    Refuse "version.sh --appx said '$versionAppx', which is not four parts ending in 0"
}
Step "version $version -> $versionAppx"

# --- the binary ------------------------------------------------------------
$binary = Join-Path $root "target\$Configuration\slipcase-open.exe"
if ($NoBuild) {
    Step "not building; packaging whatever is at target\$Configuration"
} else {
    Step "cargo build --$Configuration"
    Push-Location $root
    try {
        if ($Configuration -eq 'release') { & cargo build --release } else { & cargo build }
    } finally {
        Pop-Location
    }
    if ($LASTEXITCODE -ne 0) { Refuse 'cargo build failed' }
}
if (-not (Test-Path -LiteralPath $binary)) {
    Refuse "no binary at $binary."
}
# Said out loud, because the failure this replaces was silent: a stale binary
# looks exactly like a fresh one, and every check downstream of here -- the
# import check, the hash of what got installed -- passes on it happily.
Step "packaging a binary built $((Get-Item -LiteralPath $binary).LastWriteTime)"
Step 'checking what it imports'
$global:LASTEXITCODE = 0
& (Join-Path $here 'check-imports.ps1') -Binary $binary
if ($LASTEXITCODE -ne 0) { Refuse 'the import check refused this binary' }

# --- stage -----------------------------------------------------------------
if (-not $OutDir) { $OutDir = Join-Path $root 'dist' }
$dist = $OutDir
New-Item -ItemType Directory -Force -Path $dist | Out-Null
$stage = Join-Path $dist 'stage'
if (Test-Path -LiteralPath $stage) { Remove-Item -Recurse -Force -LiteralPath $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

Copy-Item -LiteralPath $binary -Destination (Join-Path $stage 'slipcase-open.exe')
Copy-Item -Recurse -LiteralPath (Join-Path $here 'assets') -Destination (Join-Path $stage 'Assets')
# Beside the binary rather than under Assets: `present::tray` loads these by
# path at run time, because compiling them in as resources would need `rc.exe`
# and this project has no build step to put one in.
#
# The whole set, not just the base. The icon is the tray's interface and its
# colour is what it says; a package carrying only the blue one would answer
# "everything is fine" to every question it was ever asked. Missing files fall
# back rather than failing, which is what makes that failure a silent one --
# so it is checked here instead.
foreach ($state in @('', '-working', '-yellow', '-orange', '-red')) {
    $ico = "slipcase-open$state.ico"
    $from = Join-Path $here $ico
    if (-not (Test-Path -LiteralPath $from)) {
        throw "$ico is missing. Run packaging/windows/make-ico.ps1."
    }
    Copy-Item -LiteralPath $from -Destination (Join-Path $stage $ico)
}

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

# `Excelano.SlipcaseOpen` names the package `SlipcaseOpen-<four-part>-x64.msix`,
# which is what every build-msix.ps1 in the fleet writes and what fenster's
# submit.ps1 builds from the identity name when it goes looking for the upload.
$product = ($identity.Name -split '\.')[-1]
$unsigned = Join-Path $dist "$product-$versionAppx-x64.msix"
Step "packing $unsigned"
& $makeappx pack /d $stage /p $unsigned /o | Out-Host
if ($LASTEXITCODE -ne 0) { Refuse 'makeappx refused the package' }
Write-Host "build-msix: unsigned package (this is the Store upload)" -ForegroundColor Green
Write-Host "  $unsigned"

if (-not $SelfSign) { exit 0 }

# --- a throwaway signature, for installing here ----------------------------
$signed = Join-Path $dist "$product-$versionAppx-x64-selfsigned.msix"
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

# --- the certification kit --------------------------------------------------

if ($Certify) {
    if (-not $SelfSign) {
        Refuse '-Certify needs -SelfSign: the kit installs the package it tests, and an unsigned one will not install'
    }
    $elevated = ([Security.Principal.WindowsPrincipal] `
            [Security.Principal.WindowsIdentity]::GetCurrent()
        ).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $elevated) {
        Refuse 'the Windows App Certification Kit needs an elevated session - rerun this from an administrator prompt'
    }
    $appcert = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\App Certification Kit\appcert.exe'
    if (-not (Test-Path -LiteralPath $appcert)) {
        Refuse "no appcert.exe at $appcert - the App Certification Kit is a separate feature of the Windows SDK installer"
    }

    $report = Join-Path $dist "wack-$version.xml"

    # **The old report is removed, and the new one is checked for being newer
    # than this run.** Both, and the sibling learned why: `appcert` refuses to
    # overwrite a report, printing "Please specify a unique report file name"
    # and stopping before it runs a single test. The file was still there,
    # `Test-Path` was satisfied, and the findings printed were the *previous*
    # package's -- on a run whose whole purpose was to test a different package
    # under the same version number.
    #
    # A kit that ran and failed and a kit that never ran must not come out the
    # same, and *stale* is a third state neither of those words covers.
    if (Test-Path -LiteralPath $report) { Remove-Item -LiteralPath $report -Force }
    $startedAt = Get-Date
    Step 'running the certification kit, which takes several minutes'
    & $appcert reset | Out-Null
    & $appcert test -appxpackagepath $signed -reportoutputpath $report
    if (-not (Test-Path -LiteralPath $report)) {
        Refuse "the certification kit wrote no report to $report"
    }
    if ((Get-Item -LiteralPath $report).LastWriteTime -lt $startedAt) {
        Refuse "the report at $report is older than this run - the kit did not write it, so nothing below would be about this package"
    }

    Test-CertificationReport $report
}
