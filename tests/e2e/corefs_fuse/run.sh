#!/usr/bin/env bash
# CoreFS FUSE end-to-end harness — PLACEHOLDER.
#
# This script is intentionally a no-op. The real implementation will
# orchestrate a QEMU boot, attach a CoreFS disk image, and scan the
# serial output for test markers. See README.md for the full plan.

set -euo pipefail

echo "QEMU harness placeholder — not implemented yet."
echo
echo "Planned invocation (see README.md for details):"
echo "  qemu-system-x86_64 \\"
echo "      -machine q35 -cpu max -m 512 -nographic -serial stdio \\"
echo "      -drive file=<kernel-image>,format=raw,if=virtio \\"
echo "      -drive file=<corefs-image>,format=raw,if=virtio \\"
echo
echo "Expected steps inside the booted AnyOS:"
echo "  1. init brings up corefsd, which mounts /dev/vdb at /mnt/corefs."
echo "  2. e2e-driver (not yet written) runs one scenario from ./scenarios."
echo "  3. Driver prints __E2E_PASS__ <name> or __E2E_FAIL__ <name> <why>."
echo "  4. Host-side scanner (not yet written) interprets markers and"
echo "     sets this script's exit code accordingly."
echo
echo "Blocking work items (see README.md 'Next steps'):"
echo "  * serial test-reporter convention in kernel/src/serial"
echo "  * AnyOS-user binary system/utilities/e2e-driver"
echo "  * actual QEMU-invocation + marker scanner"

exit 0
