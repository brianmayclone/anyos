# CoreFS FUSE End-to-End Harness (QEMU)

**Status: PLACEHOLDER.** Nothing here runs yet. This directory records
the plan and the moving parts required to exercise the real CoreFS
FUSE stack (kernel `/dev/fuse` + userspace `corefsd` inside AnyOS)
under QEMU.

Host-level unit tests for `corefsd` already live next to the binary
(`bin/corefsd/src/handler.rs`). Host-level integration tests for the
kernel's CoreFS driver live in `kernel/src/fs/corefs/`. What is
missing — and what this harness is meant to eventually provide — is
an end-to-end check that exercises the protocol over a real
`/dev/fuse` channel while both halves run on their native targets
(`x86_64-anyos` for the kernel, `x86_64-anyos-user` for `corefsd`).

## Why this is hard

1. **AnyOS-user target.** `corefsd` is built with
   `-Z build-std=core,alloc` against `x86_64-anyos-user.json`. It
   cannot be run on the host Linux kernel — it only links against
   the AnyOS syscall ABI.
2. **Kernel boot.** The CoreFS driver only exists inside the AnyOS
   kernel (`kernel/src/fs/corefs/`). Exercising it requires booting
   the kernel; booting the kernel requires building the full image.
3. **Serial-console protocol.** Communicating test results out of
   QEMU relies on the existing AnyOS serial panic/log channel; there
   is no dedicated test harness yet.

## Required building blocks (to be built)

| Component                    | Status       | Notes |
|------------------------------|--------------|-------|
| Bootable AnyOS image w/ corefsd | partial   | `scripts/make_image.sh` produces a `corefs`-backed image; needs a slimmed test profile |
| `mkfs.corefs` host tool      | done         | `corefs-tools` crate in the `corefs` repo |
| `corefsd` autostart service  | partial      | init wiring exists for `/System`; see `system/init` |
| Serial-only test reporter    | TODO         | convention: print `__E2E_PASS__ <name>` / `__E2E_FAIL__ <name> <reason>` |
| QEMU run harness             | TODO         | this directory (`run.sh`) |
| Scenario drivers             | TODO         | `scenarios/*.txt` describe drehbuch; actual driver would be a small AnyOS binary |

## How a real run would look

```bash
# 1. Build the AnyOS image with the e2e test profile enabled.
cd /daten1/development/brian/anyos
./scripts/make_image.sh --profile e2e --out target/e2e.img

# 2. Generate a freshly-formatted CoreFS volume image to attach as a
#    second disk.
cd /daten1/development/brian/corefs
cargo run --bin mkfs.corefs -- --size 64M ./target/e2e-corefs.img

# 3. Boot QEMU with both disks, serial redirected to stdout.
cd /daten1/development/brian/anyos/tests/e2e/corefs_fuse
./run.sh \
    --kernel-image ../../../target/e2e.img \
    --corefs-image ../../../../corefs/target/e2e-corefs.img \
    --scenario scenarios/write_read.txt
```

`run.sh` would then:

1. Launch `qemu-system-x86_64` in headless mode with serial output
   piped into a line scanner.
2. Boot AnyOS → init → `corefsd` mounts the second disk at
   `/mnt/corefs`.
3. Run the scenario driver (a tiny AnyOS binary) against the mount.
4. Scanner watches for `__E2E_PASS__` / `__E2E_FAIL__` markers and
   propagates the exit status.
5. QEMU is terminated via the monitor socket.

## Scenarios

Each file in `scenarios/` is a text "drehbuch" describing what a
real scenario driver would do. These are documentation only — they
do **not** execute.

- `write_read.txt` — format, mount, write a file, unmount, remount,
  verify content survives.

## Next steps (ordered)

1. Land the serial test-reporter convention in `kernel/src/serial`
   (purely a macro + flush contract; no kernel behavior change).
2. Add a minimal AnyOS `e2e-driver` binary under
   `system/utilities/e2e-driver/` that speaks one scenario file.
3. Wire `run.sh` to `qemu-system-x86_64` with `-serial stdio -nographic`.
4. Add one CI job that runs `./run.sh scenarios/write_read.txt` and
   fails on missing `__E2E_PASS__`.

None of those steps are in scope for the current placeholder commit.
