# Store listing text

The words that go in the Microsoft Store form, kept here so they can be reviewed
and diffed rather than retyped into a web page and forgotten.

**Every claim below has to be true of the built artefact**, and that is the check
`RELEASE.md`'s readiness review runs. It is the error this family has caught most
often — a sentence written before anybody looked. Where a sentence describes
behaviour, `debian/changelog` is where that behaviour was first written down
against a run; if the two disagree, the changelog is right and this is stale.

**One store, and it stays one.** `slipcase-desktop` maintains one draft for two
stores and records the one paragraph that differs between them. This product's
macOS channel is a Homebrew cask rather than the Mac App Store —
`packaging/macos/README.md` says why — so there is no second listing and no
second cut. The cask's `desc` is one sentence and is the crate's own.

Limits, so a later edit does not overrun them:

| Field | Microsoft Store |
| --- | --- |
| App name | reserved, not free text |
| Short description | 1,000 |
| Description | 10,000 |
| Search terms | 7 terms |

---

## App name

    Slipcase Open

Reserved in Partner Center on 2026-09-01. The application calls itself Slipcase
Open everywhere a person sees it — `Package/Properties/DisplayName` and the
`VisualElements` display name both say so.

**Not "Slipcase".** That reservation belongs to the viewer, `slipcase-desktop`,
which is already on this store. The two are separate products that claim the
same file type on purpose, and giving them the same storefront name would make
the listing that arrives second look like an update to the first.

---

## Short description

Open the payload of a Slipcase container in whatever application already
handles that kind of file, and have your edits written back into the container
when you save.

---

## App features

Up to twenty, each within 200 characters. Four-space indented, one per line,
which is what the fleet's listing parser reads.

    Double-click a .slpc container and the document inside opens in the application you already use for that kind of file.
    Edit and save as you normally would. The edit is written back into the container, with nothing to export and re-import.
    No window to learn: an icon by the clock answers whether your work is where it should be, and its menu names the files being held open.
    If an application or the machine stops while a document is open, the edit is still on disk and goes back into its container next time.
    You are asked which version was meant only when the container changed as well, which is the one case where only you can answer.
    A container that arrived from the internet keeps that marking when its document is extracted, so the opening application stays cautious.
    A payload whose contents are a program while its name claims a document is refused before anything reaches the disk, and you are told why.
    Only ordinary document and image types are opened at all.
    The file types that may be opened are configurable and lockable through Group Policy, with an ADMX template available separately.
    Machine policy beats user policy, user policy beats a user's own settings, and a deny list wins over everything.
    Makes no network connection of any kind. Collects nothing and sends nothing anywhere.
    A supported command line, not a debugging aid: open a container, list what is open, and recover what was left behind.
    The container format is an open specification published at slipcaseformat.org, and a container is an ordinary ZIP archive.

---

## Description

A Slipcase container is a `.slpc` file: a single file holding a document
together with a record describing it. Slipcase Open is what happens when you
double-click one.

The document inside opens in whatever application you already use for that kind
of file — your PDF reader, your spreadsheet, your text editor. Edit it there and
save as you normally would, and the edit is written back into the container.
There is no separate step to remember and nothing to export and re-import.

**There is no window to learn.** Slipcase Open puts an icon by the clock while
it is holding a document open, and the icon's colour is the whole interface: it
answers one question continuously, which is whether your work is where it
should be. Its menu names the files it is holding and explains the colour when
there is something to explain.

**Your work is not lost if something goes wrong.** If an application or the
machine stops while a document is open, the edit is still on disk, and Slipcase
Open puts it back into its container the next time it runs. You are only asked
about it in the one case where asking is the honest thing to do — when the
container changed as well, so that only you can say which version was meant.

**It is careful about what it opens.** A container that arrived from the
internet keeps that marking when its document is extracted, so the application
that opens it treats the file with the caution it gives anything from outside.
A payload whose contents are a program while its name claims to be a document
is refused outright, before anything reaches the disk, and you are told why. And
only ordinary document and image types are opened at all.

**For administrators.** The set of file types that may be opened is
configurable and lockable through Group Policy, with an ADMX template available
separately. Machine policy takes precedence over user policy, which takes
precedence over a user's own settings, and a deny list wins over everything. The
application says so in its own interface when settings are being administered,
so that a refusal reads as a decision somebody made rather than as software
behaving unpredictably.

Slipcase Open makes no network connection of any kind. It collects nothing and
sends nothing anywhere.

A command line is included and is a supported interface, not a debugging aid:
`slipcase-open` will open a container, list what is open, and recover what was
left behind.

The Slipcase container format is an open specification, published at
slipcaseformat.org. A container is an ordinary ZIP archive, so nothing you put
in one is locked to this application.

## Claims in the description, and where each was established

Written down because the readiness review checks them one at a time, and because
a sentence is easier to check when somebody has said what would falsify it.

| Sentence | Established by |
| --- | --- |
| the payload opens in the registered application | measured through the real association, `PLAN.md` Phase 4 |
| edits are written back on save | same, and `writeback`'s tests |
| an icon by the clock, colour as the interface | `present::Mood`, verified by hand 2026-09-03 |
| work survives a crash and is put back | `recover::Course`, and §6.3 as amended |
| only a genuine conflict asks | `State::Diverged` is the only state that asks after an edit |
| the marking is carried onto the payload | measured: `ZoneId=3` and `HostUrl` on the extracted copy |
| a program under a document's name is refused | `content`'s check, verified by hand through a double-click |
| only ordinary document and image types | `policy::EVERY` built-in set |
| Group Policy, ADMX, precedence, deny list | `policy::registry`, `packaging/windows/policy/` |
| the interface says when settings are administered | `report_policy`, and the `policy` verb |
| no network connection of any kind | nothing in the dependency tree opens a socket; `check-imports.ps1` lists what the binary imports |

**One sentence is deliberately weaker than it could be.** The marking paragraph
says the application that opens the payload *treats it with the caution it gives
anything that came from outside*, rather than promising a particular warning. On
Windows the zone is copied verbatim and `ShellExecuteEx` is called with the check
left on, so the strong claim is very likely true — but that a warning is
*displayed* has never been watched by anybody, because nothing automated can
watch a modal dialog. It stays weak until somebody has seen it.

---

## Keywords

**Microsoft Store** (seven terms, which is the limit):

    slpc, slipcase, container, payload, open, edit, archive

`slpc` first: somebody who has been sent a file they cannot open searches for the
extension, and that is concept §2's audience arriving.

---

## Release notes

*What's new in this version* on the Microsoft Store, one version's text, kept
latest first under a `### x.y.z` heading. There is none for 0.1.5: it was the
macOS release and changed nothing a Windows user of this application would
notice, so there is nothing to tell them. The next Windows-visible change gets
an entry here, written the way the rest of this file is.

---

## Screenshots

**The product has no window**, so there is no equivalent of the sibling's
photograph of its own application. What ships, both 1920x984 and in
`packaging/windows/listing/`:

| | Shows |
| --- | --- |
| `01-payoff.png` | Explorer with three containers, each typed *Slipcase Container*, and the document from one of them open in Notepad beside it. The tray icon is **blue**. |
| `02-refusal.png` | The same desktop, with the refusal for a payload that is a program under a document's name. The same tray icon, in the same place, now **red**. |

**They are a pair, and the pairing is the argument.** One icon, one position,
two colours — which is concept §12's claim about the icon being the whole
interface, demonstrated rather than asserted. Neither image needs a caption to
make that point, and a person who looks at both learns the product's central
idea without reading a word of the description.

**The plan was three different shots and it was wrong.** It listed the context
menu first, then the tray menu, then the refusal. The first screenshot is what
appears in Store search results, so it has to carry the value — *your document
opens in the application you already use* — and a context menu does not say
that. The context menu and the tray menu are still worth taking; they are not
taken, because each needs a person to hold a menu open while a timer fires, and
neither is required to submit.

`packaging/windows/screenshot.ps1` takes the refusal on its own. What it will
not do is decide whether a screenshot is any good, and it does not choose the
container: which one appears is an editorial decision and belongs here rather
than in a script.

### The desktop these were taken on

Recorded because the next release should not have to rediscover it. Everything
below is a property of the machine, not of the product, and every one of them
was wrong on at least one attempt on 2026-09-06:

- **Light theme.** The refusal is a Win32 message box, which stays light whatever
  the theme is — so a dark shell puts a light dialog on a dark desktop and reads
  as a glitch.
- **A plain background.** The first attempts photographed a wallpaper carrying
  this project's own source code.
- **No desktop icons**, and **no clock**: a visible date stamps the listing with
  the day it was made and ages badly.
- **Only this application's icon in the notification area**, which puts it beside
  the clock instead of lost in a row of six.
- **Nothing else running.** Attempts photographed the terminal driving the shoot
  and a browser tab open on Partner Center.
- **A tidy navigation pane in Explorer**, which otherwise names other products'
  folders down the side of the listing.

The document is invented — no real person, organisation, matter or date — for
the same reason the sibling's demo container is.

**All three need a person at the desktop, including the one that is automated.**
The refusal can be opened and captured without anybody watching, but the Store's
floor is 1366x768 and the box is about 370x210 — so what gets photographed is
the box *in its desktop*, and everything else on that desktop goes in the
listing with it. Measured on 2026-09-06 by running it: the capture came out
correct in every technical respect and showed a terminal full of this project's
own source behind the dialog. Tidy the desktop first; nothing in the script can
do that for you.

**Two further things the script had to learn, both in
`packaging/windows/README.md`.** It launches through `wscript` rather than
calling the shell verb itself, because a process started from a console *is* a
command line to this product — concept §9's floor, where there is no tray icon
and no dialog to wait for — so capturing from a console photographs an
invocation nobody makes. And it matches the window by owning process rather than
by title, after a title match found a terminal that merely had the word
*Slipcase* in it.

**Taken against the packaged build**, which is what a person installs. Note that
they cannot be of the exact artefact uploaded, for the reason `RELEASE.md` gives
about rebuilds differing.

---

## The answers the form asks, which no build supplies

| | |
| --- | --- |
| Price | **Free.** |
| Category | Productivity |
| Support URL | `https://excelano.com/slipcase/` |
| Privacy policy URL | `https://excelano.com/legal/#slipcase` |
| Age rating | Every answer None. |
| Export compliance | **No encryption.** Slipcase Open makes no network request and implements no cryptography. It *reads* containers whose members may be encrypted and refuses those, which is not the same claim. |
| A container for the reviewer | `https://excelano.com/slipcase/quarterly-report.pdf.slpc` |

**The reviewer has nothing to open otherwise.** Slipcase Open without a container
does nothing visible but put up an icon, and the notes field takes no attachment,
which is why the sample container is served from the website. Its URL goes in
*Notes for certification* with an instruction to download it and double-click it.

**It is the sibling's container, and deliberately not a second one.** It is built
by `slipcase-desktop/packaging/demo-container.sh`, whose stated reason for
existing is that three demonstrations of the same format that do not look alike
is not a thing to discover after two listings are live. This repository has no
script of its own for it: a second script writing a second file to one URL is
exactly the failure that one was written to prevent. Its payload is a PDF, which
exercises the path this product is for — extract, hand to the registered reader,
write back on save. `RELEASE.md` carries this.

**`runFullTrust` is the only capability**, and the justification field has a
500-character limit which counts newlines and truncates silently at the paste:

> Slipcase Open is a full-trust Win32 desktop application packaged as MSIX. It
> needs this capability to run at all. It reads the container it is launched
> with, writes a working copy inside its own per-user application data, and asks
> the shell to open that copy with the application already registered for the
> file type. It makes no network connection, requires no broad filesystem
> access, and uses no device.

**Both URLs are submission blockers**, and the check is against the served HTML
rather than against this file. `/slipcase/` also has to gain a link and lose any
promise of a listing that is *coming* once this one is live — publication is not
finished when the Store says published.
