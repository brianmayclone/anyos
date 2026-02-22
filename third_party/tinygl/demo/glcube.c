/*
 * glcube.c — TinyGL spinning cube demo for anyOS
 *
 * Demonstrates software OpenGL rendering via TinyGL.
 * Creates a compositor window and renders a lit, colored cube
 * that rotates continuously.
 */

#include <GL/gl.h>
#include <zbuffer.h>
#include <stdio.h>
#include <string.h>
#include <stdint.h>
#include <math.h>

/* ── anyOS Syscall Numbers ─────────────────────────────────────────────── */

#define SYS_EXIT            1
#define SYS_YIELD           7
#define SYS_SLEEP           8
#define SYS_UPTIME          31
#define SYS_TICK_HZ         34
#define SYS_EVT_CHAN_CREATE  63
#define SYS_EVT_CHAN_SUBSCRIBE 64
#define SYS_EVT_CHAN_EMIT   65
#define SYS_EVT_CHAN_POLL   66
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

/* ── Constants ─────────────────────────────────────────────────────────── */

#define WIN_W  320
#define WIN_H  240
#define WIN_FLAG_SCALE_CONTENT 0x80
#define CW_USEDEFAULT 0xFFFF

/* ── Compositor State ──────────────────────────────────────────────────── */

static uint32_t g_channel_id;
static uint32_t g_sub_id;
static uint32_t g_window_id;
static uint32_t g_shm_id;
static uint32_t *g_surface;
static int g_running = 1;

/* ── TinyGL State ──────────────────────────────────────────────────────── */

static ZBuffer *g_zb;
static float g_angle = 0.0f;

/* ── FPS Counter ──────────────────────────────────────────────────────── */

static uint32_t g_tick_hz;
static uint32_t g_last_time;
static uint32_t g_frame_count;
static uint32_t g_fps;

/* 5x7 bitmap font for "0123456789FPS:. " */
static const uint8_t font_5x7[][7] = {
    /* '0' */ {0x0E,0x11,0x13,0x15,0x19,0x11,0x0E},
    /* '1' */ {0x04,0x0C,0x04,0x04,0x04,0x04,0x0E},
    /* '2' */ {0x0E,0x11,0x01,0x06,0x08,0x10,0x1F},
    /* '3' */ {0x0E,0x11,0x01,0x06,0x01,0x11,0x0E},
    /* '4' */ {0x02,0x06,0x0A,0x12,0x1F,0x02,0x02},
    /* '5' */ {0x1F,0x10,0x1E,0x01,0x01,0x11,0x0E},
    /* '6' */ {0x06,0x08,0x10,0x1E,0x11,0x11,0x0E},
    /* '7' */ {0x1F,0x01,0x02,0x04,0x08,0x08,0x08},
    /* '8' */ {0x0E,0x11,0x11,0x0E,0x11,0x11,0x0E},
    /* '9' */ {0x0E,0x11,0x11,0x0F,0x01,0x02,0x0C},
    /* 'F' */ {0x1F,0x10,0x10,0x1E,0x10,0x10,0x10},
    /* 'P' */ {0x1E,0x11,0x11,0x1E,0x10,0x10,0x10},
    /* 'S' */ {0x0E,0x11,0x10,0x0E,0x01,0x11,0x0E},
    /* ':' */ {0x00,0x04,0x04,0x00,0x04,0x04,0x00},
    /* ' ' */ {0x00,0x00,0x00,0x00,0x00,0x00,0x00},
};

static int font_index(char c)
{
    if (c >= '0' && c <= '9') return c - '0';
    if (c == 'F') return 10;
    if (c == 'P') return 11;
    if (c == 'S') return 12;
    if (c == ':') return 13;
    return 14; /* space */
}

static void draw_char(int x, int y, char c, uint32_t color)
{
    int idx = font_index(c);
    int row, col;
    for (row = 0; row < 7; row++) {
        uint8_t bits = font_5x7[idx][row];
        for (col = 0; col < 5; col++) {
            if (bits & (0x10 >> col)) {
                int px = x + col, py = y + row;
                if (px >= 0 && px < WIN_W && py >= 0 && py < WIN_H)
                    g_surface[py * WIN_W + px] = color;
            }
        }
    }
}

static void draw_string(int x, int y, const char *s, uint32_t color)
{
    while (*s) {
        draw_char(x, y, *s, color);
        x += 6;
        s++;
    }
}

static void draw_fps_overlay(void)
{
    char buf[16];
    int i = 0;
    uint32_t f = g_fps;

    buf[i++] = 'F';
    buf[i++] = 'P';
    buf[i++] = 'S';
    buf[i++] = ':';
    buf[i++] = ' ';

    if (f >= 100) { buf[i++] = '0' + (f / 100) % 10; }
    if (f >= 10)  { buf[i++] = '0' + (f / 10) % 10; }
    buf[i++] = '0' + f % 10;
    buf[i] = 0;

    /* Draw with black outline for readability */
    draw_string(9, 9, buf, 0xFF000000);
    draw_string(8, 8, buf, 0xFF00FF00);
}

/* ── Compositor Setup (identical pattern to DOOM/Quake) ─────────────── */

static void init_window(void)
{
    static const char name[] = "compositor";
    g_channel_id = _syscall(SYS_EVT_CHAN_CREATE, (int)name,
                            (int)sizeof(name) - 1, 0, 0);
    g_sub_id = _syscall(SYS_EVT_CHAN_SUBSCRIBE, g_channel_id, 0, 0, 0);

    uint32_t shm_size = WIN_W * WIN_H * 4;
    g_shm_id = _syscall(SYS_SHM_CREATE, shm_size, 0, 0, 0);
    uint32_t shm_addr = _syscall(SYS_SHM_MAP, g_shm_id, 0, 0, 0);
    g_surface = (uint32_t *)shm_addr;

    uint32_t tid = _syscall(SYS_GETPID, 0, 0, 0, 0);

    uint32_t cmd[5];
    cmd[0] = CMD_CREATE_WINDOW;
    cmd[1] = tid;
    cmd[2] = (WIN_W << 16) | (WIN_H & 0xFFFF);
    cmd[3] = (CW_USEDEFAULT << 16) | CW_USEDEFAULT;
    cmd[4] = (g_shm_id << 16) | WIN_FLAG_SCALE_CONTENT;
    _syscall(SYS_EVT_CHAN_EMIT, g_channel_id, (int)cmd, 0, 0);

    /* Wait for window creation response */
    uint32_t resp[5];
    int i;
    for (i = 0; i < 100; i++) {
        if (_syscall(SYS_EVT_CHAN_POLL, g_channel_id, g_sub_id,
                     (int)resp, 0)) {
            if (resp[0] == RESP_WINDOW_CREATED && resp[3] == tid) {
                g_window_id = resp[1];
                break;
            }
        }
        _syscall(SYS_SLEEP, 10, 0, 0, 0);
    }

    /* Set window title */
    cmd[0] = CMD_SET_TITLE;
    cmd[1] = g_window_id;
    cmd[2] = 'G' | ('L' << 8) | ('C' << 16) | ('u' << 24);
    cmd[3] = 'b' | ('e' << 8);
    cmd[4] = 0;
    _syscall(SYS_EVT_CHAN_EMIT, g_channel_id, (int)cmd, 0, 0);
}

static void poll_events(void)
{
    uint32_t buf[5];
    int count;
    for (count = 0; count < 16; count++) {
        if (!_syscall(SYS_EVT_CHAN_POLL, g_channel_id, g_sub_id,
                      (int)buf, 0))
            break;

        if (buf[0] == EVT_KEY_DOWN) {
            uint32_t key_code = buf[2];
            if (key_code == 0x103) /* Escape */
                g_running = 0;
        }
        if (buf[0] == EVT_WINDOW_CLOSE) {
            g_running = 0;
        }
    }
}

static void present_frame(void)
{
    uint32_t cmd[5];
    cmd[0] = CMD_PRESENT;
    cmd[1] = g_window_id;
    cmd[2] = g_shm_id;
    cmd[3] = 0;
    cmd[4] = 0;
    _syscall(SYS_EVT_CHAN_EMIT, g_channel_id, (int)cmd, 0, 0);
}

/* ── OpenGL Setup ──────────────────────────────────────────────────────── */

static void init_gl(void)
{
    /* Use TinyGL internal buffer — copy to SHM atomically after render */
    g_zb = ZB_open(WIN_W, WIN_H, ZB_MODE_RGBA, NULL);
    glInit(g_zb);

    glViewport(0, 0, WIN_W, WIN_H);

    /* Set up perspective projection using glFrustum */
    glMatrixMode(GL_PROJECTION);
    glLoadIdentity();
    {
        float fov = 60.0f;
        float aspect = (float)WIN_W / (float)WIN_H;
        float near = 0.1f;
        float far = 100.0f;
        float top = (float)(near * tan((double)fov * 3.14159265 / 360.0));
        float bottom = -top;
        float right = top * aspect;
        float left = -right;
        glFrustum(left, right, bottom, top, near, far);
    }

    glMatrixMode(GL_MODELVIEW);
    glLoadIdentity();

    /* Enable features */
    glEnable(GL_DEPTH_TEST);
    glEnable(GL_CULL_FACE);
    glEnable(GL_LIGHTING);
    glEnable(GL_LIGHT0);

    /* Light setup */
    {
        GLfloat pos[] = { 3.0f, 3.0f, 3.0f, 1.0f };
        GLfloat amb[] = { 0.2f, 0.2f, 0.2f, 1.0f };
        GLfloat dif[] = { 1.0f, 1.0f, 1.0f, 1.0f };
        glLightfv(GL_LIGHT0, GL_POSITION, pos);
        glLightfv(GL_LIGHT0, GL_AMBIENT, amb);
        glLightfv(GL_LIGHT0, GL_DIFFUSE, dif);
    }

    glEnable(GL_COLOR_MATERIAL);
    glClearColor(0.1f, 0.1f, 0.15f, 1.0f);
}

/* ── Cube Rendering ────────────────────────────────────────────────────── */

static void draw_cube_face(float nx, float ny, float nz,
                           float v0x, float v0y, float v0z,
                           float v1x, float v1y, float v1z,
                           float v2x, float v2y, float v2z,
                           float v3x, float v3y, float v3z)
{
    glNormal3f(nx, ny, nz);
    glVertex3f(v0x, v0y, v0z);
    glVertex3f(v1x, v1y, v1z);
    glVertex3f(v2x, v2y, v2z);
    glVertex3f(v3x, v3y, v3z);
}

static void render_frame(void)
{
    glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);

    glMatrixMode(GL_MODELVIEW);
    glLoadIdentity();
    glTranslatef(0.0f, 0.0f, -4.0f);
    glRotatef(g_angle, 1.0f, 0.7f, 0.3f);

    glBegin(GL_QUADS);

    /* Front face - red */
    glColor3f(0.9f, 0.2f, 0.2f);
    draw_cube_face(0, 0, 1,
        -1, -1, 1,  1, -1, 1,  1, 1, 1,  -1, 1, 1);

    /* Back face - green */
    glColor3f(0.2f, 0.9f, 0.2f);
    draw_cube_face(0, 0, -1,
        1, -1, -1,  -1, -1, -1,  -1, 1, -1,  1, 1, -1);

    /* Top face - blue */
    glColor3f(0.2f, 0.4f, 0.9f);
    draw_cube_face(0, 1, 0,
        -1, 1, 1,  1, 1, 1,  1, 1, -1,  -1, 1, -1);

    /* Bottom face - yellow */
    glColor3f(0.9f, 0.9f, 0.2f);
    draw_cube_face(0, -1, 0,
        -1, -1, -1,  1, -1, -1,  1, -1, 1,  -1, -1, 1);

    /* Right face - magenta */
    glColor3f(0.9f, 0.2f, 0.9f);
    draw_cube_face(1, 0, 0,
        1, -1, 1,  1, -1, -1,  1, 1, -1,  1, 1, 1);

    /* Left face - cyan */
    glColor3f(0.2f, 0.9f, 0.9f);
    draw_cube_face(-1, 0, 0,
        -1, -1, -1,  -1, -1, 1,  -1, 1, 1,  -1, 1, -1);

    glEnd();

    /* Copy TinyGL internal buffer to SHM with alpha=0xFF in a single pass.
     * This is atomic from the compositor's perspective — SHM always contains
     * a complete frame, never a half-rendered one. */
    {
        const uint32_t *src = (const uint32_t *)g_zb->pbuf;
        int i, n = WIN_W * WIN_H;
        for (i = 0; i < n; i++)
            g_surface[i] = src[i] | 0xFF000000;
    }

    g_angle += 1.0f;
    if (g_angle >= 360.0f)
        g_angle -= 360.0f;
}

/* ── Main ──────────────────────────────────────────────────────────────── */

int main(int argc, char **argv)
{
    printf("GLCube: TinyGL demo starting...\n");

    init_window();
    if (g_window_id == 0) {
        printf("GLCube: failed to create window\n");
        _syscall(SYS_EXIT, 1, 0, 0, 0);
    }

    init_gl();
    printf("GLCube: OpenGL initialized, rendering...\n");

    g_tick_hz = _syscall(SYS_TICK_HZ, 0, 0, 0, 0);
    g_last_time = _syscall(SYS_UPTIME, 0, 0, 0, 0);
    g_frame_count = 0;
    g_fps = 0;

    while (g_running) {
        render_frame();         /* renders into TinyGL internal buffer, copies to SHM */
        draw_fps_overlay();     /* draws directly into SHM (already complete) */
        present_frame();        /* tells compositor: SHM is ready */
        _syscall(SYS_SLEEP, 8, 0, 0, 0); /* SHM stays valid while compositor reads it */
        poll_events();

        g_frame_count++;
        {
            uint32_t now = _syscall(SYS_UPTIME, 0, 0, 0, 0);
            uint32_t elapsed = now - g_last_time;
            if (elapsed >= g_tick_hz) {
                g_fps = (g_frame_count * g_tick_hz) / elapsed;
                printf("GLCube: %u FPS\n", (unsigned)g_fps);
                g_frame_count = 0;
                g_last_time = now;
            }
        }
    }

    glClose();
    ZB_close(g_zb);

    printf("GLCube: exiting\n");
    _syscall(SYS_EXIT, 0, 0, 0, 0);
    return 0;
}
