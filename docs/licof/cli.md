# licof CLI and Rootfs

`/System/bin/licof` is the user-facing control tool for the Linux
Compatibility Framework.

## Commands

```sh
licof status
licof run <linux-elf64> [args...]
licof rootfs create [name]
licof rootfs list
licof pkg install <file.deb>
licof apt install <package> [package...]
```

## `licof status`

Prints the current configuration:

- ABI tier.
- licof root directory.
- default rootfs.
- configured Debian package index URL.
- config source.
- supported package archive payloads.

Example:

```sh
licof status
```

## `licof run`

Starts a Linux ELF64 binary through `SYS_LICOF_SPAWN`.

```sh
licof run /System/var/licof/rootfs/debian/usr/bin/passwd root
```

Before spawning, the CLI diagnoses the ELF header:

- validates ELF64.
- warns for `ET_DYN` / PIE.
- prints `PT_INTERP` and resolved interpreter path.
- reports missing interpreter paths.

The spawned Linux process inherits Terminal stdin/stdout pipes. `licof` waits
for the child and prints a non-zero exit status.

## `licof rootfs create`

Creates a named rootfs, writes minimal apt configuration, bootstraps the
configured seed packages and then tries to start `passwd root` if the bootstrap
succeeded and the caller has an interactive terminal.

```sh
licof rootfs create debian
```

For name `default`, the path is `paths/default_rootfs`. For any other name, the
path is:

```text
<paths/rootfs_dir>/<name>
```

With defaults:

```text
default -> /System/var/licof/rootfs/default
debian  -> /System/var/licof/rootfs/debian
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
etc/apt/apt.conf.d/99licof
```

`99licof` disables valid-until checks because the default archive target is an
old Debian suite on `archive.debian.org`.

## Bootstrap Seed

The default bootstrap seed is:

```text
base-files
base-passwd
libc6
libgcc1
libstdc++6
zlib1g
libapt-pkg4.12
apt
passwd
```

The seed is configurable through:

```text
services/licof/bootstrap/packages_csv
```

## Rootfs Runtime Repair

After package extraction, `licof` repairs runtime links that are critical for
dynamic glibc binaries:

- ensures `/lib64` exists.
- materializes `/lib64/ld-linux-x86-64.so.2` from known loader candidates.
- materializes common SONAME links in:
  - `/lib/x86_64-linux-gnu`
  - `/usr/lib/x86_64-linux-gnu`

This is currently necessary because package symlinks and hardlinks are handled
best-effort and some anyOS filesystems or extract paths do not preserve Linux
link semantics perfectly.

## `licof rootfs list`

Currently prints the configured default rootfs:

```sh
licof rootfs list
```

## `licof pkg install`

Installs a local `.deb` into the default rootfs:

```sh
licof pkg install /path/to/package.deb
```

Behavior:

- ensures the default rootfs exists.
- opens the Debian `ar` container through `libzip_client`.
- finds `data.tar.gz` or `data.tar.xz`.
- extracts files into the rootfs.
- records installed package metadata when package info is available.

Maintainer scripts are not executed.

## `licof apt install`

Downloads and installs packages from the configured Debian archive:

```sh
licof apt install mawk
licof apt install apt passwd
```

Behavior:

- ensures the default rootfs exists.
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
http://archive.debian.org/debian/dists/wheezy/main/binary-amd64/Packages.gz
```

Cache paths:

```text
/System/var/licof/cache/debian-wheezy-amd64-Packages.gz
/System/var/licof/cache/debian-wheezy-amd64-Packages
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
services/licof/apt/download_attempts
```

Download errors are only reported after all retries fail.

## Config Schema

The CLI registers the `services/licof` config manifest with `confd`.

Defaults:

| Key | Default |
| --- | --- |
| `paths/root` | `/System/var/licof` |
| `paths/rootfs_dir` | `/System/var/licof/rootfs` |
| `paths/default_rootfs` | `/System/var/licof/rootfs/default` |
| `paths/cache` | `/System/var/licof/cache` |
| `paths/db` | `/System/var/licof/db` |
| `paths/installed_db` | `/System/var/licof/db/installed` |
| `apt/base_url` | `http://archive.debian.org/debian` |
| `apt/suite` | `wheezy` |
| `apt/component` | `main` |
| `apt/arch` | `amd64` |
| `apt/index_required_packages_csv` | `apt,libc6,libgcc1,libstdc++6,multiarch-support,passwd,zlib1g` |
| `apt/download_attempts` | `4` |
| `bootstrap/packages_csv` | `base-files,base-passwd,libc6,libgcc1,libstdc++6,zlib1g,libapt-pkg4.12,apt,passwd` |
| `tools/wget` | `/System/bin/wget` |

All path values are normalized by trimming trailing slashes.

## Installed Package Database

Installed package markers are stored under:

```text
<paths/installed_db>/<rootfs-key>/<package>
```

The rootfs key is a filesystem-safe encoding of the rootfs path. This database
is intentionally minimal: it prevents repeated installs and records enough
state for bootstrap progress. It is not a full dpkg database.

## Current Operational Notes

- `licof rootfs create` is expected to be run from Terminal when root password
  setup is desired.
- A successful package install does not imply maintainer scripts have run.
- Some packages require `/proc`, `/sys`, PAM, NSS, terminal or signal behavior
  that is still incomplete.
- Dynamic loader failures often point to kernel ABI gaps rather than package
  extraction problems.
- If a binary exits with status `127`, inspect loader output and serial logs.
