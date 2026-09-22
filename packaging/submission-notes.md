# Submission notes

What a store submission needs from a person and no file supplies: the notes an
App Review or certification reader is handed, the answers a form asks that no
build can give, and the reasoning behind the screenshots.

The listing text itself is not here. It is `store-listing.toml` beside this,
which `ship` checks before the tag and pushes to both stores on every release,
and what a release tells them changed is `release-notes.toml`. A field edited
in this file would reach nobody.

## Claims in the description, and where each was established

Written down because the readiness review checks them one at a time, and because
a sentence is easier to check when somebody has said what would falsify it.

| Sentence | Established by |
| --- | --- |
| the content file opens in the registered application | measured through the real association, `PLAN.md` Phase 4 |
| edits are written back on save | same, and `writeback`'s tests |
| an icon by the clock, colour as the interface | `present::Mood`, verified by hand 2026-09-03 |
| work survives a crash and is put back | `recover::Course`, and §6.3 as amended |
| only a genuine conflict asks | `State::Diverged` is the only state that asks after an edit |
| the marking is carried onto the content file | measured: `ZoneId=3` and `HostUrl` on the extracted copy |
| a program under a document's name is refused | `content`'s check, verified by hand through a double-click |
| only ordinary document and image types | `policy::EVERY` built-in set |
| Group Policy, ADMX, precedence, deny list | `policy::registry`, `packaging/windows/policy/` |
| the interface says when settings are administered | `report_policy`, and the `policy` verb |
| no network connection of any kind | nothing in the dependency tree opens a socket; `check-imports.ps1` lists what the binary imports |

**One sentence is deliberately weaker than it could be.** The marking paragraph
says the application that opens the content file *treats it with the caution it gives
anything that came from outside*, rather than promising a particular warning. On
Windows the zone is copied verbatim and `ShellExecuteEx` is called with the check
left on, so the strong claim is very likely true — but that a warning is
*displayed* has never been watched by anybody, because nothing automated can
watch a modal dialog. It stays weak until somebody has seen it.

## Screenshots

**The product has no window**, so there is no equivalent of the sibling's
photograph of its own application. What ships, both 1920x984 and in
`packaging/windows/listing/`:

| | Shows |
| --- | --- |
| `01-payoff.png` | Explorer with three containers, each typed *Slipcase Container*, and the document from one of them open in Notepad beside it. The tray icon is **blue**. |
| `02-refusal.png` | The same desktop, with the refusal for a content file that is a program under a document's name. The same tray icon, in the same place, now **red**. |

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
they cannot be of the exact artefact uploaded, because a rebuild differs.

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
exactly the failure that one was written to prevent. Its content file is a PDF, which
exercises the path this product is for — extract, hand to the registered reader,
write back on save.

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
