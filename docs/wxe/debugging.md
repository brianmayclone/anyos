# WXE Debugging

WXE should be diagnosable from the first commit. Missing loader features and
missing DLL exports are normal during bring-up; silent process death is not.

## Serial Log Prefixes

Use stable prefixes so logs can be filtered quickly:

```text
wxe loader:
wxe pe:
wxe import:
wxe dll:
wxe nt:
wxe path:
wxe handle:
wxe console:
```

Examples:

```text
wxe pe: C:\Tools\hello.exe machine=amd64 subsystem=console image_base=0x140000000
wxe import: loaded kernel32.dll for C:\Tools\hello.exe
wxe import: missing export kernelbase.dll!GetFileInformationByHandleEx
wxe nt: unsupported service nr=0x003a rip=0x00007ffb00001234
wxe path: C:\Users\Default\test.txt -> /System/var/wxe/drive_c/Users/Default/test.txt
```

## `wxe inspect`

`wxe inspect <path>` should run entirely in native anyOS userland and print:

- PE kind: `PE32+`, machine, subsystem
- entry point and image base
- section table with flags
- data-directory presence:
  - imports
  - relocations
  - TLS
  - exception directory
  - resources
- import DLLs and imported symbols
- whether each DLL/export exists in the installed WXE DLL set
- whether the binary is launchable in the current WXE profile

This prevents kernel iteration for obvious missing-DLL failures.

## Kernel Trace Switches

The Windows dispatcher should have a trace flag similar to LXE tracing:

```text
wxe trace on
wxe trace off
```

Trace output should include:

- service number and symbolic NT name
- current TID
- return NTSTATUS
- key object handles
- translated paths

Noisy probes should be suppressible once they are known-safe.

## Crash Reports

WXE process crashes should include:

- ABI personality: `WindowsX86_64`
- image path and command line
- last WXE NT syscall/service name
- last missing import, if any
- RIP/RSP/RFLAGS
- module containing RIP if the loader list can resolve it

This should use existing crash-info paths without changing native or LXE crash
semantics.

## Smoke Tests

Initial smoke binaries:

| Test | Expected behavior |
| --- | --- |
| `hello.exe` | writes one line and exits 0 |
| `args.exe one two` | prints Windows command line and parsed argv |
| `env.exe` | prints selected environment variables |
| `fileio.exe` | creates, writes, reads and deletes `C:\Users\Default\wxe-test.txt` |
| `cwd.exe` | verifies `GetCurrentDirectoryW` and drive-relative paths |
| `unicode.exe` | opens a UTF-16 path if filesystem support allows it |
| `sleep.exe` | calls `Sleep` and exits |

Each smoke should be runnable through:

```sh
wxe run C:\Tests\hello.exe
wxe shell
```

## Regression Guardrails

Every WXE change that touches shared kernel paths should be checked against:

- native anyOS process spawn
- `lxe status`
- `lxe run` on an existing Linux smoke binary if available
- scheduler fork/exec tests when thread metadata changes
- VMA/mmap tests when Windows memory services change

The WXE code should add branches at ABI selection points, not modify LXE
handlers to carry Windows behavior.
