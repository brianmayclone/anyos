#!/bin/bash
#
# Build script for NetSurf framebuffer browser for anyOS (i686-elf target)
# Produces a static library libnetsurf.a containing all NetSurf object files.
#
# Usage: bash build_netsurf.sh
#
set +e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
NS="$ROOT/third_party/netsurf/netsurf"
CC="i686-elf-gcc"
AR="i686-elf-ar"
OBJDIR="$NS/obj"

# ---------------------------------------------------------------------------
# Create output directories
# ---------------------------------------------------------------------------
mkdir -p "$OBJDIR"
mkdir -p "$OBJDIR/content"
mkdir -p "$OBJDIR/content/fetchers"
mkdir -p "$OBJDIR/content/fetchers/about"
mkdir -p "$OBJDIR/content/fetchers/file"
mkdir -p "$OBJDIR/content/handlers/css"
mkdir -p "$OBJDIR/content/handlers/html"
mkdir -p "$OBJDIR/content/handlers/image"
mkdir -p "$OBJDIR/content/handlers/javascript/none"
mkdir -p "$OBJDIR/content/handlers/text"
mkdir -p "$OBJDIR/desktop"
mkdir -p "$OBJDIR/utils"
mkdir -p "$OBJDIR/utils/http"
mkdir -p "$OBJDIR/utils/nsurl"
mkdir -p "$OBJDIR/frontends/framebuffer"
mkdir -p "$OBJDIR/frontends/framebuffer/fbtk"
mkdir -p "$OBJDIR/generated"

# ---------------------------------------------------------------------------
# CFLAGS - stored in an array to preserve quoting of string defines
# ---------------------------------------------------------------------------
CFLAGS_ARRAY=(
    -O2 -ffreestanding -nostdlib -fno-builtin -m32 -std=c99 -w
    -Wno-error=implicit-function-declaration -Wno-error=return-type -Wno-error=int-conversion -Wno-error=incompatible-pointer-types
    -isystem "$ROOT/libs/libc/include"

    # Feature/config defines
    -Dnsframebuffer -Dsmall
    '-DNETSURF_FB_RESPATH="/Applications/Browser.app/res/"'
    '-DNETSURF_FB_FONTPATH="/Applications/Browser.app/res/fonts"'
    '-DNETSURF_FB_FONT_SANS_SERIF="DejaVuSans.ttf"'
    '-DNETSURF_FB_FONT_SANS_SERIF_BOLD="DejaVuSans-Bold.ttf"'
    '-DNETSURF_FB_FONT_SANS_SERIF_ITALIC="DejaVuSans-Oblique.ttf"'
    '-DNETSURF_FB_FONT_SANS_SERIF_ITALIC_BOLD="DejaVuSans-BoldOblique.ttf"'
    '-DNETSURF_FB_FONT_SERIF="DejaVuSerif.ttf"'
    '-DNETSURF_FB_FONT_SERIF_BOLD="DejaVuSerif-Bold.ttf"'
    '-DNETSURF_FB_FONT_MONOSPACE="DejaVuSansMono.ttf"'
    '-DNETSURF_FB_FONT_MONOSPACE_BOLD="DejaVuSansMono-Bold.ttf"'
    '-DNETSURF_FB_FONT_CURSIVE="Comic_Sans_MS.ttf"'
    '-DNETSURF_FB_FONT_FANTASY="Impact.ttf"'
    '-DNETSURF_HOMEPAGE="about:blank"'
    -DNETSURF_LOG_LEVEL=WARNING
    '-DNETSURF_BUILTIN_LOG_FILTER="(level:WARNING)"'
    '-DNETSURF_BUILTIN_VERBOSE_FILTER="(level:VERBOSE)"'
    '-DNETSURF_UA_FORMAT_STRING="Mozilla/5.0 (%s) NetSurf/%d.%d"'
    -DCURL_STATICLIB
    -DSTMTEXPR=1

    # Disable features we do not have libraries for
    -UWITH_JPEG
    -UWITH_PNG
    -UWITH_WEBP
    -UWITH_VIDEO
    -UWITH_NSSPRITE
    -UWITH_NS_SVG
    -DWITH_BMP
    -DWITH_GIF

    # anyOS-specific: no POSIX networking/OS headers in freestanding
    -UHAVE_POSIX_INET_HEADERS
    -UHAVE_MMAP
    -UWITH_MMAP
    -UHAVE_SYS_SELECT
    -UHAVE_UTSNAME
    -UHAVE_REALPATH
    -UHAVE_MKDIR
    -UHAVE_SIGPIPE
    -UHAVE_EXECINFO
    -UHAVE_SCANDIR
    -UHAVE_DIRFD
    -UHAVE_UNLINKAT
    -UHAVE_FSTATAT
    -UHAVE_REGEX
    -DNO_IPV6

    # Include paths
    "-I$NS/"
    "-I$NS/include/"
    "-I$NS/frontends/"
    "-I$NS/content/handlers/"
    "-I$OBJDIR/generated/"
    "-I$ROOT/third_party/netsurf/libcss/include/"
    "-I$ROOT/third_party/netsurf/libdom/include/"
    "-I$ROOT/third_party/netsurf/libdom/"
    "-I$ROOT/third_party/netsurf/libhubbub/include/"
    "-I$ROOT/third_party/netsurf/libparserutils/include/"
    "-I$ROOT/third_party/netsurf/libwapcaplet/include/"
    "-I$ROOT/third_party/netsurf/libnsfb/include/"
    "-I$ROOT/third_party/netsurf/libnsutils/include/"
    "-I$ROOT/third_party/netsurf/libnslog/include/"
    "-I$ROOT/third_party/netsurf/libnspsl/include/"
    "-I$ROOT/third_party/netsurf/libnsgif/include/"
    "-I$ROOT/third_party/netsurf/libnsbmp/include/"
    "-I$ROOT/third_party/curl/include/"
    "-I$ROOT/third_party/bearssl/inc/"
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
    local out
    out=$("$CC" "${CFLAGS_ARRAY[@]}" -c "$src" -o "$obj" 2>&1)
    local ret=$?
    if [ $ret -ne 0 ]; then
        FAIL=$((FAIL+1))
        local fname=$(basename "$src")
        local fatal=$(echo "$out" | grep -m1 "fatal error")
        local err=$(echo "$out" | grep -m1 "error:")
        if [ -n "$fatal" ]; then
            ERRORS="$ERRORS$fname: $fatal\n"
        elif [ -n "$err" ]; then
            ERRORS="$ERRORS$fname: $err\n"
        else
            ERRORS="$ERRORS$fname: UNKNOWN ERROR\n"
        fi
    else
        SUCCESS=$((SUCCESS+1))
    fi
}

# ===========================================================================
# STEP 1: Generate stub/placeholder files that the build system normally
#          creates with tools (convert_image, convert_font, testament)
# ===========================================================================

echo "=== Generating stub files ==="

# ---------------------------------------------------------------------------
# 1a. testament.h - version/build information
# ---------------------------------------------------------------------------
cat > "$OBJDIR/generated/testament.h" << 'TESTAMENT_EOF'
/* Generated testament for anyOS build */
#ifndef NETSURF_TESTAMENT_H
#define NETSURF_TESTAMENT_H

#define WT_REVID "anyos-custom"
#define WT_COMPILEDATE __DATE__
#define WT_BRANCHPATH "main"
#define WT_MODIFIED 0
#define WT_BRANCHISTRUNK
#define WT_MODIFICATIONS { { NULL, NULL } }

#define USERNAME "anyos"
#define GECOS "anyOS Builder"
#define WT_HOSTNAME "anyos"
#define WT_ROOT "/netsurf"

#endif /* NETSURF_TESTAMENT_H */
TESTAMENT_EOF

# ---------------------------------------------------------------------------
# 1b. atestament.h - about:testament modified file list
#     This header is included by about.c and testament.c in the about fetcher.
#     It normally contains the list of modified files from the VCS.
# ---------------------------------------------------------------------------
cat > "$OBJDIR/generated/atestament.h" << 'ATESTAMENT_EOF'
/* Generated atestament for anyOS build */
#ifndef NETSURF_ATESTAMENT_H
#define NETSURF_ATESTAMENT_H

/* Empty - no modified files in anyOS custom build */

#endif /* NETSURF_ATESTAMENT_H */
ATESTAMENT_EOF

# ---------------------------------------------------------------------------
# 1c. font-ns-sans.h - internal font data
#     The real font data is generated by tools/convert_font from
#     frontends/framebuffer/res/fonts/glyph_data. We generate a minimal
#     stub with empty tables so font_internal.c compiles. The font will
#     fall back to the built-in hex codepoint renderer for all glyphs.
# ---------------------------------------------------------------------------
cat > "$OBJDIR/generated/font-ns-sans.h" << 'FONT_EOF'
/* Generated minimal font stub for anyOS build.
 * All glyph lookups will fall through to the hex codepoint renderer
 * in font_internal.c because all section tables point to section 0
 * and all offsets in sections are 0 (triggering the fallthrough).
 */
#ifndef FONT_NS_SANS_H
#define FONT_NS_SANS_H

#include <stdint.h>

/* Section tables: 256 entries each, all zero.
 * Section 0 existing means ucs4/256==0 still gets checked,
 * but the sections offset will be 0 which means "no glyph". */
static const uint8_t fb_regular_section_table_c[256] = {0};
const uint8_t *fb_regular_section_table = &fb_regular_section_table_c[0];

static const uint8_t fb_italic_section_table_c[256] = {0};
const uint8_t *fb_italic_section_table = &fb_italic_section_table_c[0];

static const uint8_t fb_bold_section_table_c[256] = {0};
const uint8_t *fb_bold_section_table = &fb_bold_section_table_c[0];

static const uint8_t fb_bold_italic_section_table_c[256] = {0};
const uint8_t *fb_bold_italic_section_table = &fb_bold_italic_section_table_c[0];

/* Sections: 1 section of 256 uint16_t entries, all zero (= no glyph). */
static const uint16_t fb_regular_sections_c[256] = {0};
const uint16_t *fb_regular_sections = &fb_regular_sections_c[0];

static const uint16_t fb_italic_sections_c[256] = {0};
const uint16_t *fb_italic_sections = &fb_italic_sections_c[0];

static const uint16_t fb_bold_sections_c[256] = {0};
const uint16_t *fb_bold_sections = &fb_bold_sections_c[0];

static const uint16_t fb_bold_italic_sections_c[256] = {0};
const uint16_t *fb_bold_italic_sections = &fb_bold_italic_sections_c[0];

/* Glyph data: just one 16-byte "missing glyph" block at offset 0.
 * Offset 0 is the "no glyph" sentinel, so this is never actually used
 * as a real glyph - the fallthrough to get_codepoint() handles it. */
static const uint8_t font_glyph_data_c[16] = {
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff
};
const uint8_t *font_glyph_data = &font_glyph_data_c[0];

#endif /* FONT_NS_SANS_H */
FONT_EOF

# ---------------------------------------------------------------------------
# 1d. Image data stubs
#     NetSurf's framebuffer frontend expects struct fbtk_bitmap globals
#     for toolbar icons, cursor images, and throbber animation frames.
#     Normally generated by tools/convert_image from PNG files.
#     We provide 1x1 pixel BGRA stubs (transparent).
# ---------------------------------------------------------------------------
cat > "$OBJDIR/generated/image_data_stubs.c" << 'IMAGE_EOF'
/*
 * Generated image data stubs for anyOS build.
 * All images are 1x1 transparent pixels.
 * The real images would be converted from PNG by tools/convert_image.
 */
#include <stdint.h>
#include <stdbool.h>

struct fbtk_bitmap {
    int width;
    int height;
    uint8_t *pixdata;
    bool opaque;
    int hot_x;
    int hot_y;
};

/* 1x1 transparent pixel (BGRA) */
static uint8_t stub_pixel_transparent[4] = {0, 0, 0, 0};
/* 1x1 opaque black pixel (BGRA) */
static uint8_t stub_pixel_opaque[4] = {0, 0, 0, 255};

/* Toolbar icons */
struct fbtk_bitmap left_arrow = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap right_arrow = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap reload = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap stop_image = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap history_image = { 1, 1, stub_pixel_opaque, true, 0, 0 };

/* Greyed-out toolbar icons */
struct fbtk_bitmap left_arrow_g = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap right_arrow_g = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap reload_g = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap stop_image_g = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap history_image_g = { 1, 1, stub_pixel_opaque, true, 0, 0 };

/* Scroll bar icons */
struct fbtk_bitmap scrolll = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap scrollr = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap scrollu = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap scrolld = { 1, 1, stub_pixel_opaque, true, 0, 0 };

/* On-screen keyboard icon */
struct fbtk_bitmap osk_image = { 1, 1, stub_pixel_opaque, true, 0, 0 };

/* Cursor images */
struct fbtk_bitmap pointer_image = { 1, 1, stub_pixel_opaque, false, 0, 0 };
struct fbtk_bitmap hand_image = { 1, 1, stub_pixel_opaque, false, 0, 0 };
struct fbtk_bitmap caret_image = { 1, 1, stub_pixel_opaque, false, 0, 0 };
struct fbtk_bitmap menu_image = { 1, 1, stub_pixel_opaque, false, 0, 0 };
struct fbtk_bitmap move_image = { 1, 1, stub_pixel_opaque, false, 0, 0 };
struct fbtk_bitmap progress_image = { 1, 1, stub_pixel_opaque, false, 0, 0 };

/* Throbber animation frames */
struct fbtk_bitmap throbber0 = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap throbber1 = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap throbber2 = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap throbber3 = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap throbber4 = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap throbber5 = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap throbber6 = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap throbber7 = { 1, 1, stub_pixel_opaque, true, 0, 0 };
struct fbtk_bitmap throbber8 = { 1, 1, stub_pixel_opaque, true, 0, 0 };
IMAGE_EOF

echo "   Generated: testament.h, atestament.h, font-ns-sans.h, image_data_stubs.c"

# ===========================================================================
# STEP 2: Compile all source files
# ===========================================================================

echo ""
echo "=== Compiling NetSurf sources ==="

# ---------------------------------------------------------------------------
# 2a. Generated/stub files
# ---------------------------------------------------------------------------
echo "  [generated stubs]"
compile_file "$OBJDIR/generated/image_data_stubs.c" "$OBJDIR/generated/image_data_stubs.o"

# ---------------------------------------------------------------------------
# 2b. content/ - Core content handling
# ---------------------------------------------------------------------------
echo "  [content/]"
for f in content.c content_factory.c fetch.c hlcache.c llcache.c \
         mimesniff.c textsearch.c urldb.c no_backing_store.c; do
    compile_file "$NS/content/$f" "$OBJDIR/content/${f%.c}.o"
done

# ---------------------------------------------------------------------------
# 2c. content/fetchers/ - Data fetchers
# ---------------------------------------------------------------------------
echo "  [content/fetchers/]"
# Core fetchers (data, resource, curl)
for f in data.c resource.c curl.c; do
    compile_file "$NS/content/fetchers/$f" "$OBJDIR/content/fetchers/${f%.c}.o"
done

# About fetcher
echo "  [content/fetchers/about/]"
for f in about.c blank.c certificate.c chart.c choices.c config.c \
         imagecache.c nscolours.c query.c query_auth.c query_fetcherror.c \
         query_privacy.c query_timeout.c testament.c websearch.c; do
    compile_file "$NS/content/fetchers/about/$f" "$OBJDIR/content/fetchers/about/${f%.c}.o"
done

# File fetcher
echo "  [content/fetchers/file/]"
for f in dirlist.c file.c; do
    compile_file "$NS/content/fetchers/file/$f" "$OBJDIR/content/fetchers/file/${f%.c}.o"
done

# ---------------------------------------------------------------------------
# 2d. content/handlers/css/ - CSS engine bindings
# ---------------------------------------------------------------------------
echo "  [content/handlers/css/]"
for f in css.c dump.c internal.c hints.c select.c; do
    compile_file "$NS/content/handlers/css/$f" "$OBJDIR/content/handlers/css/${f%.c}.o"
done

# ---------------------------------------------------------------------------
# 2e. content/handlers/javascript/ - JavaScript (none/stub)
# ---------------------------------------------------------------------------
echo "  [content/handlers/javascript/]"
compile_file "$NS/content/handlers/javascript/none/none.c" \
             "$OBJDIR/content/handlers/javascript/none/none.o"
compile_file "$NS/content/handlers/javascript/fetcher.c" \
             "$OBJDIR/content/handlers/javascript/fetcher.o"

# ---------------------------------------------------------------------------
# 2f. content/handlers/html/ - HTML content handler
# ---------------------------------------------------------------------------
echo "  [content/handlers/html/]"
for f in box_construct.c box_inspect.c box_manipulate.c box_normalise.c \
         box_special.c box_textarea.c css.c css_fetcher.c dom_event.c \
         font.c form.c forms.c html.c imagemap.c interaction.c layout.c \
         layout_flex.c object.c redraw.c redraw_border.c script.c \
         table.c textselection.c; do
    compile_file "$NS/content/handlers/html/$f" "$OBJDIR/content/handlers/html/${f%.c}.o"
done

# ---------------------------------------------------------------------------
# 2g. content/handlers/image/ - Image handlers (only core + BMP/GIF)
# ---------------------------------------------------------------------------
echo "  [content/handlers/image/]"
# Always included: image.c image_cache.c
# BMP/GIF: bmp.c ico.c gif.c (we have libnsbmp and libnsgif)
for f in image.c image_cache.c bmp.c ico.c gif.c; do
    compile_file "$NS/content/handlers/image/$f" "$OBJDIR/content/handlers/image/${f%.c}.o"
done

# ---------------------------------------------------------------------------
# 2h. content/handlers/text/ - Text content handler
# ---------------------------------------------------------------------------
echo "  [content/handlers/text/]"
compile_file "$NS/content/handlers/text/textplain.c" \
             "$OBJDIR/content/handlers/text/textplain.o"

# ---------------------------------------------------------------------------
# 2i. utils/ - Utility functions
# ---------------------------------------------------------------------------
echo "  [utils/]"
for f in bloom.c corestrings.c file.c filepath.c hashmap.c hashtable.c \
         idna.c libdom.c log.c messages.c nscolour.c nsoption.c punycode.c \
         ssl_certs.c talloc.c time.c url.c useragent.c utf8.c utils.c; do
    compile_file "$NS/utils/$f" "$OBJDIR/utils/${f%.c}.o"
done

# ---------------------------------------------------------------------------
# 2j. utils/http/ - HTTP utility functions
# ---------------------------------------------------------------------------
echo "  [utils/http/]"
for f in challenge.c generics.c primitives.c parameter.c \
         cache-control.c content-disposition.c content-type.c \
         strict-transport-security.c www-authenticate.c; do
    compile_file "$NS/utils/http/$f" "$OBJDIR/utils/http/${f%.c}.o"
done

# ---------------------------------------------------------------------------
# 2k. utils/nsurl/ - URL parsing
# ---------------------------------------------------------------------------
echo "  [utils/nsurl/]"
for f in nsurl.c parse.c; do
    compile_file "$NS/utils/nsurl/$f" "$OBJDIR/utils/nsurl/${f%.c}.o"
done

# ---------------------------------------------------------------------------
# 2l. desktop/ - Desktop/browser core
# ---------------------------------------------------------------------------
echo "  [desktop/]"
# S_DESKTOP (common desktop sources)
for f in cookie_manager.c knockout.c hotlist.c mouse.c plot_style.c \
         print.c search.c searchweb.c scrollbar.c textarea.c version.c \
         system_colour.c local_history.c global_history.c treeview.c \
         page-info.c; do
    compile_file "$NS/desktop/$f" "$OBJDIR/desktop/${f%.c}.o"
done

# S_BROWSER (browser window sources)
for f in bitmap.c browser.c browser_window.c browser_history.c \
         download.c frames.c netsurf.c cw_helper.c \
         save_complete.c save_text.c selection.c textinput.c gui_factory.c \
         save_pdf.c font_haru.c; do
    compile_file "$NS/desktop/$f" "$OBJDIR/desktop/${f%.c}.o"
done

# ---------------------------------------------------------------------------
# 2m. frontends/framebuffer/ - Framebuffer frontend
# ---------------------------------------------------------------------------
echo "  [frontends/framebuffer/]"
for f in gui.c framebuffer.c schedule.c bitmap.c fetch.c \
         findfile.c corewindow.c local_history.c clipboard.c \
         font_internal.c; do
    compile_file "$NS/frontends/framebuffer/$f" "$OBJDIR/frontends/framebuffer/${f%.c}.o"
done

# ---------------------------------------------------------------------------
# 2n. frontends/framebuffer/fbtk/ - Framebuffer toolkit
# ---------------------------------------------------------------------------
echo "  [frontends/framebuffer/fbtk/]"
for f in fbtk.c event.c fill.c bitmap.c user.c window.c \
         text.c scroll.c osk.c; do
    compile_file "$NS/frontends/framebuffer/fbtk/$f" "$OBJDIR/frontends/framebuffer/fbtk/${f%.c}.o"
done

# ===========================================================================
# STEP 3: Report results
# ===========================================================================

echo ""
echo "=== Build Results ==="
echo "SUCCESS: $SUCCESS, FAIL: $FAIL"

if [ -n "$ERRORS" ]; then
    echo ""
    echo "=== ERRORS ==="
    echo -e "$ERRORS" | sort -u
fi

# ===========================================================================
# STEP 4: Create static library (only if no failures)
# ===========================================================================

if [ $FAIL -eq 0 ]; then
    echo ""
    echo "Creating libnetsurf.a..."

    # Collect all .o files
    OBJS=$(find "$OBJDIR" -name '*.o' -type f)

    OUTPUT="$NS/libnetsurf.a"
    $AR rcs "$OUTPUT" $OBJS
    echo "=== Done: $OUTPUT ($(du -h "$OUTPUT" | cut -f1)) ==="
else
    echo ""
    echo "Skipping library creation due to $FAIL compile error(s)."
    echo ""
    echo "To debug, try compiling a single failing file manually:"
    echo "  $CC [CFLAGS] -c <source.c> -o /dev/null"
fi
