# Releasing slipcase-open

What happens, in what order, and what has to be true before any of it. The
reasoning for the product lives in `slipcase-open-concept.md` and the order of
the work in `PLAN.md`; this is only the release.

**Two platforms, not three.** Linux ships from the Excelano apt repository, and
Windows ships to the Microsoft Store. macOS is Phase 5 and is not built — when
it lands, this file grows a section and `packaging/store-listing.md` grows a
second cut.

**Much of what follows is the sibling's experience rather than this project's.**
`slipcase-desktop` has been through Microsoft Store certification, failed it
once, and wrote down why. Where a fact here came from there it says so, because
a borrowed lesson and a measured one are different kinds of thing and the next
person has to be able to tell them apart.

---

## The order

1. **Preflight**, which is local and refuses.
2. **Linux**, which needs no other machine.
3. **Windows**, including the certification kit.
4. **The readiness review**, which is the listing read against the artefact.
5. **Submit.**

**Nothing is submitted until step 4.** A Store submission is an event with a
queue behind it, and the point of the review is that the thing in the queue is
one somebody looked at.

**apt is the exception, taken deliberately.** It is our own repository:
publishing is one command and unpublishing is a prune, and nothing sits in
anybody's review queue meanwhile.

---

## One number, two spellings

`Cargo.toml` holds the version and nothing else should.

| Where | Spelling | Read by |
| --- | --- | --- |
| Cargo, Debian | `0.1.4` | `cargo deb`, `debian/changelog` by hand |
| `AppxManifest` | `0.1.4.0` | `build-msix.ps1`, appending `.0` |

**There is no `version.sh` here, and that is a decision.** The sibling has one
because three artefacts wanted three spellings and two build scripts had each
grown their own `sed`. Here there is one script that needs the number —
`build-msix.ps1`, which reads `Cargo.toml` directly — so a shared parser would
mean teaching PowerShell to call POSIX `sh`, which is machinery the sibling
needed and this does not. `preflight.sh` reads the same line with the same
shape, and its checks 3 and 4 are what keep the two from drifting apart in
silence: the changelog must name Cargo's version, and the version must have a
spelling `build-msix.ps1` can use.

**The third component moves for everything at 0.x**, which is this family's
convention and not semver's. What the rule rules out is reusing a number:
`preflight.sh` refuses when the tag already exists.

---

## Preflight

    ./packaging/preflight.sh              # everything it can check locally
    ./packaging/preflight.sh --ci         # and ask GitHub about HEAD
    ./packaging/preflight.sh --quick      # skip the gate, which is the slow one

It refuses and does not repair. It runs `check.sh` rather than reimplementing
it, and takes the status from it — two gates that could disagree would be worse
than one, and the one that disagreed quietly would be the new one.

**What it cannot check is the listing**, which is step 4 below and a person's
job.

---

## Linux

    cargo build --release
    cargo deb --no-build

`packaging/README.md` has the detail, including the two things worth checking in
the result with `dpkg-deb`: that the shipped policy file will not overwrite an
administrator's, and the trigger behaviour of the directories it writes into.

`debian/changelog` is hand-written and is the record of what changed for
somebody installing the package. Preflight checks it names the version; nothing
can check that it is *true*, which is the same problem the store listing has.

---

## Windows

    cargo build --release
    powershell -File packaging\windows\check-imports.ps1
    powershell -File packaging\windows\build-msix.ps1 -SelfSign
    # then, from an ADMINISTRATOR prompt:
    powershell -File packaging\windows\build-msix.ps1 -SelfSign -Certify

`check-imports.ps1` walks the PE import table and refuses any DLL not known to
ship with Windows. `build-msix.ps1` runs it and will not package a binary that
fails it. **This is why `.cargo/config.toml` links the CRT statically**: the
sibling's 0.1.1 installed on a tester's clean machine and then refused to start,
because `VCRUNTIME140.dll` ships in the Visual C++ Redistributable and Windows
does not carry it. Every machine this project builds on has Visual Studio, so
every machine that could have caught it had already hidden it. Measured here on
2026-09-06: twelve distinct imports, none outside Windows.

**One administrator action gates the local install.** The throwaway signing
certificate must reach `LocalMachine\TrustedPeople`; the per-user store is not
read for package deployment and leaves `Add-AppxPackage` failing `0x800B0109`.
The script prints the command and does not attempt it. The certification kit
needs the same elevated prompt.

**Do not rebuild before uploading.** A rebuild of identical source produces a
different file — the sibling measured 24 bytes, being the COFF timestamp, three
debug directory timestamps and the CodeView PDB GUID. The artefact uploaded has
to be the one the certification kit passed. Two packages come out of one staging
tree with no rebuild between them: the unsigned one is the upload, because the
Store signs what it distributes, and the signed copy is only for installing
here.

### The certification kit

`-Certify` runs it and applies a gate that is a comparison against
`$KNOWN_FINDINGS`, not a count of things that are not PASS. The sibling has a
finding that fails on every run it will ever do, so the obvious gate would be
red always — and a check whose red is the normal state announces nothing.

**Each finding earns its place in `$KNOWN_FINDINGS` only after somebody traces
it, and the decision to submit with it outstanding is recorded here**, below,
with the tracing. The list started empty on purpose — a baseline written before
the first run is a list of assumptions — and the first run refused, which is
what a first run should do.

**Findings accepted so far: one.** Run 2026-09-06, `dist\wack-0.1.4.xml`.

| Finding | Verdict | Why it is accepted |
| --- | --- | --- |
| `Blocked executables` | FAIL | `slipcase-open.exe` references `shell32.dll!ShellExecuteExW`. That call *is* concept §5 step 7 — hand the payload to the registered application, deliberately without `SEE_MASK_NOZONECHECKS`, which is what makes the zone check happen at all. Removing the reference removes the product. Read from our own report: `APP_TYPE="Centennial"`, and the kit marks this task optional for Centennial packages, which is why the overall verdict is WARNING over a test reading FAIL. `slipcase-desktop` shipped with the same finding. |

`DPIAwarenessValidation` was reported WARNING on the same run and was **fixed
rather than accepted**: there was no PE application manifest for the kit to
read, and unlike the sibling — where `winit` sets awareness at run time anyway —
this process genuinely was unaware, so the dialog and the tray icon were being
bitmap-stretched on any high-DPI display. `build.rs` embeds one now, and the
re-run the same day did not report it at all — which took `OVERALL_RESULT` from
WARNING to **PASS**, with `Blocked executables` the only thing still reported and
now quiet because it is baselined.

`-ReadReport <path>` applies the gate to a report that already exists and does
nothing else. That is what makes the gate checkable without an elevated session:
break `$KNOWN_FINDINGS` on purpose and watch it refuse. Checked that way on
2026-09-06 against synthetic reports covering all-pass, an unexpected finding, a
baselined one, a baselined one whose verdict changed, one that has gone away, a
report with no overall verdict, and a report that is not there.

### What the Partner Center form does, from the sibling

Three things no documentation said, all learned there and none re-measured here:

- **The Store logo field takes 1080x1080 or 2160x2160**, not the 300x300 older
  documentation describes. Both are in `packaging/windows/listing/` rather than
  in the package assets, because a file added to the assets lands in the MSIX
  and a package that gains a file has to be certified again.
- **The restricted-capability justification caps at 500 characters**, counting
  newlines, and truncates silently at the paste.
- **The reviewer has nothing to open.** This product without a container puts up
  an icon and does nothing else, and the notes field takes no attachment.

**The submission API cannot make a first submission.** MSIX apps use the API at
`manage.devcenter.microsoft.com`, and it requires one submission to already
exist, made in Partner Center with the age-ratings questionnaire answered. A
submission created through the API must then be edited only through the API.

### The container the reviewer opens

`https://excelano.com/slipcase/quarterly-report.pdf.slpc`, which is **the
sibling's container and deliberately not a second one.** It is built by
`slipcase-desktop/packaging/demo-container.sh`, whose whole reason for existing
is that three demonstrations of the same format that do not look alike is not a
thing to discover after two listings are live. This repository has no
`demo-container.sh` for that reason: a second script writing a second file to
one URL is the failure that one was written to prevent.

Its payload is a PDF, so it exercises the path this product is for: extract,
hand to the registered reader, write back on save.

### Reading the listing back, which needs no login

    $id = (Import-PowerShellDataFile packaging\windows\identity.psd1).StoreId
    winget show --id $id --source msstore
    Invoke-RestMethod ("https://displaycatalog.mp.microsoft.com/v7.0/products/" +
        $id + "?market=US&languages=en-US&fieldsTemplate=Details")

A dashboard reporting its own success is the same evidence as a build script
reporting its own output, so the listing is read back from a public channel.
`winget` gives the description, price, category, publisher and both URLs, so the
description can be **diffed** against `packaging/store-listing.md`. The display
catalogue gives the package: `PackageFullName` carries the version, which is
what says the Store is serving the build that was uploaded.

**What neither serves back** is the screenshots, the search terms, the
`runFullTrust` justification, the notes to certification, and *What's new in this
version*. Those stay write-only, so a check here says so rather than implying
the listing was verified.

---

## The readiness review

The listing makes claims about behaviour. Every one has to be true of the
artefact being uploaded, and nothing automated can read a sentence and decide
whether it is. `packaging/store-listing.md` carries the claims in a table with
where each was established; this is the pass that walks it.

**Against the built package, installed** — not against the checkout, and not
against a debug binary. The rule this project learned twice: a harness that does
not go through the shipped artefact is not measuring the product.

- [ ] Every row of store-listing.md's claims table, checked or explicitly
      deferred with a reason.
- [ ] The description diffed against what `winget` serves for the previous
      version, so an edit made in the form and not here is visible.
- [ ] Screenshots taken **against the packaged build** and still showing what
      the current version does.
- [ ] `debian/changelog`'s newest entry describes this version and is true.
- [ ] The two hand checks below, or an explicit note that they are still
      outstanding.

### Still outstanding, and each needs a person

- **The zone warning.** A marked container's payload carries `ZoneId=3` onto the
  extracted copy and `ShellExecuteEx` is called with the check left on, but that
  the warning is *displayed* has never been watched. store-listing.md's marking
  sentence is deliberately weaker than the code allows, and stays weak until
  somebody has seen it.
- **The ADMX in `gpedit.msc`.** The pair is well formed and every reference in
  it resolves, and a test cross-checks its value names against the reader. That
  the Group Policy editor renders it and writes what is expected needs the files
  in `PolicyDefinitions`, which needs administrator.

---

## What a patch costs afterwards

A Store submission is a queue, so a fix is not a fix until it clears one. That
is the reason for the order at the top and for the review before it: the cheap
moment to catch something is before the queue, and there is no cheap moment
after.

Version numbers are not reused, so a patch is a new number even when the change
is one character — `preflight.sh` refuses when the tag exists, which is that
rule with teeth.

---

## Still to write

- **A macOS section**, with Phase 5.
- **`SUBMITTING.local.md`**, the walk-through of the form as it actually is,
  carrying this account's identifiers. Deliberately not committed, like
  `packaging/windows/identity.psd1`.
