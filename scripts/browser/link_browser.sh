#!/bin/bash
# Link the NetSurf browser for anyOS
# Combines all static libraries into a single ELF binary
#
# Output: third_party/netsurf/netsurf/browser.elf
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
NS="$ROOT/third_party/netsurf"
CC="i686-elf-gcc"

LIBC_DIR="$ROOT/libs/libc"
CRT0="$LIBC_DIR/obj/crt0.o"
LIBC_A="$LIBC_DIR/libc.a"
LINK_LD="$LIBC_DIR/link.ld"

# Build identifier for finding component library build dirs
BUILD_SUFFIX=$(ls -d "$NS/libwapcaplet/build-"*"-release-lib-static" 2>/dev/null | head -1)
BUILD_SUFFIX=$(basename "$BUILD_SUFFIX")

OUTPUT="$NS/netsurf/browser.elf"

echo "=== Linking NetSurf browser for anyOS ==="

# Verify all required inputs exist
MISSING=""
for f in "$CRT0" "$LIBC_A" "$LINK_LD"; do
    [ ! -f "$f" ] && MISSING="$MISSING  $(basename "$f")\n"
done

LIBS=(
    "$NS/netsurf/libnetsurf.a"
    "$NS/libcss/$BUILD_SUFFIX/libcss.a"
    "$NS/libdom/$BUILD_SUFFIX/libdom.a"
    "$NS/libhubbub/$BUILD_SUFFIX/libhubbub.a"
    "$NS/libparserutils/$BUILD_SUFFIX/libparserutils.a"
    "$NS/libwapcaplet/$BUILD_SUFFIX/libwapcaplet.a"
    "$NS/libnsfb/$BUILD_SUFFIX/libnsfb.a"
    "$NS/libnsgif/$BUILD_SUFFIX/libnsgif.a"
    "$NS/libnsbmp/$BUILD_SUFFIX/libnsbmp.a"
    "$NS/libnsutils/$BUILD_SUFFIX/libnsutils.a"
    "$NS/libnslog/libnslog.a"
    "$NS/libnspsl/$BUILD_SUFFIX/libnspsl.a"
    "$ROOT/third_party/curl/lib/libcurl.a"
    "$ROOT/third_party/bearssl/build/libbearssl.a"
    "$ROOT/third_party/zlib/libz.a"
)

for lib in "${LIBS[@]}"; do
    [ ! -f "$lib" ] && MISSING="$MISSING  $lib\n"
done

if [ -n "$MISSING" ]; then
    echo "ERROR: Missing files:"
    echo -e "$MISSING"
    exit 1
fi

echo "  Linking with $(echo "${LIBS[@]}" | wc -w | tr -d ' ') libraries..."

LIBNSFB="$NS/libnsfb/$BUILD_SUFFIX/libnsfb.a"

# Build LIBS_NO_NSFB (all libs except libnsfb — it needs --whole-archive)
LIBS_NO_NSFB=()
for lib in "${LIBS[@]}"; do
    [[ "$lib" != "$LIBNSFB" ]] && LIBS_NO_NSFB+=("$lib")
done

$CC -nostdlib -static -m32 \
    -T "$LINK_LD" \
    -o "$OUTPUT" \
    "$CRT0" \
    -Wl,--allow-multiple-definition \
    -Wl,--start-group \
    -Wl,--whole-archive "$LIBNSFB" -Wl,--no-whole-archive \
    "${LIBS_NO_NSFB[@]}" \
    "$LIBC_A" \
    -lgcc \
    -Wl,--end-group

SIZE=$(du -h "$OUTPUT" | cut -f1)
echo "=== Done: $OUTPUT ($SIZE) ==="
