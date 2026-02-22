/*
 * doomgeneric_anyos.c — DOOM platform layer for anyOS
 *
 * Implements the 6 doomgeneric platform functions:
 *   DG_Init, DG_DrawFrame, DG_SleepMs, DG_GetTicksMs, DG_GetKey, DG_SetWindowTitle
 *
 * Uses raw syscalls to communicate with the anyOS compositor via:
 *   - Event channels for IPC commands/events
 *   - Shared memory (SHM) for the window pixel buffer
 */

#include "doomgeneric.h"
#include "doomkeys.h"

#include <stdio.h>
#include <string.h>
#include <stdint.h>

/* ── anyOS Syscall Numbers ─────────────────────────────────────────────── */

#define SYS_SLEEP           8
#define SYS_UPTIME          31
#define SYS_TICK_HZ         34
#define SYS_EVT_CHAN_CREATE  63
#define SYS_EVT_CHAN_SUBSCRIBE 64
#define SYS_EVT_CHAN_EMIT   65
#define SYS_EVT_CHAN_POLL   66
#define SYS_SCREEN_SIZE     72
#define SYS_SHM_CREATE      140
#define SYS_SHM_MAP         141
#define SYS_GETPID          6

/* ── IPC Protocol Constants (must match compositor) ────────────────────── */

#define CMD_CREATE_WINDOW   0x1001
#define CMD_PRESENT         0x1003
#define CMD_SET_TITLE       0x1004
#define RESP_WINDOW_CREATED 0x2001

#define EVT_KEY_DOWN        0x3001
#define EVT_KEY_UP          0x3002
#define EVT_WINDOW_CLOSE    0x3007

/* ── Raw Syscall ───────────────────────────────────────────────────────── */

extern int _syscall(int num, int a1, int a2, int a3, int a4);

/* ── Compositor State ──────────────────────────────────────────────────── */

static uint32_t g_channel_id;
static uint32_t g_sub_id;
static uint32_t g_window_id;
static uint32_t g_shm_id;
static uint32_t *g_surface;     /* SHM pixel buffer (DOOM_W * DOOM_H) */

#define DOOM_W DOOMGENERIC_RESX
#define DOOM_H DOOMGENERIC_RESY

/* ── Key Event Queue ───────────────────────────────────────────────────── */

#define KEY_QUEUE_SIZE 32

struct key_event {
    int pressed;
    unsigned char doom_key;
};

static struct key_event g_key_queue[KEY_QUEUE_SIZE];
static int g_key_head;
static int g_key_tail;

static void key_push(int pressed, unsigned char doom_key)
{
    int next = (g_key_head + 1) % KEY_QUEUE_SIZE;
    if (next == g_key_tail) return; /* full */
    g_key_queue[g_key_head].pressed = pressed;
    g_key_queue[g_key_head].doom_key = doom_key;
    g_key_head = next;
}

/* ── anyOS Key Code → DOOM Key Mapping ─────────────────────────────────── */
/*
 * anyOS compositor sends key_code values from keys.rs:
 *   0x100=Enter, 0x101=Backspace, 0x102=Tab, 0x103=Escape, 0x104=Space,
 *   0x105=Up, 0x106=Down, 0x107=Left, 0x108=Right,
 *   0x140-0x14B=F1-F12,
 *   0x120=Delete, 0x121=Home, 0x122=End, 0x123=PgUp, 0x124=PgDn
 * For regular ASCII keys, chr (word[3]) contains the ASCII code.
 */
static unsigned char translate_key(uint32_t key_code, uint32_t chr)
{
    switch (key_code) {
    case 0x100: return KEY_ENTER;
    case 0x101: return KEY_BACKSPACE;
    case 0x102: return KEY_TAB;
    case 0x103: return KEY_ESCAPE;
    case 0x104: return ' ';
    case 0x105: return KEY_UPARROW;
    case 0x106: return KEY_DOWNARROW;
    case 0x107: return KEY_LEFTARROW;
    case 0x108: return KEY_RIGHTARROW;
    case 0x120: return KEY_DEL;
    case 0x121: return KEY_HOME;
    case 0x122: return KEY_END;
    case 0x123: return KEY_PGUP;
    case 0x124: return KEY_PGDN;
    case 0x140: return KEY_F1;
    case 0x141: return KEY_F2;
    case 0x142: return KEY_F3;
    case 0x143: return KEY_F4;
    case 0x144: return KEY_F5;
    case 0x145: return KEY_F6;
    case 0x146: return KEY_F7;
    case 0x147: return KEY_F8;
    case 0x148: return KEY_F9;
    case 0x149: return KEY_F10;
    case 0x14A: return KEY_F11;
    case 0x14B: return KEY_F12;
    default:
        break;
    }

    /* Check modifiers for shift/ctrl/alt */
    /* word[4] = mods:  bit0=shift, bit1=ctrl, bit2=alt */

    /* Regular ASCII character */
    if (chr >= 'a' && chr <= 'z') return (unsigned char)chr;
    if (chr >= 'A' && chr <= 'Z') return (unsigned char)(chr - 'A' + 'a');
    if (chr >= '0' && chr <= '9') return (unsigned char)chr;
    if (chr == '-') return KEY_MINUS;
    if (chr == '=') return KEY_EQUALS;

    /* Modifier-only keys detected via scancode ranges */
    /* Left/Right Shift = scancode 0x2A/0x36 → key_code passes through */
    if (key_code == 0x2A || key_code == 0x36) return KEY_RSHIFT;
    /* Left/Right Ctrl */
    if (key_code == 0x1D) return KEY_RCTRL;
    /* Left/Right Alt */
    if (key_code == 0x38) return KEY_RALT;

    return 0;
}

/* ── Poll Compositor Events ────────────────────────────────────────────── */

static void poll_events(void)
{
    uint32_t buf[5];
    int count;

    for (count = 0; count < 32; count++) {
        if (!_syscall(SYS_EVT_CHAN_POLL, g_channel_id, g_sub_id,
                      (int)buf, 0))
            break;

        uint32_t evt_type = buf[0];
        /* uint32_t target_wid = buf[1]; */

        if (evt_type == EVT_KEY_DOWN || evt_type == EVT_KEY_UP) {
            uint32_t key_code = buf[2];
            uint32_t chr = buf[3];
            unsigned char dk = translate_key(key_code, chr);
            if (dk != 0) {
                key_push(evt_type == EVT_KEY_DOWN ? 1 : 0, dk);
            }
        }
    }
}

/* ── Platform Functions ────────────────────────────────────────────────── */

void DG_Init(void)
{
    printf("DG_Init: connecting to compositor...\n");

    /* Connect to the compositor event channel */
    static const char name[] = "compositor";
    g_channel_id = _syscall(SYS_EVT_CHAN_CREATE, (int)name,
                            (int)sizeof(name) - 1, 0, 0);
    printf("DG_Init: channel_id=%u\n", (unsigned)g_channel_id);

    g_sub_id = _syscall(SYS_EVT_CHAN_SUBSCRIBE, g_channel_id, 0, 0, 0);
    printf("DG_Init: sub_id=%u\n", (unsigned)g_sub_id);

    /* Create SHM for the window surface */
    uint32_t shm_size = DOOM_W * DOOM_H * 4;
    g_shm_id = _syscall(SYS_SHM_CREATE, shm_size, 0, 0, 0);
    printf("DG_Init: shm_id=%u (size=%u)\n", (unsigned)g_shm_id, (unsigned)shm_size);

    uint32_t shm_addr = _syscall(SYS_SHM_MAP, g_shm_id, 0, 0, 0);
    printf("DG_Init: shm_addr=0x%x\n", (unsigned)shm_addr);
    g_surface = (uint32_t *)shm_addr;

    /* Send CMD_CREATE_WINDOW (with window chrome + scale-on-resize) */
    uint32_t tid = _syscall(SYS_GETPID, 0, 0, 0, 0);
    printf("DG_Init: tid=%u\n", (unsigned)tid);

    #define WIN_FLAG_SCALE_CONTENT 0x80
    #define CW_USEDEFAULT 0xFFFF
    uint32_t cmd[5];
    cmd[0] = CMD_CREATE_WINDOW;
    cmd[1] = tid;
    cmd[2] = (DOOM_W << 16) | (DOOM_H & 0xFFFF);         /* packed w|h */
    cmd[3] = (CW_USEDEFAULT << 16) | CW_USEDEFAULT;       /* auto-place */
    cmd[4] = (g_shm_id << 16) | WIN_FLAG_SCALE_CONTENT;
    printf("DG_Init: sending CMD_CREATE_WINDOW [%x %u %x %x %x]\n",
           cmd[0], cmd[1], cmd[2], cmd[3], cmd[4]);

    _syscall(SYS_EVT_CHAN_EMIT, g_channel_id, (int)cmd, 0, 0);

    /* Wait for RESP_WINDOW_CREATED */
    uint32_t resp[5];
    int i;
    for (i = 0; i < 100; i++) {
        if (_syscall(SYS_EVT_CHAN_POLL, g_channel_id, g_sub_id,
                     (int)resp, 0)) {
            printf("DG_Init: poll got [%x %u %u %u %u]\n",
                   resp[0], resp[1], resp[2], resp[3], resp[4]);
            if (resp[0] == RESP_WINDOW_CREATED && resp[3] == tid) {
                g_window_id = resp[1];
                printf("DG_Init: window created! id=%u\n", g_window_id);
                break;
            }
        }
        _syscall(SYS_SLEEP, 10, 0, 0, 0);
    }
    if (g_window_id == 0) {
        printf("DG_Init: WARNING - failed to create window!\n");
    }
}

void DG_DrawFrame(void)
{
    /* Copy DG_ScreenBuffer to SHM surface */
    if (g_surface && DG_ScreenBuffer) {
        memcpy(g_surface, DG_ScreenBuffer, DOOM_W * DOOM_H * 4);
    }

    /* Send CMD_PRESENT to compositor */
    uint32_t cmd[5];
    cmd[0] = CMD_PRESENT;
    cmd[1] = g_window_id;
    cmd[2] = g_shm_id;
    cmd[3] = 0;
    cmd[4] = 0;
    _syscall(SYS_EVT_CHAN_EMIT, g_channel_id, (int)cmd, 0, 0);

    /* Poll input events after each frame */
    poll_events();
}

void DG_SleepMs(uint32_t ms)
{
    if (ms > 0) {
        _syscall(SYS_SLEEP, ms, 0, 0, 0);
    }
}

uint32_t DG_GetTicksMs(void)
{
    /* SYS_UPTIME returns PIT ticks; SYS_TICK_HZ gives the rate in Hz */
    uint32_t ticks = (uint32_t)_syscall(SYS_UPTIME, 0, 0, 0, 0);
    uint32_t hz = (uint32_t)_syscall(SYS_TICK_HZ, 0, 0, 0, 0);
    if (hz == 0) hz = 1000;
    return ticks * 1000 / hz;
}

int DG_GetKey(int *pressed, unsigned char *doom_key)
{
    /* Also poll events here in case DG_DrawFrame hasn't been called recently */
    poll_events();

    if (g_key_tail == g_key_head)
        return 0;

    *pressed = g_key_queue[g_key_tail].pressed;
    *doom_key = g_key_queue[g_key_tail].doom_key;
    g_key_tail = (g_key_tail + 1) % KEY_QUEUE_SIZE;
    return 1;
}

int main(int argc, char **argv)
{
    doomgeneric_Create(argc, argv);
    while (1) {
        doomgeneric_Tick();
    }
    return 0;
}

void DG_SetWindowTitle(const char *title)
{
    /* Pack title bytes into 3 u32 words (max 12 chars) */
    uint32_t packed[3] = {0, 0, 0};
    int len = 0;
    while (title[len] && len < 12) len++;
    int i;
    for (i = 0; i < len; i++) {
        packed[i / 4] |= ((uint32_t)(unsigned char)title[i]) << ((i % 4) * 8);
    }

    uint32_t cmd[5];
    cmd[0] = CMD_SET_TITLE;
    cmd[1] = g_window_id;
    cmd[2] = packed[0];
    cmd[3] = packed[1];
    cmd[4] = packed[2];
    _syscall(SYS_EVT_CHAN_EMIT, g_channel_id, (int)cmd, 0, 0);
}
