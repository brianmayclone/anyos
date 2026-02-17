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
