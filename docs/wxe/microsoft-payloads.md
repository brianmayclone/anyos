# WXE Microsoft Payload Policy

WXE may support Microsoft-provided console tools later, but it must do so with
an explicit license boundary. `wxe init` must never silently download or install
Microsoft Windows binaries.

## Rules

- anyOS does not bundle Microsoft Windows binaries.
- `wxe init` creates only WXE-owned files, directories, manifests and generated
  compatibility DLLs.
- Microsoft payload import must be opt-in and visible to the user.
- Before importing or downloading Microsoft payloads, WXE must show the
  applicable Microsoft license terms or an explicit link/source summary and
  require acceptance.
- Accepted terms must be recorded in WXE state with source, version, timestamp
  and package hash.
- WXE must prefer user-provided Windows installation media for Windows OS
  components such as `cmd.exe`.
- WXE may support official Microsoft redistributable channels only where the
  package terms permit direct installation by the user.
- WXE must not mirror, redistribute, repackage or cache Microsoft binaries in
  anyOS images.

## Command Processor

The WXE command processor target is:

```text
C:\Windows\System32\cmd.exe
```

This follows the Windows `COMSPEC` convention. WXE should provide its own
compatible `cmd.exe` eventually, because importing Microsoft's `cmd.exe` from a
licensed Windows installation is user-specific and cannot be assumed.

`command.com` is intentionally out of scope for the x86_64 WXE layer. It is a
DOS/Windows 9x/NTVDM-era command interpreter, not the native 64-bit NT console
shell.

## Built-Ins vs Executables

Not every Windows command is a standalone executable:

- `cmd.exe` is the command shell.
- `copy`, `dir`, `cd`, `set`, `echo`, `cls`, `type` and similar commands are
  command-shell built-ins.
- Tools such as modern `edit` may be separate Microsoft packages or Windows
  components depending on Windows version and channel.

This means WXE's first reliable path is a WXE-owned `cmd.exe` with built-ins,
plus optional import of standalone Microsoft tools only after license consent.

## Planned CLI

```text
wxe import-ms windows-media <path>
wxe import-ms official-package <id>
wxe import-ms sysinternals <tool>
```

The first implementation should only inspect sources and print the license gate.
Actual import should be added after:

1. source verification
2. license acceptance UI
3. hash recording
4. explicit user confirmation
5. no image-level redistribution

## Current Official References

- Microsoft documents the Windows command set and notes that Windows has the
  Command shell and PowerShell.
- Microsoft's `Edit` command-line editor is a modern Windows 11 tool and can be
  installed via Microsoft's package channels before it ships broadly.
- Sysinternals tools are downloadable from Microsoft, but the published FAQ says
  third-party redistribution is not offered.
- Windows license terms are separate from WXE and must be accepted by the user
  for user-provided Windows components.
