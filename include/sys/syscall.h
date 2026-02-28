/*
 * anyOS syscall numbers — SINGLE SOURCE OF TRUTH for C code.
 *
 * Rust source of truth: kernel/src/syscall/mod.rs
 * Keep both in sync!
 *
 * Used by libc (32-bit) and libc64 (64-bit) via symlink:
 *   libs/libc/include/sys/syscall.h   -> ../../../include/sys/syscall.h
 *   libs/libc64/include/sys/syscall.h -> ../../../include/sys/syscall.h
 */

#ifndef _SYS_SYSCALL_H
#define _SYS_SYSCALL_H

/* ---- Process management ---- */
#define SYS_EXIT             1
#define SYS_WRITE            2
#define SYS_READ             3
#define SYS_OPEN             4
#define SYS_CLOSE            5
#define SYS_GETPID           6
#define SYS_YIELD            7
#define SYS_SLEEP            8
#define SYS_SBRK             9
#define SYS_FORK            10
#define SYS_EXEC            11
#define SYS_WAITPID         12
#define SYS_KILL            13
#define SYS_MMAP            14
#define SYS_MUNMAP          15

/* ---- Device management ---- */
#define SYS_DEVLIST         16
#define SYS_DEVOPEN         17
#define SYS_DEVCLOSE        18
#define SYS_DEVREAD         19
#define SYS_DEVWRITE        20
#define SYS_DEVIOCTL        21
#define SYS_IRQWAIT         22

/* ---- Filesystem (basic) ---- */
#define SYS_READDIR         23
#define SYS_STAT            24
#define SYS_GETCWD          25
#define SYS_CHDIR           26

/* ---- Process spawning ---- */
#define SYS_SPAWN           27
#define SYS_GETARGS         28
#define SYS_TRY_WAITPID     29

/* ---- System information ---- */
#define SYS_TIME            30
#define SYS_UPTIME          31
#define SYS_SYSINFO         32
#define SYS_DMESG           33
#define SYS_TICK_HZ         34
#define SYS_UPTIME_MS       35

/* ---- Networking (general) ---- */
#define SYS_NET_CONFIG      40
#define SYS_NET_PING        41
#define SYS_NET_DHCP        42
#define SYS_NET_DNS         43
#define SYS_NET_ARP         44

/* ---- Pipes (named IPC) ---- */
#define SYS_PIPE_CREATE     45
#define SYS_PIPE_READ       46
#define SYS_PIPE_CLOSE      47
#define SYS_PIPE_WRITE      48
#define SYS_PIPE_OPEN       49

/* ---- Network poll ---- */
#define SYS_NET_POLL        50

/* ---- Event bus ---- */
#define SYS_EVT_SYS_SUBSCRIBE   60
#define SYS_EVT_SYS_POLL        61
#define SYS_EVT_SYS_UNSUBSCRIBE 62
#define SYS_EVT_CHAN_CREATE      63
#define SYS_EVT_CHAN_SUBSCRIBE   64
#define SYS_EVT_CHAN_EMIT        65
#define SYS_EVT_CHAN_POLL        66
#define SYS_EVT_CHAN_UNSUBSCRIBE 67
#define SYS_EVT_CHAN_DESTROY     68
#define SYS_EVT_CHAN_EMIT_TO     69
#define SYS_EVT_CHAN_WAIT        70

/* ---- Display ---- */
#define SYS_SCREEN_SIZE     72

/* ---- DLL loading ---- */
#define SYS_DLL_LOAD        80

/* ---- Filesystem (extended) ---- */
#define SYS_MKDIR           90
#define SYS_UNLINK          91
#define SYS_TRUNCATE        92

/* ---- Mount/unmount ---- */
#define SYS_MOUNT           93
#define SYS_UMOUNT          94
#define SYS_LIST_MOUNTS     95

/* ---- Symlinks ---- */
#define SYS_SYMLINK         96
#define SYS_READLINK        97
#define SYS_LSTAT           98

/* ---- Filesystem (POSIX-like) ---- */
#define SYS_RENAME          99

/* ---- TCP networking ---- */
#define SYS_TCP_CONNECT    100
#define SYS_TCP_SEND       101
#define SYS_TCP_RECV       102
#define SYS_TCP_CLOSE      103
#define SYS_TCP_STATUS     104

/* ---- File I/O (POSIX) ---- */
#define SYS_LSEEK          105
#define SYS_FSTAT          106
#define SYS_FTRUNCATE      107
#define SYS_ISATTY         108

/* ---- Display (resolution) ---- */
#define SYS_SET_RESOLUTION  110
#define SYS_LIST_RESOLUTIONS 111
#define SYS_GPU_INFO        112

/* ---- Audio ---- */
#define SYS_AUDIO_WRITE    120
#define SYS_AUDIO_CTL      121

/* ---- TCP networking (extended) ---- */
#define SYS_TCP_RECV_AVAILABLE 130
#define SYS_TCP_SHUTDOWN_WR    131
#define SYS_TCP_LISTEN         132
#define SYS_TCP_ACCEPT         133
#define SYS_TCP_LIST           134

/* ---- GPU acceleration ---- */
#define SYS_GPU_HAS_ACCEL      135
#define SYS_BOOT_READY         137
#define SYS_GPU_HAS_HW_CURSOR  138

/* ---- Shared memory ---- */
#define SYS_SHM_CREATE     140
#define SYS_SHM_MAP        141
#define SYS_SHM_UNMAP      142
#define SYS_SHM_DESTROY    143

/* ---- Compositor ---- */
#define SYS_MAP_FRAMEBUFFER     144
#define SYS_GPU_COMMAND         145
#define SYS_INPUT_POLL          146
#define SYS_REGISTER_COMPOSITOR 147
#define SYS_CURSOR_TAKEOVER     148

/* ---- UDP networking ---- */
#define SYS_UDP_BIND       150
#define SYS_UDP_UNBIND     151
#define SYS_UDP_SENDTO     152
#define SYS_UDP_RECVFROM   153
#define SYS_UDP_SET_OPT    154
#define SYS_UDP_LIST       155
#define SYS_NET_STATS      156
#define SYS_PIPE_BYTES_AVAILABLE 157

/* ---- Screen capture ---- */
#define SYS_CAPTURE_SCREEN 161

/* ---- Threading ---- */
#define SYS_THREAD_CREATE  170
#define SYS_SET_PRIORITY   171
#define SYS_SET_CRITICAL   172

/* ---- Pipe listing ---- */
#define SYS_PIPE_LIST      180

/* ---- Environment variables ---- */
#define SYS_SETENV         182
#define SYS_GETENV         183
#define SYS_LISTENV        184

/* ---- DLL configuration ---- */
#define SYS_SET_DLL_U32    190

/* ---- Keyboard layout ---- */
#define SYS_KBD_GET_LAYOUT   200
#define SYS_KBD_SET_LAYOUT   201
#define SYS_KBD_LIST_LAYOUTS 202

/* ---- Random ---- */
#define SYS_RANDOM         210

/* ---- Security / Users / Permissions ---- */
#define SYS_GET_CAPABILITIES 220
#define SYS_GETUID           221
#define SYS_GETGID           222
#define SYS_AUTHENTICATE     223
#define SYS_CHMOD            224
#define SYS_CHOWN            225
#define SYS_ADDUSER          226
#define SYS_DELUSER          227
#define SYS_LISTUSERS        228
#define SYS_ADDGROUP         229
#define SYS_DELGROUP         230
#define SYS_LISTGROUPS       231
#define SYS_GETUSERNAME      232
#define SYS_SET_IDENTITY     233
#define SYS_CHPASSWD         234

/* ---- POSIX FD operations ---- */
#define SYS_PIPE2          240
#define SYS_DUP            241
#define SYS_DUP2           242
#define SYS_FCNTL          243

/* ---- Signals ---- */
#define SYS_SIGACTION      244
#define SYS_SIGPROCMASK    245
#define SYS_SIGRETURN      246

/* ---- Process info ---- */
#define SYS_GETPPID        247

/* ---- Permissions subsystem ---- */
#define SYS_PERM_CHECK        250
#define SYS_PERM_STORE        251
#define SYS_PERM_LIST         252
#define SYS_PERM_DELETE       253
#define SYS_PERM_PENDING_INFO 254

/* ---- GPU VRAM / backbuffer ---- */
#define SYS_GPU_VRAM_SIZE          256
#define SYS_VRAM_MAP               257
#define SYS_GPU_REGISTER_BACKBUFFER 258

/* ---- Crash info ---- */
#define SYS_GET_CRASH_INFO 260

/* ---- Disk / partition management ---- */
#define SYS_DISK_LIST          270
#define SYS_DISK_PARTITIONS    271
#define SYS_DISK_READ          272
#define SYS_DISK_WRITE         273
#define SYS_PARTITION_CREATE   274
#define SYS_PARTITION_DELETE   275
#define SYS_PARTITION_RESCAN   276

/* ---- Hostname ---- */
#define SYS_GET_HOSTNAME   280
#define SYS_SET_HOSTNAME   281

/* ---- System control ---- */
#define SYS_SHUTDOWN       282

/* ---- GPU 3D acceleration ---- */
#define SYS_GPU_3D_SUBMIT          512
#define SYS_GPU_3D_QUERY           513
#define SYS_GPU_3D_SYNC            514
#define SYS_GPU_3D_SURFACE_DMA     515
#define SYS_GPU_3D_SURFACE_DMA_READ 516

#endif /* _SYS_SYSCALL_H */
