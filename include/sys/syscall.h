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
#define SYS_SLEEP_US        36
#define SYS_SBRK             9
#define SYS_FORK            10
#define SYS_EXEC            11
#define SYS_WAITPID         12
#define SYS_KILL            13
#define SYS_MMAP            14
#define SYS_MUNMAP          15
#define SYS_MMAP64         631
#define SYS_MUNMAP64       632
#define SYS_LICOF_SPAWN    710

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
#define SYS_SET_TIME        37

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

/* ---- Monitor detection ---- */
#define SYS_MONITOR_COUNT   327
#define SYS_MONITOR_INFO    328
#define SYS_MONITOR_EDID    329
#define SYS_MONITOR_MODES   330

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
#define SYS_WIFI           158
#define SYS_NET_PING6      159
#define SYS_NET_DNS6       160

/* ---- Screen capture ---- */
#define SYS_CAPTURE_SCREEN 161
#define SYS_UDP_SENDTO_V6  162
#define SYS_TCP_CONNECT_V6 163
#define SYS_UDP_RECVFROM_V6 164
#define SYS_TCP_ACCEPT_V6  165

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

/* ---- Hardware virtualization (VT-x / AMD-V) ---- */
/* VM lifecycle */
#define SYS_VM_CREATE          600
#define SYS_VM_DESTROY         601
#define SYS_VM_SET_MEMORY      602
#define SYS_VM_SET_CPUID       612
#define SYS_VM_HW_INFO         613  /* returns 0=none, 1=VMX, 2=SVM */
#define SYS_VM_GET_DIRTY_LOG   614  /* arg1=vm_id, arg2=ptr to DirtyLogRequest */
/* vCPU lifecycle */
#define SYS_VCPU_CREATE        603
#define SYS_VCPU_RUN           604  /* blocks until VM-exit; arg3=ptr to VmExitInfo */
#define SYS_VCPU_PAUSE         615
#define SYS_VCPU_RESUME        616
/* vCPU register access */
#define SYS_VCPU_GET_REGS      605  /* arg3=ptr to GuestGprs */
#define SYS_VCPU_SET_REGS      606
#define SYS_VCPU_GET_SREGS     607  /* arg3=ptr to GuestSregs */
#define SYS_VCPU_SET_SREGS     608
#define SYS_VCPU_GET_FPU       617  /* arg3=ptr to GuestFpuState (512 B, FXSAVE) */
#define SYS_VCPU_SET_FPU       618
/* vCPU event injection */
#define SYS_VCPU_INJECT_IRQ        609  /* arg3=vector */
#define SYS_VCPU_INJECT_EXCEPTION  610  /* arg3=(vector|error_code<<8) */
#define SYS_VCPU_INJECT_NMI        611
/* vCPU multi-processor state */
#define SYS_VCPU_GET_MP_STATE  619  /* returns VcpuMpState (0=run,1=uninit,2=halt,3=init) */
#define SYS_VCPU_SET_MP_STATE  620  /* arg3=VcpuMpState */
/* GVA translation */
#define SYS_VCPU_TRANSLATE     621  /* arg3=ptr to TranslateRequest */

/*
 * Shared data structures (C layout — must match Rust #[repr(C)] types in virt/mod.rs)
 */
#ifndef __ANYOS_VIRT_STRUCTS
#define __ANYOS_VIRT_STRUCTS
#include <stdint.h>

/* VmExitInfo — returned by SYS_VCPU_RUN */
typedef struct {
    uint32_t reason;          /* exit reason code (see EXIT_REASON_* / VMEXIT_* below) */
    uint32_t _pad0;
    uint64_t qualification;   /* exit qualification / exit_info1 */
    uint64_t guest_phys_addr; /* for EPT/NPF violations */
    uint32_t instruction_len; /* bytes of faulting instruction */
    uint16_t io_port;         /* I/O port (I/O exits) */
    uint8_t  access_size;     /* 1/2/4/8 bytes */
    uint8_t  is_read;         /* 0=write/out, 1=read/in */
    uint64_t io_data;         /* OUT data / MMIO write value / CPUID EAX input */
    uint64_t io_data2;        /* CPUID ECX index / WRMSR value high */
    uint32_t msr_index;       /* MSR number (RDMSR/WRMSR exits) */
    uint32_t cpuid_function;  /* CPUID leaf EAX */
    uint32_t cpuid_index;     /* CPUID subleaf ECX */
    uint32_t _pad1;
} VmExitInfo;

/* Exit reason codes (portable — same values used for VMX and SVM) */
#define VM_EXIT_EXCEPTION          0   /* guest exception / NMI */
#define VM_EXIT_EXTERNAL_INTERRUPT 1   /* host interrupt while in guest */
#define VM_EXIT_TRIPLE_FAULT       2   /* guest triple-fault */
#define VM_EXIT_CPUID             10   /* CPUID instruction */
#define VM_EXIT_HLT               12   /* HLT instruction */
#define VM_EXIT_VMCALL            18   /* VMCALL / VMMCALL (hypercall) */
#define VM_EXIT_IO                30   /* IN/OUT instruction */
#define VM_EXIT_RDMSR             31   /* RDMSR */
#define VM_EXIT_WRMSR             32   /* WRMSR */
#define VM_EXIT_SHUTDOWN          0x7F /* guest triple-fault / shutdown (SVM 0x7F) */
#define VM_EXIT_EPT_VIOLATION     48   /* EPT/NPT page not present or permission fault */
#define VM_EXIT_EPT_MISCONFIG     49   /* EPT/NPT misconfiguration */
#define VM_EXIT_MMIO              0x1000 /* synthetic: MMIO access (userspace-defined) */

/* GuestGprs — general-purpose registers */
typedef struct {
    uint64_t rax, rbx, rcx, rdx;
    uint64_t rsi, rdi, rbp;
    uint64_t r8, r9, r10, r11, r12, r13, r14, r15;
} GuestGprs;

/* GuestSregs — segment and control registers */
typedef struct {
    uint16_t cs_selector;  uint16_t _p0; uint32_t cs_limit;  uint32_t cs_ar;  uint32_t _p1;
    uint64_t cs_base;
    uint16_t ds_selector;  uint16_t _p2; uint32_t ds_limit;  uint32_t ds_ar;  uint32_t _p3;
    uint64_t ds_base;
    uint16_t es_selector;  uint16_t _p4; uint32_t es_limit;  uint32_t es_ar;  uint32_t _p5;
    uint64_t es_base;
    uint16_t fs_selector;  uint16_t _p6; uint32_t fs_limit;  uint32_t fs_ar;  uint32_t _p7;
    uint64_t fs_base;
    uint16_t gs_selector;  uint16_t _p8; uint32_t gs_limit;  uint32_t gs_ar;  uint32_t _p9;
    uint64_t gs_base;
    uint16_t ss_selector;  uint16_t _pa; uint32_t ss_limit;  uint32_t ss_ar;  uint32_t _pb;
    uint64_t ss_base;
    uint16_t tr_selector;  uint16_t _pc; uint32_t tr_limit;  uint32_t tr_ar;  uint32_t _pd;
    uint64_t tr_base;
    uint16_t ldtr_selector; uint16_t _pe; uint32_t ldtr_limit; uint32_t ldtr_ar; uint32_t _pf;
    uint64_t ldtr_base;
    uint64_t gdtr_base;    uint32_t gdtr_limit; uint32_t _pg;
    uint64_t idtr_base;    uint32_t idtr_limit; uint32_t _ph;
    uint64_t cr0, cr3, cr4, efer;
    uint64_t rip, rsp, rflags;
} GuestSregs;

/* GuestFpuState — 512-byte FXSAVE layout (16-byte aligned) */
typedef struct __attribute__((aligned(16))) {
    uint8_t data[512];
} GuestFpuState;

/* VcpuMpState — multi-processor state */
#define VCPU_MP_RUNNABLE    0
#define VCPU_MP_UNINITIALIZED 1
#define VCPU_MP_HALTED      2
#define VCPU_MP_INIT_RECEIVED 3

/* TranslateRequest — for SYS_VCPU_TRANSLATE */
typedef struct {
    uint64_t gva;       /* input: guest virtual address */
    uint64_t out_gpa;   /* output: guest physical address */
    uint32_t out_valid; /* output: 1 if translation succeeded */
    uint32_t _pad;
} TranslateRequest;

/* MemRegionDesc — for SYS_VM_SET_MEMORY */
typedef struct {
    uint64_t guest_phys; /* guest physical base address */
    uint64_t size;       /* size in bytes (must be page-aligned) */
    uint64_t host_phys;  /* host virtual address of the backing buffer */
} MemRegionDesc;

/* DirtyLogRequest — for SYS_VM_GET_DIRTY_LOG */
typedef struct {
    uint32_t slot;       /* memory slot index */
    uint32_t _pad;
    uint64_t bitmap_ptr; /* pointer to caller-allocated bitmap */
    uint64_t bitmap_size;/* size of bitmap in bytes */
} DirtyLogRequest;

/* CpuidEntry — for SYS_VM_SET_CPUID */
typedef struct {
    uint32_t function;
    uint32_t index;
    uint32_t eax, ebx, ecx, edx;
} CpuidEntry;

#endif /* __ANYOS_VIRT_STRUCTS */

#endif /* _SYS_SYSCALL_H */
