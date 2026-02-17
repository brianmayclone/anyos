/*
 * sys_anyos.c — Quake system layer for anyOS
 *
 * Implements Sys_* functions using anyOS libc (fopen/fread/etc.)
 * and raw syscalls for timing.
 */

#include "quakedef.h"
#include "errno.h"

/* anyOS syscall numbers */
#define SYS_EXIT    1
#define SYS_SLEEP   8
#define SYS_UPTIME  31
#define SYS_TICK_HZ 34

extern int _syscall(int num, int a1, int a2, int a3, int a4);

qboolean isDedicated = false;

/* ====================================================================
   FILE I/O — uses standard libc fopen/fread/fseek
   ==================================================================== */

#define MAX_HANDLES 32
static FILE *sys_handles[MAX_HANDLES];

static int findhandle(void)
{
    int i;
    for (i = 1; i < MAX_HANDLES; i++)
        if (!sys_handles[i])
            return i;
    Sys_Error("out of handles");
    return -1;
}

static int filelength_f(FILE *f)
{
    int pos, end;
    pos = ftell(f);
    fseek(f, 0, SEEK_END);
    end = ftell(f);
    fseek(f, pos, SEEK_SET);
    return end;
}

int Sys_FileOpenRead(char *path, int *hndl)
{
    FILE *f;
    int i;

    i = findhandle();
    f = fopen(path, "rb");
    if (!f) {
        *hndl = -1;
        return -1;
    }
    sys_handles[i] = f;
    *hndl = i;
    return filelength_f(f);
}

int Sys_FileOpenWrite(char *path)
{
    FILE *f;
    int i;

    i = findhandle();
    f = fopen(path, "wb");
    if (!f)
        Sys_Error("Error opening %s", path);
    sys_handles[i] = f;
    return i;
}

void Sys_FileClose(int handle)
{
    if (handle >= 0 && handle < MAX_HANDLES && sys_handles[handle]) {
        fclose(sys_handles[handle]);
        sys_handles[handle] = NULL;
    }
}

void Sys_FileSeek(int handle, int position)
{
    fseek(sys_handles[handle], position, SEEK_SET);
}

int Sys_FileRead(int handle, void *dest, int count)
{
    return fread(dest, 1, count, sys_handles[handle]);
}

int Sys_FileWrite(int handle, void *data, int count)
{
    return fwrite(data, 1, count, sys_handles[handle]);
}

int Sys_FileTime(char *path)
{
    FILE *f;
    f = fopen(path, "rb");
    if (f) {
        fclose(f);
        return 1;
    }
    return -1;
}

void Sys_mkdir(char *path)
{
    /* No mkdir syscall yet — no-op */
}

/* ====================================================================
   SYSTEM
   ==================================================================== */

void Sys_MakeCodeWriteable(unsigned long startaddr, unsigned long length)
{
    /* Not needed — flat memory model */
}

void Sys_Error(char *error, ...)
{
    va_list argptr;
    char buf[1024];

    va_start(argptr, error);
    vprintf(error, argptr);
    va_end(argptr);
    printf("\n");

    Host_Shutdown();
    _syscall(SYS_EXIT, 1, 0, 0, 0);
    while (1) {}
}

void Sys_Printf(char *fmt, ...)
{
    va_list argptr;
    va_start(argptr, fmt);
    vprintf(fmt, argptr);
    va_end(argptr);
}

void Sys_Quit(void)
{
    Host_Shutdown();
    _syscall(SYS_EXIT, 0, 0, 0, 0);
    while (1) {}
}

double Sys_FloatTime(void)
{
    unsigned int ticks = (unsigned int)_syscall(SYS_UPTIME, 0, 0, 0, 0);
    unsigned int hz = (unsigned int)_syscall(SYS_TICK_HZ, 0, 0, 0, 0);
    if (hz == 0) hz = 1000;
    return (double)ticks / (double)hz;
}

char *Sys_ConsoleInput(void)
{
    return NULL;
}

void Sys_Sleep(void)
{
    _syscall(SYS_SLEEP, 1, 0, 0, 0);
}

/* Keyboard events are pumped from vid_anyos.c */
extern void VID_PumpEvents(void);

void Sys_SendKeyEvents(void)
{
    VID_PumpEvents();
}

void Sys_HighFPPrecision(void)
{
}

void Sys_LowFPPrecision(void)
{
}

/* ====================================================================
   MAIN
   ==================================================================== */

int main(int argc, char **argv)
{
    static quakeparms_t parms;
    double time, oldtime, newtime;

    parms.memsize = 16 * 1024 * 1024; /* 16 MB */
    parms.membase = malloc(parms.memsize);
    if (!parms.membase) {
        printf("Quake: failed to allocate %d bytes\n", parms.memsize);
        return 1;
    }
    parms.basedir = "/apps/quake";

    COM_InitArgv(argc, argv);
    parms.argc = com_argc;
    parms.argv = com_argv;

    printf("Host_Init\n");
    Host_Init(&parms);

    oldtime = Sys_FloatTime();
    while (1) {
        newtime = Sys_FloatTime();
        time = newtime - oldtime;
        if (time < 0.001)
            time = 0.001;
        oldtime = newtime;
        Host_Frame(time);
    }

    return 0;
}
