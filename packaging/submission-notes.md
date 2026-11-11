# Submission notes

What a store submission needs from a person and no file supplies. The listing
text is `store-listing.toml`, including the notes for certification as
`microsoft-review-notes`, and what a release changed is `release-notes.toml`;
the release apps push both. A field edited here reaches nobody.

## Capability justification

Partner Center asks once, when `runFullTrust` is first declared, and keeps the
answer against the product. 500 characters, newlines counted.

> Slipcase Open is a full-trust Win32 desktop application packaged as MSIX. It
> needs this capability to run at all. It reads the container it is launched
> with, writes a working copy inside its own per-user application data, and asks
> the shell to open that copy with the application already registered for the
> file type. It makes no network connection, requires no broad filesystem
> access, and uses no device.

## The answers a form asks

    Category                  Productivity
    Copyright and trademark   Excelano LLC
    Additional license terms  blank; the application is MIT and the
                              repository carries the license
    Privacy policy            https://excelano.com/legal/#slipcase
    Age rating                no objectionable content of any kind
    Export compliance         no encryption; it makes no network request
                              and implements no cryptography
    Price                     free

The reviewer needs a container to open, and the notes field takes no
attachment, so `microsoft-review-notes` links the sibling's demonstration
container, `https://excelano.com/slipcase/quarterly-report.pdf.slpc`, built by
`slipcase-desktop/packaging/demo-container.sh`. This repository has no script
of its own for it, so one URL has one source.

## Screenshots

The product has no window, so the frames are a pair in
`packaging/windows/listing/`: a container's document open in its own
application with the tray icon blue, and the refusal of a program under a
document's name with the same icon red. `packaging/windows/screenshot.ps1`
captures the refusal, launched through `wscript` so the product sees a
double-click rather than a command line, against the installed package.
Everything on the desktop goes into the frame, so it is set up first: the light
theme, because the refusal is a message box that stays light; a plain
background; no desktop icons and no clock; only this application's icon in the
notification area; nothing else running; and a tidy navigation pane in Explorer.
The document in the demo container is invented and names no real person or
organization.
