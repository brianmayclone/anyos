#!/usr/bin/env bash
set -euo pipefail

# Build anyOS through ccargo/anyrc while reusing the normal CMake/Ninja
# bootloader, sysroot, app packaging, and image machinery from build.sh.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
BUILD_DIR="${ANYRC_BUILD_DIR:-${PROJECT_DIR}/build-anyrc}"
SHIM_DIR="${BUILD_DIR}/anyrc-toolchain"
CARGO_SHIM="${SHIM_DIR}/cargo"
REAL_CARGO="${CARGO:-cargo}"
BOOT_TIMEOUT="${ANYRC_BOOT_TIMEOUT:-60s}"
BOOT_MARKER="${ANYRC_BOOT_MARKER:-[OK] Syscall interface initialized}"

BUILD_IMAGE=1
VERIFY_BOOT=0
CLEAN=0
DEBUG_VERBOSE=0
DEBUG_SURF=0
NO_CROSS=1
RESET=0
ANYOS_ARCH="x86_64"
ANYOS_VERSION="$(tr -d '[:space:]' < "${PROJECT_DIR}/VERSION")"
SYSTEM_FS="exfat"
SYSTEM_FS_SIZE_MIB=""
CMAKE_PASSTHROUGH=()

usage() {
  cat <<'EOF'
Usage:
  scripts/build_with_anyrc.sh [options]

Builds the x86_64 anyOS workspace with ccargo/anyrc, then creates the normal
bootable BIOS image. QEMU is only started when --boot is passed.

Options:
  --kernel-only          Build only the kernel target, skip programs/image.
  --no-boot              Build image and skip QEMU (default).
  --boot                 Launch QEMU after building the image.
  --clean                Remove build-anyrc before configuring.
  --debug                Enable verbose kernel debug prints.
  --debug-surf           Forward Surf debug flag to CMake.
  --no-cross             Keep C/C++ cross toolchain disabled (default).
  --with-cross           Let CMake try external C/C++ cross builds.
  --reset                Recreate the disk image.
  --system-fs exfat
  --system-fs-size <MiB>
  --version <VERSION>    Override ANYOS_VERSION for this build.
  -D<VAR>=<VAL>          Pass an additional CMake cache definition.
  -h, --help             Show this help.

Environment:
  ANYRC_BUILD_DIR        Build directory (default: build-anyrc).
  ANYRC_BOOT_TIMEOUT     QEMU timeout when --boot is used (default: 60s).
  ANYRC_BOOT_MARKER      Serial marker that counts as boot success.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --kernel-only)
      BUILD_IMAGE=0
      VERIFY_BOOT=0
      ;;
    --no-boot)
      VERIFY_BOOT=0
      ;;
    --boot)
      VERIFY_BOOT=1
      BUILD_IMAGE=1
      ;;
    --clean)
      CLEAN=1
      ;;
    --debug)
      DEBUG_VERBOSE=1
      ;;
    --debug-surf)
      DEBUG_SURF=1
      ;;
    --no-cross)
      NO_CROSS=1
      ;;
    --with-cross)
      NO_CROSS=0
      ;;
    --reset)
      RESET=1
      ;;
    --system-fs)
      shift
      SYSTEM_FS="${1:-}"
      ;;
    --system-fs=*)
      SYSTEM_FS="${1#--system-fs=}"
      ;;
    --system-fs-size)
      shift
      SYSTEM_FS_SIZE_MIB="${1:-}"
      ;;
    --system-fs-size=*)
      SYSTEM_FS_SIZE_MIB="${1#--system-fs-size=}"
      ;;
    --version)
      shift
      ANYOS_VERSION="${1:-}"
      ;;
    --arm64|--uefi|--iso|--all)
      echo "build_with_anyrc: $1 is not wired yet; the anyrc proof path is x86_64 BIOS."
      exit 2
      ;;
    -D*)
      CMAKE_PASSTHROUGH+=("$1")
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "build_with_anyrc: unknown option: $1"
      usage
      exit 2
      ;;
  esac
  shift
done

case "$SYSTEM_FS" in
  ""|exfat) ;;
  *)
    echo "build_with_anyrc: --system-fs currently supports only 'exfat', got '${SYSTEM_FS}'"
    exit 2
    ;;
esac

if [[ -n "$SYSTEM_FS_SIZE_MIB" && ! "$SYSTEM_FS_SIZE_MIB" =~ ^[0-9]+$ ]]; then
  echo "build_with_anyrc: --system-fs-size expects a positive integer"
  exit 2
fi

if [[ "$CLEAN" -eq 1 ]]; then
  rm -rf "$BUILD_DIR"
fi

mkdir -p "$SHIM_DIR"

echo "Building host ccargo..."
"$REAL_CARGO" build \
  --manifest-path "${PROJECT_DIR}/bin/acargo/Cargo.toml" \
  --features host \
  --bin ccargo

cat > "$CARGO_SHIM" <<EOF
#!/usr/bin/env bash
set -euo pipefail

repo="${PROJECT_DIR}"
real_cargo="${REAL_CARGO}"
host_ccargo="${PROJECT_DIR}/target/debug/ccargo"
python_bin="${PYTHON:-python3}"

if [[ "\${1:-}" == +* ]]; then
  exec "\$real_cargo" "\$@"
fi

if [[ "\${1:-}" != "build" ]]; then
  exec "\$real_cargo" "\$@"
fi

copy_ccargo_elf_artifacts() {
  local src_profile="\$1"
  local dst_profile="\$2"
  mkdir -p "\$dst_profile"
  shopt -s nullglob
  for artifact in "\$src_profile"/*; do
    [[ -f "\$artifact" ]] || continue
    local base
    base="\$(basename "\$artifact")"
    case "\$base" in
      *.rlib|*.rmeta|*.d|*.o|*.a|*.so|*.dlib|*.kdrv|*.fp)
        continue
        ;;
    esac
    if [[ "\$base" == *.elf ]]; then
      cp "\$artifact" "\$dst_profile/\$base"
    else
      cp "\$artifact" "\$dst_profile/\$base.elf"
    fi
  done
  shopt -u nullglob
}

ccargo_target_from_cargo_target() {
  local target="\$1"
  local name
  name="\$(basename "\$target")"
  case "\$name" in
    x86_64-anyos-user.json|x86_64-anyos.json|x86_64-anyos-user|x86_64-anyos)
      echo "x86_64-anyos"
      ;;
    aarch64-anyos-user.json|aarch64-anyos.json|aarch64-anyos-user|aarch64-anyos)
      echo "aarch64-anyos"
      ;;
    "")
      echo "x86_64-anyos"
      ;;
    *)
      echo "\$target"
      ;;
  esac
}

target_triple_from_cargo_target() {
  local target="\$1"
  local name
  name="\$(basename "\$target")"
  case "\$name" in
    *.json)
      echo "\${name%.json}"
      ;;
    "")
      echo "x86_64-anyos-user"
      ;;
    *)
      echo "\$name"
      ;;
  esac
}

manifest_path=""
target_dir=""
target_spec=""
features=""
release=0
workspace=0
excludes=()

i=1
while [[ \$i -lt \$# ]]; do
  arg="\${!i}"
  case "\$arg" in
    --workspace)
      workspace=1
      ;;
    --exclude)
      i=\$((i + 1))
      excludes+=("\${!i:-}")
      ;;
    --release)
      release=1
      ;;
    --manifest-path)
      i=\$((i + 1))
      manifest_path="\${!i:-}"
      ;;
    --target-dir)
      i=\$((i + 1))
      target_dir="\${!i:-}"
      ;;
    --target)
      i=\$((i + 1))
      target_spec="\${!i:-}"
      ;;
    --features)
      i=\$((i + 1))
      features="\${!i:-}"
      ;;
  esac
  i=\$((i + 1))
done

if [[ "\$workspace" -eq 1 ]]; then
  profile="debug"
  if [[ "\$release" -eq 1 ]]; then
    profile="release"
  fi
  if [[ -z "\$target_dir" ]]; then
    target_dir="\$repo/target"
  fi
  ccargo_target="\$(ccargo_target_from_cargo_target "\$target_spec")"
  cargo_triple="\$(target_triple_from_cargo_target "\$target_spec")"
  dst_profile="\$target_dir/\$cargo_triple/\$profile"

  mapfile -t manifests < <("\$real_cargo" metadata --no-deps --format-version 1 \
    --manifest-path "\$repo/Cargo.toml" |
    ANYRC_REPO_ROOT="\$repo" "\$python_bin" -c '
import json, os, sys
from pathlib import Path
data = json.load(sys.stdin)
repo = Path(os.environ["ANYRC_REPO_ROOT"]).resolve()
for package in data.get("packages", []):
    manifest = package.get("manifest_path", "")
    name = package.get("name", "")
    if not manifest or name == "anyos_kernel":
        continue
    path = Path(manifest).resolve()
    try:
        rel = path.relative_to(repo).as_posix()
    except ValueError:
        continue
    if rel == "kernel/Cargo.toml":
        continue
    if not (rel.startswith("bin/") or rel.startswith("apps/") or rel.startswith("system/")):
        continue
    if package.get("id") in data.get("workspace_members", []):
        print(manifest)
')

  echo "anyrc-cargo-shim: building \${#manifests[@]} workspace packages with ccargo/anyrc"
  for manifest in "\${manifests[@]}"; do
    package_name="\$("\$python_bin" -c '
import pathlib, re, sys
text = pathlib.Path(sys.argv[1]).read_text()
m = re.search(r"(?m)^name\\s*=\\s*\\"([^\\"]+)\\"", text)
print(m.group(1) if m else pathlib.Path(sys.argv[1]).parent.name)
' "\$manifest")"
    skip=0
    for excluded in "\${excludes[@]}"; do
      if [[ "\$package_name" == "\$excluded" ]]; then
        skip=1
      fi
    done
    if [[ "\$skip" -eq 1 ]]; then
      continue
    fi

    package_dir="\$(dirname "\$manifest")"
    ccargo_args=(build "\$package_dir" --target "\$ccargo_target" --format elf)
    if [[ "\$release" -eq 1 ]]; then
      ccargo_args+=(--release)
    fi
    if [[ -n "\$features" ]]; then
      ccargo_args+=(--features "\$features")
    fi

    echo "anyrc-cargo-shim: workspace package \$package_name"
    "\$host_ccargo" "\${ccargo_args[@]}"
    copy_ccargo_elf_artifacts "\$package_dir/target/\$profile" "\$dst_profile"
  done
  exit 0
fi

if [[ "\$manifest_path" == "\$repo/kernel/Cargo.toml" ]]; then
  profile="debug"
  ccargo_args=(build "\$repo/kernel" --target x86_64-anyos --format elf)
  if [[ "\$release" -eq 1 ]]; then
    profile="release"
    ccargo_args+=(--release)
  fi
  if [[ -n "\$features" ]]; then
    ccargo_args+=(--features "\$features")
  fi

  "\$host_ccargo" "\${ccargo_args[@]}"

  out="\$repo/kernel/target/\$profile/anyos_kernel"
  if [[ ! -f "\$out" ]]; then
    echo "anyrc-cargo-shim: expected kernel artifact not found: \$out" >&2
    exit 1
  fi

  if [[ -n "\$target_dir" ]]; then
    mkdir -p "\$target_dir/x86_64-anyos/\$profile"
    cp "\$out" "\$target_dir/x86_64-anyos/\$profile/anyos_kernel.elf"
  fi
  exit 0
fi

if [[ -n "\$manifest_path" && "\$manifest_path" == "\$repo/system/"* ]]; then
  profile="debug"
  if [[ "\$release" -eq 1 ]]; then
    profile="release"
  fi
  if [[ -z "\$target_dir" ]]; then
    target_dir="\$(dirname "\$manifest_path")/target"
  fi
  ccargo_target="\$(ccargo_target_from_cargo_target "\$target_spec")"
  cargo_triple="\$(target_triple_from_cargo_target "\$target_spec")"
  ccargo_args=(build "\$(dirname "\$manifest_path")" --target "\$ccargo_target" --format elf)
  if [[ "\$release" -eq 1 ]]; then
    ccargo_args+=(--release)
  fi
  if [[ -n "\$features" ]]; then
    ccargo_args+=(--features "\$features")
  fi

  "\$host_ccargo" "\${ccargo_args[@]}"
  copy_ccargo_elf_artifacts "\$(dirname "\$manifest_path")/target/\$profile" "\$target_dir/\$cargo_triple/\$profile"
  exit 0
fi

exec "\$real_cargo" "\$@"
EOF
chmod +x "$CARGO_SHIM"

cmake_args=(
  -B "$BUILD_DIR"
  -G Ninja
  "-DCARGO_EXECUTABLE=${CARGO_SHIM}"
  "-DANYOS_DEBUG_VERBOSE=$([[ "$DEBUG_VERBOSE" -eq 1 ]] && echo ON || echo OFF)"
  "-DANYOS_DEBUG_SURF=$([[ "$DEBUG_SURF" -eq 1 ]] && echo ON || echo OFF)"
  "-DANYOS_NO_CROSS=$([[ "$NO_CROSS" -eq 1 ]] && echo ON || echo OFF)"
  "-DANYOS_RESET=$([[ "$RESET" -eq 1 ]] && echo ON || echo OFF)"
  "-DANYOS_VERSION=${ANYOS_VERSION}"
  "-DANYOS_ARCH=${ANYOS_ARCH}"
  "-DANYOS_BOOT_MODE=uefi"
)
cmake_args+=("-DANYOS_SYSTEM_FS=${SYSTEM_FS}")
cmake_args+=("-DANYOS_DUAL_PARTITION=OFF")
if [[ -n "$SYSTEM_FS_SIZE_MIB" ]]; then
  cmake_args+=("-DANYOS_SYSTEM_FS_SIZE_MIB=${SYSTEM_FS_SIZE_MIB}")
fi
cmake_args+=("${CMAKE_PASSTHROUGH[@]}")
cmake_args+=("$PROJECT_DIR")

echo "Configuring CMake/Ninja for anyrc kernel build..."
cmake "${cmake_args[@]}"

if [[ "$BUILD_IMAGE" -eq 0 ]]; then
  echo "Building kernel with ccargo/anyrc through the normal CMake target..."
  ninja -C "$BUILD_DIR" kernel

  KERNEL_ELF="${BUILD_DIR}/kernel/x86_64-anyos/release/anyos_kernel.elf"
  if [[ ! -s "$KERNEL_ELF" ]]; then
    echo "build_with_anyrc: missing kernel artifact: $KERNEL_ELF"
    exit 1
  fi
  echo "anyrc kernel: $KERNEL_ELF"
  exit 0
fi

echo "Building anyOS workspace, apps, sysroot, and BIOS image..."
ninja -C "$BUILD_DIR"

IMAGE="${BUILD_DIR}/anyos.img"
KERNEL_ELF="${BUILD_DIR}/kernel/x86_64-anyos/release/anyos_kernel.elf"
if [[ ! -s "$KERNEL_ELF" ]]; then
  echo "build_with_anyrc: missing kernel artifact: $KERNEL_ELF"
  exit 1
fi
if [[ ! -s "$IMAGE" ]]; then
  echo "build_with_anyrc: missing image artifact: $IMAGE"
  exit 1
fi
echo "anyrc kernel: $KERNEL_ELF"
echo "anyrc image:  $IMAGE"

if [[ "$VERIFY_BOOT" -eq 0 ]]; then
  exit 0
fi

BOOT_LOG="${BUILD_DIR}/anyrc-boot.log"
echo "Launching QEMU for up to ${BOOT_TIMEOUT}; serial log: ${BOOT_LOG}"
set +e
timeout "$BOOT_TIMEOUT" qemu-system-x86_64 \
  -cpu qemu64,+sse3,+ssse3,+sse4.1,+sse4.2,+popcnt \
  -drive "format=raw,file=${IMAGE}" \
  -m 1024M \
  -smp cpus=4 \
  -serial stdio \
  -vga std \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -no-reboot -no-shutdown \
  > "$BOOT_LOG" 2>&1
qemu_rc=$?
set -e

if grep -Fq "$BOOT_MARKER" "$BOOT_LOG"; then
  echo "Boot smoke: ok"
  exit 0
fi

echo "Boot smoke: marker not seen (qemu exit ${qemu_rc}). Log: ${BOOT_LOG}"
tail -80 "$BOOT_LOG" || true
exit 1
