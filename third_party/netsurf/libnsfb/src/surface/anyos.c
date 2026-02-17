/*
 * anyOS surface backend for libnsfb
 *
 * Uses the anyOS compositor IPC protocol (event channels + shared memory)
 * to create a window and blit pixels, same approach as DOOM.
 */

#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

#include "libnsfb.h"
#include "libnsfb_plot.h"
#include "libnsfb_event.h"

#include "nsfb.h"
#include "surface.h"
#include "plot.h"

/* anyOS syscall interface */
extern int _syscall(int num, int a1, int a2, int a3, int a4);

/* Syscall numbers */
#define SYS_SLEEP           8
#define SYS_UPTIME          31
#define SYS_GETPID          6
#define SYS_EVT_CHAN_CREATE  63
#define SYS_EVT_CHAN_SUBSCRIBE 64
#define SYS_EVT_CHAN_EMIT   65
#define SYS_EVT_CHAN_POLL   66
#define SYS_SHM_CREATE      140
#define SYS_SHM_MAP         141

/* Compositor IPC commands */
#define CMD_CREATE_WINDOW   0x1001
#define CMD_PRESENT         0x1003
#define CMD_SET_TITLE       0x1004
#define RESP_WINDOW_CREATED 0x2001

/* Input events from compositor */
#define EVT_KEY_DOWN        0x3001
#define EVT_KEY_UP          0x3002
#define EVT_RESIZE          0x3003
#define EVT_MOUSE_DOWN      0x3004
#define EVT_MOUSE_UP        0x3005
#define EVT_MOUSE_MOVE      0x3006
#define EVT_MOUSE_SCROLL    0x3007
#define EVT_WINDOW_CLOSE    0x3008

/* Private surface data */
typedef struct {
    uint32_t channel_id;
    uint32_t sub_id;
    uint32_t window_id;
    uint32_t shm_id;
    uint32_t *shm_surface;  /* SHM pixel buffer mapped into our address space */
    int mouse_x;
    int mouse_y;
} anyos_priv_t;


static int anyos_defaults(nsfb_t *nsfb)
{
    nsfb->width = 800;
    nsfb->height = 600;
    nsfb->format = NSFB_FMT_XRGB8888;

    select_plotters(nsfb);

    return 0;
}

static int anyos_initialise(nsfb_t *nsfb)
{
    anyos_priv_t *priv;
    uint32_t shm_size;
    uint32_t tid;
    uint32_t cmd[5];
    uint32_t resp[5];
    int i;

    fprintf(stderr, "[browser] anyos_initialise: %dx%d\n", nsfb->width, nsfb->height);

    priv = calloc(1, sizeof(anyos_priv_t));
    if (priv == NULL)
        return -1;

    nsfb->surface_priv = priv;

    /* 1. Create event channel named "compositor" */
    {
        static const char name[] = "compositor";
        priv->channel_id = (uint32_t)_syscall(SYS_EVT_CHAN_CREATE,
            (int)name, sizeof(name) - 1, 0, 0);
    }
    fprintf(stderr, "[browser] channel_id=%u\n", priv->channel_id);

    /* 2. Subscribe to compositor events */
    priv->sub_id = (uint32_t)_syscall(SYS_EVT_CHAN_SUBSCRIBE,
        (int)priv->channel_id, 0, 0, 0);
    fprintf(stderr, "[browser] sub_id=%u\n", priv->sub_id);

    /* 3. Create shared memory for pixel buffer */
    shm_size = (uint32_t)(nsfb->width * nsfb->height * 4);
    priv->shm_id = (uint32_t)_syscall(SYS_SHM_CREATE, (int)shm_size, 0, 0, 0);
    fprintf(stderr, "[browser] shm_id=%u (size=%u)\n", priv->shm_id, shm_size);
    if (priv->shm_id == 0) {
        fprintf(stderr, "[browser] SHM create FAILED\n");
        free(priv);
        return -1;
    }

    /* 4. Map SHM into our address space */
    priv->shm_surface = (uint32_t *)(uintptr_t)_syscall(SYS_SHM_MAP,
        (int)priv->shm_id, 0, 0, 0);
    fprintf(stderr, "[browser] shm_surface=%p\n", (void *)priv->shm_surface);
    if (priv->shm_surface == NULL) {
        fprintf(stderr, "[browser] SHM map FAILED\n");
        free(priv);
        return -1;
    }

    /* 5. Send CMD_CREATE_WINDOW */
    tid = (uint32_t)_syscall(SYS_GETPID, 0, 0, 0, 0);
    cmd[0] = CMD_CREATE_WINDOW;
    cmd[1] = tid;
    cmd[2] = (uint32_t)nsfb->width;
    cmd[3] = (uint32_t)nsfb->height;
    cmd[4] = priv->shm_id << 16; /* flags = 0 (normal window with chrome) */
    _syscall(SYS_EVT_CHAN_EMIT, (int)priv->channel_id, (int)cmd, 0, 0);
    fprintf(stderr, "[browser] CMD_CREATE_WINDOW sent (tid=%u)\n", tid);

    /* 6. Poll for RESP_WINDOW_CREATED */
    priv->window_id = 0;
    for (i = 0; i < 200; i++) {
        if (_syscall(SYS_EVT_CHAN_POLL, (int)priv->channel_id,
                     (int)priv->sub_id, (int)resp, 0)) {
            if (resp[0] == RESP_WINDOW_CREATED && resp[3] == tid) {
                priv->window_id = resp[1];
                break;
            }
        }
        _syscall(SYS_SLEEP, 10, 0, 0, 0);
    }
    fprintf(stderr, "[browser] window_id=%u (polled %d times)\n", priv->window_id, i);

    if (priv->window_id == 0) {
        fprintf(stderr, "[browser] window creation FAILED\n");
        free(priv);
        return -1;
    }

    /* Set window title */
    cmd[0] = CMD_SET_TITLE;
    cmd[1] = priv->window_id;
    {
        static const char title[] = "Browser";
        cmd[2] = (uint32_t)(uintptr_t)title;
        cmd[3] = sizeof(title) - 1;
    }
    cmd[4] = 0;
    _syscall(SYS_EVT_CHAN_EMIT, (int)priv->channel_id, (int)cmd, 0, 0);

    /* Allocate local rendering buffer (libnsfb renders here) */
    nsfb->ptr = malloc((size_t)(nsfb->width * nsfb->height * 4));
    if (nsfb->ptr == NULL) {
        fprintf(stderr, "[browser] render buffer malloc FAILED\n");
        free(priv);
        return -1;
    }
    memset(nsfb->ptr, 0xFF, (size_t)(nsfb->width * nsfb->height * 4));
    nsfb->linelen = nsfb->width * 4;

    fprintf(stderr, "[browser] anyos_initialise OK (buf=%p)\n", (void *)nsfb->ptr);
    return 0;
}


static int anyos_finalise(nsfb_t *nsfb)
{
    /* TODO: send window destroy command */
    free(nsfb->ptr);
    free(nsfb->surface_priv);
    return 0;
}


static int anyos_set_geometry(nsfb_t *nsfb, int width, int height,
                              enum nsfb_format_e format)
{
    if (width > 0)
        nsfb->width = width;
    if (height > 0)
        nsfb->height = height;
    if (format != NSFB_FMT_ANY)
        nsfb->format = format;

    select_plotters(nsfb);

    nsfb->linelen = nsfb->width * (nsfb->bpp / 8);

    return 0;
}


/* Translate anyOS keycode to nsfb keycode */
static enum nsfb_key_code_e translate_key(uint32_t anyos_key)
{
    /* anyOS uses ASCII-compatible keycodes for printable chars */
    if (anyos_key >= 32 && anyos_key <= 126)
        return (enum nsfb_key_code_e)anyos_key;

    switch (anyos_key) {
    case 8:   return NSFB_KEY_BACKSPACE;
    case 9:   return NSFB_KEY_TAB;
    case 13:  return NSFB_KEY_RETURN;
    case 27:  return NSFB_KEY_ESCAPE;
    case 127: return NSFB_KEY_DELETE;
    /* Arrow keys — anyOS codes may differ, adjust as needed */
    case 0x48: return NSFB_KEY_UP;
    case 0x50: return NSFB_KEY_DOWN;
    case 0x4D: return NSFB_KEY_RIGHT;
    case 0x4B: return NSFB_KEY_LEFT;
    default:   return NSFB_KEY_UNKNOWN;
    }
}


static bool anyos_input(nsfb_t *nsfb, nsfb_event_t *event, int timeout)
{
    anyos_priv_t *priv = nsfb->surface_priv;
    uint32_t evt[5];
    int waited = 0;

    if (priv == NULL)
        return false;

    while (1) {
        if (_syscall(SYS_EVT_CHAN_POLL, (int)priv->channel_id,
                     (int)priv->sub_id, (int)evt, 0)) {

            switch (evt[0]) {
            case EVT_KEY_DOWN:
                event->type = NSFB_EVENT_KEY_DOWN;
                event->value.keycode = translate_key(evt[2]);
                return true;

            case EVT_KEY_UP:
                event->type = NSFB_EVENT_KEY_UP;
                event->value.keycode = translate_key(evt[2]);
                return true;

            case EVT_MOUSE_MOVE:
                event->type = NSFB_EVENT_MOVE_ABSOLUTE;
                event->value.vector.x = (int)evt[1];
                event->value.vector.y = (int)evt[2];
                event->value.vector.z = 0;
                priv->mouse_x = (int)evt[1];
                priv->mouse_y = (int)evt[2];
                return true;

            case EVT_MOUSE_DOWN:
                event->type = NSFB_EVENT_KEY_DOWN;
                event->value.keycode = NSFB_KEY_MOUSE_1 + (int)(evt[3] - 1);
                return true;

            case EVT_MOUSE_UP:
                event->type = NSFB_EVENT_KEY_UP;
                event->value.keycode = NSFB_KEY_MOUSE_1 + (int)(evt[3] - 1);
                return true;

            case EVT_MOUSE_SCROLL:
                event->type = NSFB_EVENT_KEY_DOWN;
                if ((int)evt[1] < 0)
                    event->value.keycode = NSFB_KEY_MOUSE_4; /* scroll up */
                else
                    event->value.keycode = NSFB_KEY_MOUSE_5; /* scroll down */
                return true;

            case EVT_WINDOW_CLOSE:
                event->type = NSFB_EVENT_CONTROL;
                event->value.controlcode = NSFB_CONTROL_QUIT;
                return true;

            case EVT_RESIZE:
                event->type = NSFB_EVENT_RESIZE;
                event->value.resize.w = (int)evt[1];
                event->value.resize.h = (int)evt[2];
                return true;

            default:
                /* Unknown event, skip */
                break;
            }
        }

        /* No event available */
        if (timeout == 0)
            return false;
        if (timeout > 0 && waited >= timeout)
            return false;

        _syscall(SYS_SLEEP, 10, 0, 0, 0);
        waited += 10;

        if (timeout < 0) {
            /* Wait forever — keep looping */
            continue;
        }
    }
}


static int anyos_claim(nsfb_t *nsfb, nsfb_bbox_t *box)
{
    (void)nsfb;
    (void)box;
    return 0;
}


static int update_count = 0;

static int anyos_update(nsfb_t *nsfb, nsfb_bbox_t *box)
{
    anyos_priv_t *priv = nsfb->surface_priv;
    uint32_t cmd[5];

    update_count++;
    if (update_count <= 5 || (update_count % 100) == 0)
        fprintf(stderr, "[browser] anyos_update #%d box=(%d,%d)-(%d,%d)\n",
                update_count, box->x0, box->y0, box->x1, box->y1);

    if (priv == NULL || priv->shm_surface == NULL)
        return -1;

    /* Copy the dirty region from local buffer to SHM surface */
    int x0 = box->x0 < 0 ? 0 : box->x0;
    int y0 = box->y0 < 0 ? 0 : box->y0;
    int x1 = box->x1 > nsfb->width ? nsfb->width : box->x1;
    int y1 = box->y1 > nsfb->height ? nsfb->height : box->y1;

    if (x0 >= x1 || y0 >= y1)
        return 0;

    /* Row-by-row copy */
    int row_bytes = (x1 - x0) * 4;
    for (int y = y0; y < y1; y++) {
        uint8_t *src = nsfb->ptr + y * nsfb->linelen + x0 * 4;
        uint8_t *dst = (uint8_t *)priv->shm_surface + y * nsfb->width * 4 + x0 * 4;
        memcpy(dst, src, (size_t)row_bytes);
    }

    /* Tell compositor to present */
    cmd[0] = CMD_PRESENT;
    cmd[1] = priv->window_id;
    cmd[2] = priv->shm_id;
    cmd[3] = 0;
    cmd[4] = 0;
    _syscall(SYS_EVT_CHAN_EMIT, (int)priv->channel_id, (int)cmd, 0, 0);

    return 0;
}


static int anyos_cursor(nsfb_t *nsfb, struct nsfb_cursor_s *cursor)
{
    (void)nsfb;
    (void)cursor;
    /* Compositor handles cursor rendering */
    return 0;
}


const nsfb_surface_rtns_t anyos_rtns = {
    .defaults = anyos_defaults,
    .initialise = anyos_initialise,
    .finalise = anyos_finalise,
    .input = anyos_input,
    .geometry = anyos_set_geometry,
    .claim = anyos_claim,
    .update = anyos_update,
    .cursor = anyos_cursor,
};

NSFB_SURFACE_DEF(anyos, NSFB_SURFACE_ABLE, &anyos_rtns)
