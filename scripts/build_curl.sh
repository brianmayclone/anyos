#!/bin/bash
#
# Build script for curl (libcurl + curl CLI) for anyOS (i686-elf target)
# Produces: libcurl.a (static library) + curl.o (linked into ELF by CMake)
#
# Usage: bash scripts/build_curl.sh
#
set +e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CURL="$ROOT/third_party/curl"
CC="i686-elf-gcc"
AR="i686-elf-ar"
OBJDIR="$CURL/obj"

# ---------------------------------------------------------------------------
# Create output directories
# ---------------------------------------------------------------------------
mkdir -p "$OBJDIR/lib"
mkdir -p "$OBJDIR/lib/vauth"
mkdir -p "$OBJDIR/lib/vtls"
mkdir -p "$OBJDIR/lib/vquic"
mkdir -p "$OBJDIR/src"

# ---------------------------------------------------------------------------
# CFLAGS
# ---------------------------------------------------------------------------
CFLAGS_ARRAY=(
    -O2 -ffreestanding -nostdlib -fno-builtin -m32 -std=gnu99 -w
    -isystem "$ROOT/libs/libc/include"

    # Use config-anyos.h via the include path trick:
    # curl_setup.h does: #ifdef HAVE_CONFIG_H / #include "curl_config.h"
    # We place our config-anyos.h content into the expected location.
    -DHAVE_CONFIG_H
    -include stdbool.h
    -include "$CURL/lib/config-anyos.h"
    -DCURL_STATICLIB

    # Include paths
    "-I$CURL/include"
    "-I$CURL/lib"
    "-I$CURL/src"
    "-I$ROOT/third_party/bearssl/inc"
)

# ---------------------------------------------------------------------------
# Error tracking
# ---------------------------------------------------------------------------
SUCCESS=0
FAIL=0
ERRORS=""

compile_file() {
    local src="$1"
    local obj="$2"
    shift 2
    local out
    out=$("$CC" "${CFLAGS_ARRAY[@]}" "$@" -c "$src" -o "$obj" 2>&1)
    local ret=$?
    if [ $ret -ne 0 ]; then
        FAIL=$((FAIL+1))
        local fname=$(basename "$src")
        local err=$(echo "$out" | grep -m1 "error:")
        if [ -n "$err" ]; then
            ERRORS="$ERRORS$fname: $err\n"
        else
            ERRORS="$ERRORS$fname: UNKNOWN ERROR\n"
        fi
    else
        SUCCESS=$((SUCCESS+1))
    fi
}

# ---------------------------------------------------------------------------
# Create curl_config.h that just includes our config
# ---------------------------------------------------------------------------
cat > "$CURL/lib/curl_config.h" <<'CONF'
/* Auto-generated — redirects to config-anyos.h */
#include "config-anyos.h"
CONF

# ===========================================================================
# Compile libcurl (library)
# ===========================================================================
echo "=== Compiling libcurl ==="

# Core library files — we skip protocol/feature files that are disabled
LIB_CORE_FILES=(
    base64.c
    bufq.c
    bufref.c
    cf-https-connect.c
    cf-socket.c
    cfilters.c
    conncache.c
    connect.c
    content_encoding.c
    cookie.c
    curl_addrinfo.c
    curl_sha512_256.c
    curl_endian.c
    curl_fnmatch.c
    curl_get_line.c
    curl_gethostname.c
    curl_memrchr.c
    curl_multibyte.c
    curl_range.c
    curl_sasl.c
    curl_trc.c
    cw-out.c
    dynbuf.c
    dynhds.c
    easy.c
    easygetopt.c
    easyoptions.c
    escape.c
    file.c
    fileinfo.c
    fopen.c
    formdata.c
    ftp.c
    ftplistparser.c
    getenv.c
    getinfo.c
    hash.c
    headers.c
    hmac.c
    hostasyn.c
    hostip.c
    hostip4.c
    hostsyn.c
    http.c
    http1.c
    http_chunks.c
    http_digest.c
    idn.c
    if2ip.c
    inet_ntop.c
    inet_pton.c
    llist.c
    md5.c
    mime.c
    mprintf.c
    multi.c
    nonblock.c
    noproxy.c
    parsedate.c
    pingpong.c
    progress.c
    rand.c
    rename.c
    request.c
    select.c
    sendf.c
    setopt.c
    sha256.c
    share.c
    slist.c
    speedcheck.c
    splay.c
    strcase.c
    strdup.c
    strerror.c
    strparse.c
    strtok.c
    strtoofft.c
    timediff.c
    timeval.c
    transfer.c
    url.c
    urlapi.c
    version.c
    warnless.c
)

# vauth
LIB_VAUTH_FILES=(
    vauth/cleartext.c
    vauth/cram.c
    vauth/digest.c
    vauth/oauth2.c
    vauth/vauth.c
)

# vtls (BearSSL TLS backend)
LIB_VTLS_FILES=(
    vtls/bearssl.c
    vtls/cipher_suite.c
    vtls/hostcheck.c
    vtls/keylog.c
    vtls/vtls.c
    vtls/vtls_scache.c
)

echo "  [lib core]"
for f in "${LIB_CORE_FILES[@]}"; do
    obj="$OBJDIR/lib/$(basename "$f" .c).o"
    if [ "$CURL/lib/$f" -nt "$obj" ] || [ ! -f "$obj" ]; then
        compile_file "$CURL/lib/$f" "$obj" -DBUILDING_LIBCURL
    else
        SUCCESS=$((SUCCESS+1))
    fi
done

echo "  [lib vauth]"
for f in "${LIB_VAUTH_FILES[@]}"; do
    obj="$OBJDIR/lib/vauth/$(basename "$f" .c).o"
    if [ "$CURL/lib/$f" -nt "$obj" ] || [ ! -f "$obj" ]; then
        compile_file "$CURL/lib/$f" "$obj" -DBUILDING_LIBCURL
    else
        SUCCESS=$((SUCCESS+1))
    fi
done

echo "  [lib vtls]"
for f in "${LIB_VTLS_FILES[@]}"; do
    obj="$OBJDIR/lib/vtls/$(basename "$f" .c).o"
    if [ "$CURL/lib/$f" -nt "$obj" ] || [ ! -f "$obj" ]; then
        compile_file "$CURL/lib/$f" "$obj" -DBUILDING_LIBCURL
    else
        SUCCESS=$((SUCCESS+1))
    fi
done

echo "  [lib vquic]"
compile_file "$CURL/lib/vquic/vquic.c" "$OBJDIR/lib/vquic/vquic.o" -DBUILDING_LIBCURL

# ===========================================================================
# Compile curl CLI tool
# ===========================================================================
echo "  [curl tool]"

TOOL_FILES=(
    terminal.c
    slist_wc.c
    tool_bname.c
    tool_cb_dbg.c
    tool_cb_hdr.c
    tool_cb_prg.c
    tool_cb_rea.c
    tool_cb_see.c
    tool_cb_soc.c
    tool_cb_wrt.c
    tool_cfgable.c
    tool_dirhie.c
    tool_doswin.c
    tool_easysrc.c
    tool_filetime.c
    tool_findfile.c
    tool_formparse.c
    tool_getparam.c
    tool_getpass.c
    tool_help.c
    tool_helpers.c
    tool_ipfs.c
    tool_libinfo.c
    tool_listhelp.c
    tool_main.c
    tool_msgs.c
    tool_operate.c
    tool_operhlp.c
    tool_paramhlp.c
    tool_parsecfg.c
    tool_progress.c
    tool_setopt.c
    tool_sleep.c
    tool_ssls.c
    tool_stderr.c
    tool_strdup.c
    tool_urlglob.c
    tool_util.c
    tool_vms.c
    tool_writeout.c
    tool_writeout_json.c
    tool_xattr.c
    var.c
)

for f in "${TOOL_FILES[@]}"; do
    obj="$OBJDIR/src/$(basename "$f" .c).o"
    if [ "$CURL/src/$f" -nt "$obj" ] || [ ! -f "$obj" ]; then
        compile_file "$CURL/src/$f" "$obj"
    else
        SUCCESS=$((SUCCESS+1))
    fi
done

# Shared lib files compiled for tool (without BUILDING_LIBCURL for curlx_ names)
echo "  [tool shared libs]"
TOOL_LIB_FILES=(dynbuf.c warnless.c base64.c)
for f in "${TOOL_LIB_FILES[@]}"; do
    obj="$OBJDIR/src/tool_$(basename "$f" .c).o"
    if [ "$CURL/lib/$f" -nt "$obj" ] || [ ! -f "$obj" ]; then
        compile_file "$CURL/lib/$f" "$obj"
    else
        SUCCESS=$((SUCCESS+1))
    fi
done

# ===========================================================================
# Results
# ===========================================================================
echo ""
echo "=== Build Results ==="
echo "SUCCESS: $SUCCESS, FAIL: $FAIL"

if [ $FAIL -gt 0 ]; then
    echo ""
    echo "=== Errors ==="
    echo -e "$ERRORS"
    exit 1
fi

# ===========================================================================
# Create static library
# ===========================================================================
echo ""
echo "Creating libcurl.a..."
"$AR" rcs "$CURL/libcurl.a" \
    "$OBJDIR"/lib/*.o \
    "$OBJDIR"/lib/vauth/*.o \
    "$OBJDIR"/lib/vtls/*.o \
    "$OBJDIR"/lib/vquic/*.o \
    "$OBJDIR"/src/*.o

echo "=== Done: $CURL/libcurl.a ($(du -h "$CURL/libcurl.a" | cut -f1)) ==="
