/*
 * vid_anyos.c — Quake video driver for anyOS compositor
 *
 * Creates a 640x480 window via the anyOS compositor IPC protocol.
 * Quake renders into an 8-bit indexed buffer; VID_Update converts
 * palette indices to ARGB and blits to the SHM window surface.
 */

#include "quakedef.h"
#include "d_local.h"

#include <stdio.h>
#include <string.h>
#include <stdint.h>

/* ── anyOS Syscall Numbers ── */
#define SYS_SLEEP              8
#define SYS_GETPID             6
#define SYS_EVT_CHAN_CREATE    63
#define SYS_EVT_CHAN_SUBSCRIBE 64
#define SYS_EVT_CHAN_EMIT      65
#define SYS_EVT_CHAN_POLL      66
#define SYS_SHM_CREATE        140
#define SYS_SHM_MAP           141

/* ── IPC Protocol Constants ── */
#define CMD_CREATE_WINDOW   0x1001
#define CMD_PRESENT         0x1003
#define CMD_SET_TITLE       0x1004
#define RESP_WINDOW_CREATED 0x2001
#define EVT_KEY_DOWN        0x3001
#define EVT_KEY_UP          0x3002
#define EVT_MOUSE_MOVE      0x3003
#define EVT_MOUSE_DOWN      0x3004
#define EVT_MOUSE_UP        0x3005
#define EVT_WINDOW_CLOSE    0x3007

extern int _syscall(int num, int a1, int a2, int a3, int a4);

/* ── Video Configuration ── */
#define BASEWIDTH   640
#define BASEHEIGHT  480

viddef_t vid;

static byte vid_buffer[BASEWIDTH * BASEHEIGHT];
static short zbuffer[BASEWIDTH * BASEHEIGHT];
static byte surfcache[2048 * 1024]; /* 2 MB surface cache for 640x480 */

unsigned short d_8to16table[256];
unsigned d_8to24table[256];

/* ── Compositor State ── */
static uint32_t g_channel_id;
static uint32_t g_sub_id;
static uint32_t g_window_id;
static uint32_t g_shm_id;
static uint32_t *g_surface;

/* Palette: 256 entries, ARGB */
static uint32_t g_palette[256];

/* ── anyOS Key Code → Quake Key Mapping ── */

/* anyOS compositor key codes (from keys.rs) */
#define AK_ENTER     0x100
#define AK_BACKSPACE 0x101
#define AK_TAB       0x102
#define AK_ESCAPE    0x103
#define AK_SPACE     0x104
#define AK_UP        0x105
#define AK_DOWN      0x106
#define AK_LEFT      0x107
#define AK_RIGHT     0x108
#define AK_DELETE    0x120
#define AK_HOME      0x121
#define AK_END       0x122
#define AK_PGUP      0x123
#define AK_PGDN      0x124
#define AK_F1        0x140
#define AK_LSHIFT    0x130
#define AK_RSHIFT    0x131
#define AK_LCTRL     0x132
#define AK_RCTRL     0x133
#define AK_LALT      0x134
#define AK_RALT      0x135

static int translate_key(uint32_t key_code, uint32_t chr)
{
    switch (key_code) {
    case AK_ENTER:    return K_ENTER;
    case AK_BACKSPACE:return K_BACKSPACE;
    case AK_TAB:      return K_TAB;
    case AK_ESCAPE:   return K_ESCAPE;
    case AK_SPACE:    return K_SPACE;
    case AK_UP:       return K_UPARROW;
    case AK_DOWN:     return K_DOWNARROW;
    case AK_LEFT:     return K_LEFTARROW;
    case AK_RIGHT:    return K_RIGHTARROW;
    case AK_DELETE:   return K_DEL;
    case AK_HOME:     return K_HOME;
    case AK_END:      return K_END;
    case AK_PGUP:     return K_PGUP;
    case AK_PGDN:     return K_PGDN;
    case AK_LSHIFT:
    case AK_RSHIFT:   return K_SHIFT;
    case AK_LCTRL:
    case AK_RCTRL:    return K_CTRL;
    case AK_LALT:
    case AK_RALT:     return K_ALT;
    }

    /* F-keys */
    if (key_code >= 0x140 && key_code <= 0x14B)
        return K_F1 + (key_code - 0x140);

    /* Regular ASCII */
    if (chr >= 'A' && chr <= 'Z')
        return chr - 'A' + 'a';
    if (chr >= 'a' && chr <= 'z')
        return chr;
    if (chr >= '0' && chr <= '9')
        return chr;
    if (chr == '-') return '-';
    if (chr == '=') return '=';
    if (chr == '[') return '[';
    if (chr == ']') return ']';
    if (chr == '\\') return '\\';
    if (chr == ';') return ';';
    if (chr == '\'') return '\'';
    if (chr == ',') return ',';
    if (chr == '.') return '.';
    if (chr == '/') return '/';
    if (chr == '`') return '`';

    return 0;
}

/* ── Mouse State ── */
static int mouse_dx, mouse_dy;
static int mouse_buttons;

/* ── Event Pump (called from Sys_SendKeyEvents) ── */

void VID_PumpEvents(void)
{
    uint32_t buf[5];
    int count;

    for (count = 0; count < 64; count++) {
        if (!_syscall(SYS_EVT_CHAN_POLL, g_channel_id, g_sub_id,
                      (int)buf, 0))
            break;

        uint32_t evt = buf[0];

        if (evt == EVT_KEY_DOWN || evt == EVT_KEY_UP) {
            int qkey = translate_key(buf[2], buf[3]);
            if (qkey)
                Key_Event(qkey, evt == EVT_KEY_DOWN);
        }

        if (evt == EVT_MOUSE_MOVE) {
            int16_t dx = (int16_t)(buf[2] & 0xFFFF);
            int16_t dy = (int16_t)(buf[2] >> 16);
            mouse_dx += dx;
            mouse_dy += dy;
        }

        if (evt == EVT_MOUSE_DOWN) {
            int btn = buf[2]; /* 0=left, 1=right, 2=middle */
            if (btn == 0) { mouse_buttons |= 1; Key_Event(K_MOUSE1, true); }
            if (btn == 1) { mouse_buttons |= 2; Key_Event(K_MOUSE2, true); }
            if (btn == 2) { mouse_buttons |= 4; Key_Event(K_MOUSE3, true); }
        }

        if (evt == EVT_MOUSE_UP) {
            int btn = buf[2];
            if (btn == 0) { mouse_buttons &= ~1; Key_Event(K_MOUSE1, false); }
            if (btn == 1) { mouse_buttons &= ~2; Key_Event(K_MOUSE2, false); }
            if (btn == 2) { mouse_buttons &= ~4; Key_Event(K_MOUSE3, false); }
        }

        if (evt == EVT_WINDOW_CLOSE) {
            Sys_Quit();
        }
    }
}

/* ── Video Driver Functions ── */

void VID_SetPalette(unsigned char *palette)
{
    int i;
    for (i = 0; i < 256; i++) {
        unsigned char r = palette[i * 3 + 0];
        unsigned char g = palette[i * 3 + 1];
        unsigned char b = palette[i * 3 + 2];
        g_palette[i] = 0xFF000000 | (r << 16) | (g << 8) | b;
        d_8to24table[i] = g_palette[i];
    }
}

void VID_ShiftPalette(unsigned char *palette)
{
    VID_SetPalette(palette);
}

void VID_Init(unsigned char *palette)
{
    printf("VID_Init: %dx%d for anyOS\n", BASEWIDTH, BASEHEIGHT);

    vid.maxwarpwidth  = vid.width  = vid.conwidth  = BASEWIDTH;
    vid.maxwarpheight = vid.height = vid.conheight  = BASEHEIGHT;
    vid.aspect = ((float)BASEHEIGHT / (float)BASEWIDTH) * (320.0 / 240.0);
    vid.numpages = 1;
    vid.colormap = host_colormap;
    vid.fullbright = 256 - LittleLong(*((int *)vid.colormap + 2048));
    vid.buffer = vid.conbuffer = vid_buffer;
    vid.rowbytes = vid.conrowbytes = BASEWIDTH;

    d_pzbuffer = zbuffer;
    D_InitCaches(surfcache, sizeof(surfcache));

    VID_SetPalette(palette);

    /* ── Connect to anyOS compositor ── */
    static const char chan_name[] = "compositor";
    g_channel_id = _syscall(SYS_EVT_CHAN_CREATE, (int)chan_name,
                            sizeof(chan_name) - 1, 0, 0);
    g_sub_id = _syscall(SYS_EVT_CHAN_SUBSCRIBE, g_channel_id, 0, 0, 0);

    /* Create SHM for window surface */
    uint32_t shm_size = BASEWIDTH * BASEHEIGHT * 4;
    g_shm_id = _syscall(SYS_SHM_CREATE, shm_size, 0, 0, 0);
    uint32_t shm_addr = _syscall(SYS_SHM_MAP, g_shm_id, 0, 0, 0);
    g_surface = (uint32_t *)shm_addr;

    /* Create window */
    uint32_t tid = _syscall(SYS_GETPID, 0, 0, 0, 0);
    uint32_t cmd[5];
    cmd[0] = CMD_CREATE_WINDOW;
    cmd[1] = tid;
    cmd[2] = BASEWIDTH;
    cmd[3] = BASEHEIGHT;
    cmd[4] = (g_shm_id << 16);
    _syscall(SYS_EVT_CHAN_EMIT, g_channel_id, (int)cmd, 0, 0);

    /* Wait for RESP_WINDOW_CREATED */
    uint32_t resp[5];
    int i;
    for (i = 0; i < 200; i++) {
        if (_syscall(SYS_EVT_CHAN_POLL, g_channel_id, g_sub_id,
                     (int)resp, 0)) {
            if (resp[0] == RESP_WINDOW_CREATED && resp[3] == tid) {
                g_window_id = resp[1];
                printf("VID_Init: window created (id=%u)\n", g_window_id);
                break;
            }
        }
        _syscall(SYS_SLEEP, 10, 0, 0, 0);
    }

    /* Set window title */
    {
        uint32_t tc[5];
        tc[0] = CMD_SET_TITLE;
        tc[1] = g_window_id;
        /* "Quake" packed into u32 words */
        tc[2] = 'Q' | ('u' << 8) | ('a' << 16) | ('k' << 24);
        tc[3] = 'e';
        tc[4] = 0;
        _syscall(SYS_EVT_CHAN_EMIT, g_channel_id, (int)tc, 0, 0);
    }

    printf("VID_Init: done\n");
}

void VID_Shutdown(void)
{
    /* Window will be cleaned up by the compositor when the process exits */
}

void VID_Update(vrect_t *rects)
{
    if (!g_surface)
        return;

    /* Convert 8-bit indexed pixels → ARGB in SHM surface */
    /* Only convert dirty rectangles for performance */
    while (rects) {
        int x, y;
        int x0 = rects->x;
        int y0 = rects->y;
        int x1 = x0 + rects->width;
        int y1 = y0 + rects->height;
        if (x0 < 0) x0 = 0;
        if (y0 < 0) y0 = 0;
        if (x1 > BASEWIDTH) x1 = BASEWIDTH;
        if (y1 > BASEHEIGHT) y1 = BASEHEIGHT;

        for (y = y0; y < y1; y++) {
            byte *src = vid_buffer + y * BASEWIDTH + x0;
            uint32_t *dst = g_surface + y * BASEWIDTH + x0;
            int w = x1 - x0;
            for (x = 0; x < w; x++) {
                dst[x] = g_palette[src[x]];
            }
        }
        rects = rects->pnext;
    }

    /* Present to compositor */
    uint32_t cmd[5];
    cmd[0] = CMD_PRESENT;
    cmd[1] = g_window_id;
    cmd[2] = g_shm_id;
    cmd[3] = 0;
    cmd[4] = 0;
    _syscall(SYS_EVT_CHAN_EMIT, g_channel_id, (int)cmd, 0, 0);
}

void D_BeginDirectRect(int x, int y, byte *pbitmap, int width, int height)
{
}

void D_EndDirectRect(int x, int y, int width, int height)
{
}

/* ── Input Functions (in_anyos.c equivalent) ── */

void IN_Init(void)
{
}

void IN_Shutdown(void)
{
}

void IN_Commands(void)
{
}

void IN_Move(usercmd_t *cmd)
{
    /* Apply accumulated mouse movement */
    if (mouse_dx || mouse_dy) {
        cmd->forwardmove -= m_forward.value * mouse_dy;
        cmd->sidemove += m_side.value * mouse_dx;

        cl.viewangles[YAW] -= m_yaw.value * mouse_dx;
        cl.viewangles[PITCH] += m_pitch.value * mouse_dy;

        if (cl.viewangles[PITCH] > 80)
            cl.viewangles[PITCH] = 80;
        if (cl.viewangles[PITCH] < -70)
            cl.viewangles[PITCH] = -70;
    }
    mouse_dx = 0;
    mouse_dy = 0;
}
