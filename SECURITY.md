# Security Policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately, using the **Report a vulnerability** button on the Security tab of this repository. If that button is not there, or you would rather not go through GitHub, email david.anderson@excelano.com instead. I aim to respond within seven days.

Please do not open public issues for security problems.

## Supported versions

The latest release receives security fixes. Older versions are not supported. A fix ships in a new release; update the way you installed.

## What Slipcase Open can access

Slipcase Open runs locally and makes no network connection of any kind: no server, no account, no analytics, no telemetry and no crash reporting. It can only reach files your operating-system user already can.

It reads the container it is asked to open, validates it through the `slpc` library, and extracts the content file. Before anything is extracted it checks the content file's extension against policy, and refuses a content file whose bytes are an executable image or a shebang script under a name claiming otherwise. It then asks the platform to open the extracted copy with whatever application is registered for that extension: `xdg-open` on Linux, `open` on macOS, and `ShellExecuteEx` on Windows with its zone check left on. It runs no program of its own choosing and carries no table of file types.

**It writes the container back.** While a session is open, every save of the content file is repacked into a temporary container beside the original, validated, and renamed over it. The flyleaf member and any members the tool does not recognize are preserved byte for byte. On Windows and macOS the container's trust-zone mark (Mark of the Web, `com.apple.quarantine`) is copied onto the extracted content file and carried onto each replacement, and a write-back that cannot carry it fails rather than replacing a marked container with an unmarked one. Linux has no such mark to carry.

It reads its settings from `/etc/slipcase/open.toml` and `$XDG_CONFIG_HOME/slipcase-open/policy.toml` on Linux and macOS, and from `SOFTWARE\Policies\Excelano\Slipcase\Open` under `HKLM` and `HKCU` and the user's own `HKCU\SOFTWARE\Excelano\Slipcase\Open` on Windows. It reports and asks through the desktop's notification service (D-Bus `org.freedesktop.Notifications` on Linux, toasts and a tray icon on Windows) or, where there is none, on the terminal.

A second invocation hands its container to the running instance through a local endpoint restricted to your user: a Unix socket in a directory only you can enter, under `$XDG_RUNTIME_DIR` where the platform sets it, or a named pipe whose ACL names your account on Windows.

**The extension allowlist is not a security boundary.** A container is a plain ZIP and anyone can extract its content file with standard tools. The allowlist guards the convenient path against mistakes and social engineering; the boundary is application control such as AppLocker or WDAC.

## What Slipcase Open stores

Each open session is a directory of its own, mode 0700, holding a small TOML record (the container's path, the content file's name, the session start, the write-back count, and the container's CRC at the last moment the two agreed) and a `content/` directory holding the extracted content file. The directories live under `$XDG_STATE_HOME/slipcase-open/sessions` on Linux, `~/Library/Application Support/slipcase-open/sessions` on macOS, and `%LOCALAPPDATA%\slipcase-open\sessions` on Windows, or the package's `LocalCache` when installed as an MSIX. `slipcase-open policy` prints the path this machine resolves.

A session directory is removed when its session closes cleanly. One left by a crash, or by an application still working in it at close, stays until the next launch puts the edit back or asks about it. Until then a copy of the content file sits in your state directory, which backup and sync software may capture; the packaged Windows location is excluded from roaming and device backup.

The front-door socket is runtime state and goes with the instance or at logout. Nothing else persists.
