# lxe CLI and Linux Base

`/System/bin/lxe` is the user-facing control tool for the Linux Experience
Extension. lxe intentionally manages one active Linux base, not a collection
of named root filesystems.

## Commands

```sh
lxe status
lxe init
lxe repair
lxe run <linux-elf64> [args...]
lxe pkg install <file.deb>
lxe apt install <package> [package...]
```

## `lxe status`

Prints the current configuration:

- ABI tier.
- lxe root directory.
- active Linux base path.
- configured Debian package index URL.
- config source.
- supported package archive payloads.

## `lxe init`

Creates the configured Linux base, writes minimal apt configuration,
bootstraps the configured seed packages and then tries to start `passwd root`
when the caller has an interactive terminal.

```sh
lxe init
```

Default Linux base:

```text
/System/var/lxe/rootfs
```

Created directories include:

```text
bin/
lib/
lib64/
usr/
usr/bin/
etc/
etc/apt/
etc/apt/apt.conf.d/
```

Generated apt files:

```text
etc/apt/sources.list
etc/apt/apt.conf.d/99lxe
```

`99lxe` disables valid-until checks for compatibility with archived or
mirrored Debian package indexes.

After package installation, `lxe init` verifies that the Linux base contains
an executable shell candidate (`/bin/bash`, `/bin/dash` or `/bin/sh`) and a
`passwd` binary before it reports bootstrap success. Missing paths are printed
with path probes so stale package markers cannot silently turn into a false
`bootstrap complete`.

## `lxe repair`

Recreates the base directory layout, repairs runtime links and syncs the
filesystem.

```sh
lxe repair
```

Repair currently:

- ensures `/lib64` exists.
- recreates `/lib64/ld-linux-x86-64.so.2` as a symlink to a known loader
  candidate when it is missing.
- recreates missing common SONAME aliases as symlinks in:
  - `/lib/x86_64-linux-gnu`
  - `/usr/lib/x86_64-linux-gnu`

Package symlinks are installed as symlinks when the filesystem supports them.
If symlink creation or verification fails during package extraction, the
package install fails. Runtime libraries must keep their package metadata and
SONAME symlinks intact; lxe no longer copies a library over a broken symlink
to "repair" it.

## `lxe run`

Starts a Linux ELF64 binary through `SYS_LXE_SPAWN`.

```sh
lxe run /usr/bin/passwd root
lxe run /System/var/lxe/rootfs/usr/bin/passwd root
```

Linux-style absolute paths such as `/usr/bin/passwd` are resolved inside the
active Linux base. anyOS paths under `/System`, `/Applications` and `/Users`
are passed through unchanged.

Before spawning, the CLI diagnoses the ELF header:

- validates ELF64.
- warns for `ET_DYN` / PIE.
- prints `PT_INTERP` and resolved interpreter path.
- reports missing interpreter paths.

The spawned Linux process inherits Terminal stdin/stdout pipes. `lxe` waits
for the child and prints a non-zero exit status.

## Bootstrap Seed

The default bootstrap seed is:

```text
base-files
base-passwd
libc6
libgcc-s1
libstdc++6
zlib1g
apt
debian-archive-keyring
dash
bash
coreutils
libpam-runtime
login
passwd
libcom-err2
mc
procps
htop
gcc
make
```

The seed is configurable through:

```text
services/lxe/bootstrap/packages_csv
```

## `lxe pkg install`

Installs a local `.deb` into the active Linux base:

```sh
lxe pkg install /path/to/package.deb
```

Behavior:

- ensures the Linux base exists.
- opens the Debian `ar` container through `libzip_client`.
- finds `data.tar.gz` or `data.tar.xz`.
- extracts files into the Linux base.
- installs package symlinks as symlinks.
- materializes package hardlinks by copying the already extracted target.
- records installed package metadata when package info is available.

Maintainer scripts are not executed.

Because maintainer scripts are not executed, `lxe init` seeds a tiny account
database when the files are missing. The seed includes `/etc/passwd`,
`/etc/group`, `/etc/shadow`, `/etc/gshadow`, `/etc/nsswitch.conf`, and `/root`.
It also creates minimal PAM `common-*` include files in `/etc/pam.d` so tools
such as `passwd` can reach the local `pam_unix` path, plus `/etc/pam.d/other`
as the PAM fallback service file. The runtime repair also creates a conservative
UTC timezone setup: `/etc/timezone`, `/etc/localtime`, and
`/usr/share/zoneinfo/Etc/UTC`. Existing files are preserved so `passwd`, later
package installs, or manual edits are not overwritten.

## `lxe apt install`

Downloads and installs packages from the configured Debian archive:

```sh
lxe apt install mawk
lxe apt install apt passwd
```

Behavior:

- ensures the Linux base exists.
- ensures the package index exists and is valid.
- parses Debian `Packages` paragraphs.
- supports exact package matches and basic `Provides`.
- resolves simple `Pre-Depends` and `Depends` recursively.
- downloads `.deb` files into the cache.
- verifies package size and MD5 when available.
- extracts package data.

Dependency alternatives are tried in order. Some virtual packages are mapped to
preferred concrete packages; currently `awk` resolves to `mawk`.

## Package Index

Default index URL:

```text
http://deb.debian.org/debian/dists/bookworm/main/binary-amd64/Packages.gz
```

Cache paths:

```text
/System/var/lxe/cache/bookworm/debian-bookworm-amd64-Packages.gz
/System/var/lxe/cache/bookworm/debian-bookworm-amd64-Packages
```

The index is accepted only when:

- the downloaded file is not empty.
- gzip decompression succeeds, or the server returned a plain `Packages` file.
- the result looks like a Debian `Packages` index.
- required bootstrap entries are present.

## Downloads

Download order:

1. use `libhttp_client` when available.
2. fall back to configured `wget`.

Retries are controlled by:

```text
services/lxe/apt/download_attempts
```

Download errors are only reported after all retries fail.

## Config Schema

The CLI registers the `services/lxe` config manifest with `confd`.

Defaults:

| Key | Default |
| --- | --- |
| `paths/root` | `/System/var/lxe` |
| `paths/rootfs` | `/System/var/lxe/rootfs` |
| `paths/cache` | `/System/var/lxe/cache/bookworm` |
| `paths/db` | `/System/var/lxe/db/bookworm` |
| `paths/installed_db` | `/System/var/lxe/db/bookworm/installed` |
| `apt/base_url` | `http://deb.debian.org/debian` |
| `apt/suite` | `bookworm` |
| `apt/component` | `main` |
| `apt/arch` | `amd64` |
| `apt/index_required_packages_csv` | `apt,base-files,base-passwd,bash,coreutils,dash,debian-archive-keyring,libc6,libgcc-s1,libpam-runtime,libstdc++6,login,passwd,zlib1g` |
| `apt/download_attempts` | `4` |
| `bootstrap/packages_csv` | `base-files,base-passwd,libc6,libgcc-s1,libstdc++6,zlib1g,apt,debian-archive-keyring,dash,bash,coreutils,libpam-runtime,login,passwd,libcom-err2,mc,procps,htop,gcc,make` |
| `tools/wget` | `/System/bin/wget` |

All path values are normalized by trimming trailing slashes.

## Installed Package Database

Installed package markers are stored under:

```text
<paths/installed_db>/<rootfs-key>/<package>
```

The rootfs key is a filesystem-safe encoding of the active Linux base path.
Each marker records the package version, source filename, file count and the
extracted payload paths. A marker is accepted only when at least one payload
path is recorded and all recorded files or links still exist in the active
Linux base. The check does not follow the final symlink, because Debian
packages can contain Linux-absolute links that must be resolved relative to the
Linux base at runtime. Invalid or legacy markers are ignored and the package is
installed again. This is still not a full dpkg database; it is a bootstrap
integrity cache.

Bootstrap progress is also written to:

```text
<paths/db>/bootstrap-state
```

The state file is recomputed during `lxe init` and records `Status`,
`Installed`, `Missing` and `Failed` package lines for the configured bootstrap
seed. Re-running `lxe init` uses the package markers above, ignores stale
markers, downloads the missing seed packages and updates this state file after
each package.

## Current Operational Notes

- `lxe init` is expected to be run from Terminal when root password setup is
  desired.
- A successful package install does not imply maintainer scripts have run.
- Some packages require `/proc`, `/sys`, PAM, NSS, terminal or signal behavior
  that is still incomplete.
- Dynamic loader failures often point to kernel ABI gaps rather than package
  extraction problems.
- If a binary exits with status `127`, inspect loader output and serial logs.
