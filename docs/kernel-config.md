# Kernel Configuration

anyOS keeps kernel policy configuration in `confd`, not in ad-hoc files under
`/System/etc`. The kernel itself only exposes mechanisms such as syscalls and
memory-management primitives. Early userspace owns policy decisions.

## Ownership

- **Owner:** `/System/init`
- **Registry scope:** `system`
- **Namespace:** `kernel`
- **Backend:** `confd`

`/System/init` waits until `confd` is ready, registers the `kernel` manifest,
reads the effective values, and then applies the requested policy through normal
kernel syscalls.

There is intentionally no `/System/etc/swap.conf`. A future read-only `/etc`
projection may expose selected `confd` values for compatibility, but `confd`
remains authoritative.

## Swap

Swap configuration lives under `system/kernel/swap`.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `kernel/swap/enabled` | bool | `true` | Enable the configured swap file during early init. |
| `kernel/swap/path` | string | `/swap` | Absolute path to the swap backing file. |
| `kernel/swap/size_mb` | int | `256` | Desired fixed swap file size in MiB. Clamped by init to 8..2047 MiB. |

Boot behavior:

1. The kernel starts `amid` and `confd` before the desktop/text-mode stack.
2. The compositor starts `/System/init`.
3. `init` waits for `confd` readiness.
4. `init` registers the `kernel` manifest and ensures defaults exist.
5. If `kernel/swap/enabled` is true, `init` creates or opens
   `kernel/swap/path`, resizes it to `kernel/swap/size_mb`, then calls
   `SYS_SWAPON`.
6. If `kernel/swap/enabled` is false, `init` calls `SYS_SWAPOFF` for the
   configured path and continues boot.
7. The normal `svc start-all` service wave starts after this early kernel
   policy pass.

The swap file is fixed-size. It is not dynamically grown by the kernel. This is
intentional: the kernel's swap subsystem can treat enabled swap areas as stable
backing stores, while policy and provisioning stay in userspace.

## Kernel Boundary

The kernel provides:

- `SYS_SWAPON(path, flags)` to attach an existing regular file as swap backing
  storage.
- `SYS_SWAPOFF(path)` to detach a swap file when no slots are in use.
- Swap statistics through `SYS_SYSINFO` and Linux-compatible `/proc/meminfo`.
- Swap slot allocation and page I/O internally.

The kernel does not:

- read `confd`;
- read `/System/etc` files for swap policy;
- choose a default swap path or size;
- create, grow, or shrink swap files on its own.

That separation keeps the kernel small and deterministic while still allowing
machine policy to change without rebuilding the kernel.
