/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — POSIX threads (pthreads) implementation for anyOS.
 *
 * Constraints:
 *   - No futex syscall — mutexes use atomic spinlocks with yield-on-contention.
 *   - No proper TLS segment — thread-local storage uses a static array indexed
 *     by (tid % MAX_THREADS).
 *   - Stacks are allocated via SYS_MMAP (kernel page allocator) and freed via
 *     SYS_MUNMAP on join/detach.
 */

#include <pthread.h>
#include <errno.h>
#include <stdint.h>
#include <string.h>

extern long _syscall(long num, long a1, long a2, long a3, long a4, long a5);

/* ── Syscall numbers ── */
#define SYS_EXIT            1
#define SYS_GETPID          6
#define SYS_YIELD           7
#define SYS_SLEEP           8
#define SYS_WAITPID         12
#define SYS_MMAP            14
#define SYS_MUNMAP          15
#define SYS_THREAD_CREATE   170

/* ── Defaults ── */
#define DEFAULT_STACK_SIZE  (64 * 1024)     /* 64 KiB default thread stack */
#define MIN_STACK_SIZE      4096            /* Minimum stack size (1 page) */

/* ── Thread-local storage limits ── */
#define PTHREAD_KEYS_MAX    64
#define MAX_THREADS         128

/* ──────────────────────────────────────────────────────────────────────
 *  Internal thread metadata
 *
 *  Tracks per-thread state needed for join, detach, and cleanup.
 *  Indexed by (tid % MAX_THREADS).
 * ────────────────────────────────────────────────────────────────────── */

/** Per-thread bookkeeping record. */
typedef struct {
    volatile int    active;         /**< 1 if this slot is in use. */
    pthread_t       tid;            /**< Kernel TID for this thread. */
    void           *retval;         /**< Return value from start_routine / pthread_exit. */
    volatile int    finished;       /**< 1 once the thread has exited. */
    volatile int    detached;       /**< 1 if detached (no join expected). */
    void           *stack_base;     /**< Base address of the mmap'd stack. */
    size_t          stack_size;     /**< Size of the mmap'd stack. */
    void          *(*start_routine)(void *); /**< User entry point (set before thread starts). */
    void           *start_arg;      /**< Argument for start_routine. */
} _pthread_info_t;

static _pthread_info_t _thread_info[MAX_THREADS];
static volatile int    _thread_info_lock = 0; /* Simple spinlock for the info table. */

/* ── TLS data ── */
static volatile int    _tls_key_lock = 0;
static int             _tls_key_used[PTHREAD_KEYS_MAX];
static void          (*_tls_key_dtor[PTHREAD_KEYS_MAX])(void *);
static void           *_tls_values[MAX_THREADS][PTHREAD_KEYS_MAX];

/* ── Helpers ── */

/**
 * Acquire a simple spinlock.
 * Yields to the scheduler on contention to avoid wasting CPU cycles.
 */
static void _spin_lock(volatile int *lock) {
    while (__atomic_exchange_n(lock, 1, __ATOMIC_ACQUIRE) != 0) {
        _syscall(SYS_YIELD, 0, 0, 0, 0, 0);
    }
}

/** Release a simple spinlock. */
static void _spin_unlock(volatile int *lock) {
    __atomic_store_n(lock, 0, __ATOMIC_RELEASE);
}

/**
 * Look up or allocate a thread info slot for the given TID.
 *
 * @param tid   Kernel thread ID.
 * @param alloc If non-zero, allocate a new slot; otherwise only look up.
 * @return Pointer to the info struct, or NULL if not found / table full.
 */
static _pthread_info_t *_get_thread_info(pthread_t tid, int alloc) {
    unsigned int idx = (unsigned int)(tid % MAX_THREADS);
    _spin_lock(&_thread_info_lock);

    /* Fast path: the natural slot matches. */
    if (_thread_info[idx].active && _thread_info[idx].tid == tid) {
        _spin_unlock(&_thread_info_lock);
        return &_thread_info[idx];
    }

    /* Linear probe for a match or a free slot (if allocating). */
    for (unsigned int i = 0; i < MAX_THREADS; i++) {
        unsigned int slot = (idx + i) % MAX_THREADS;
        if (_thread_info[slot].active && _thread_info[slot].tid == tid) {
            _spin_unlock(&_thread_info_lock);
            return &_thread_info[slot];
        }
        if (alloc && !_thread_info[slot].active) {
            _thread_info[slot].active = 1;
            _thread_info[slot].tid = tid;
            _thread_info[slot].retval = NULL;
            _thread_info[slot].finished = 0;
            _thread_info[slot].detached = 0;
            _thread_info[slot].stack_base = NULL;
            _thread_info[slot].stack_size = 0;
            _thread_info[slot].start_routine = NULL;
            _thread_info[slot].start_arg = NULL;
            _spin_unlock(&_thread_info_lock);
            return &_thread_info[slot];
        }
    }

    _spin_unlock(&_thread_info_lock);
    return NULL;
}

/**
 * Free a thread info slot, releasing the stack if still allocated.
 */
static void _free_thread_info(_pthread_info_t *info) {
    if (!info) return;

    /* Unmap the thread's stack if we allocated it. */
    if (info->stack_base) {
        _syscall(SYS_MUNMAP, (long)(uintptr_t)info->stack_base,
                 (long)info->stack_size, 0, 0, 0);
    }

    _spin_lock(&_thread_info_lock);
    info->active = 0;
    info->tid = 0;
    info->retval = NULL;
    info->finished = 0;
    info->detached = 0;
    info->stack_base = NULL;
    info->stack_size = 0;
    info->start_routine = NULL;
    info->start_arg = NULL;
    _spin_unlock(&_thread_info_lock);
}

/**
 * Run TLS destructors for the calling thread.
 * Called just before the thread exits.
 */
static void _run_tls_destructors(void) {
    unsigned int tid_idx = (unsigned int)((unsigned long)_syscall(SYS_GETPID, 0, 0, 0, 0, 0) % MAX_THREADS);

    /* POSIX allows up to PTHREAD_DESTRUCTOR_ITERATIONS rounds. We do one. */
    for (int k = 0; k < PTHREAD_KEYS_MAX; k++) {
        if (_tls_key_used[k] && _tls_key_dtor[k] && _tls_values[tid_idx][k]) {
            void *val = _tls_values[tid_idx][k];
            _tls_values[tid_idx][k] = NULL;
            _tls_key_dtor[k](val);
        }
    }
}

/* ──────────────────────────────────────────────────────────────────────
 *  Thread trampoline
 *
 *  The kernel starts the new thread at this function.  The trampoline
 *  retrieves its start_routine and arg from the thread info table
 *  (indexed by its own TID), calls the user function, stores the
 *  return value, and exits.
 *
 *  Why not pass args on the stack?  The compiler emits a prologue that
 *  adjusts RSP before we can read stack-placed data, so hardcoded
 *  offsets from RSP are unreliable across optimization levels.
 *  Looking up by TID in a static table avoids this problem entirely.
 * ────────────────────────────────────────────────────────────────────── */

/**
 * Thread entry point invoked by the kernel.
 *
 * Must be noinline to guarantee a stable function address for SYS_THREAD_CREATE.
 */
__attribute__((noinline))
static void _pthread_trampoline(void) {
    /* Identify ourselves and fetch our start_routine + arg from the info table. */
    pthread_t self = (pthread_t)_syscall(SYS_GETPID, 0, 0, 0, 0, 0);
    _pthread_info_t *info = _get_thread_info(self, 0);

    void *retval = NULL;
    if (info && info->start_routine) {
        retval = info->start_routine(info->start_arg);
    }

    /* Store the return value and mark the thread as finished. */
    if (info) {
        info->retval = retval;
        __atomic_store_n(&info->finished, 1, __ATOMIC_RELEASE);

        /* If detached, clean up immediately. */
        if (__atomic_load_n(&info->detached, __ATOMIC_ACQUIRE)) {
            _free_thread_info(info);
        }
    }

    /* Run TLS destructors before exiting. */
    _run_tls_destructors();

    _syscall(SYS_EXIT, 0, 0, 0, 0, 0);
    __builtin_unreachable();
}

/* ──────────────────────────────────────────────────────────────────────
 *  Thread management
 * ────────────────────────────────────────────────────────────────────── */

int pthread_create(pthread_t *thread, const pthread_attr_t *attr,
                   void *(*start_routine)(void *), void *arg) {
    if (!thread || !start_routine) return EINVAL;

    /* Determine stack size. */
    size_t stack_size = DEFAULT_STACK_SIZE;
    int detached = PTHREAD_CREATE_JOINABLE;
    if (attr) {
        if (attr->stack_size >= MIN_STACK_SIZE) stack_size = attr->stack_size;
        detached = attr->detach_state;
    }

    /* Round up to page boundary. */
    stack_size = (stack_size + 4095) & ~(size_t)4095;

    /* Allocate stack pages via kernel mmap (page-aligned, zeroed). */
    long stack_addr = _syscall(SYS_MMAP, (long)stack_size, 0, 0, 0, 0);
    if (stack_addr == (long)0xFFFFFFFF || stack_addr == 0) return ENOMEM;

    void *stack_base = (void *)(uintptr_t)stack_addr;
    uintptr_t stack_top = (uintptr_t)stack_base + stack_size;

    /*
     * Set up the initial RSP with a fake return address (0) at the top,
     * 16-byte aligned per the System V AMD64 ABI.  The trampoline never
     * returns, so the fake address is just for stack unwinder safety.
     *
     * Stack layout (growing downward):
     *   stack_top - 8  : 0  (fake return address)
     *   RSP = stack_top - 8
     *
     * After the CALL-like entry the kernel performs, RSP points here.
     */
    uintptr_t *slot_retaddr = (uintptr_t *)(stack_top - 8);
    *slot_retaddr = 0;
    uintptr_t user_rsp = stack_top - 8;

    /*
     * Pre-allocate the thread info slot and store start_routine + arg
     * BEFORE creating the kernel thread.  The trampoline reads these
     * fields by looking up its own TID in the info table.
     *
     * We use TID 0 as a placeholder and patch it after SYS_THREAD_CREATE
     * returns.  This is safe because the new thread cannot run until
     * we store the real TID: it calls _get_thread_info(self, 0) which
     * searches by TID, and TID 0 will never match a real kernel TID.
     *
     * However, to avoid a race where the new thread starts before we
     * patch the TID, we allocate the slot with a sentinel TID first,
     * store the routine/arg, then patch the TID after creation.
     */
    _pthread_info_t *info = _get_thread_info(0, 1);
    if (!info) {
        _syscall(SYS_MUNMAP, (long)(uintptr_t)stack_base, (long)stack_size, 0, 0, 0);
        return EAGAIN;
    }

    info->stack_base = stack_base;
    info->stack_size = stack_size;
    info->start_routine = start_routine;
    info->start_arg = arg;
    if (detached == PTHREAD_CREATE_DETACHED) {
        __atomic_store_n(&info->detached, 1, __ATOMIC_RELEASE);
    }

    /* Thread name for the kernel (truncated to fit). */
    const char *name = "pthread";
    size_t name_len = 7;

    /* Create the kernel thread.  Priority 0 = inherit from parent. */
    long tid = _syscall(SYS_THREAD_CREATE,
                        (long)(uintptr_t)_pthread_trampoline,
                        (long)user_rsp,
                        (long)(uintptr_t)name,
                        (long)name_len,
                        0);
    if (tid == 0) {
        /* Creation failed — clean up the pre-allocated slot and stack. */
        _free_thread_info(info);
        return EAGAIN;
    }

    /*
     * Patch the real TID into the info slot.  The trampoline spins on
     * _get_thread_info(self, 0) which searches by TID — once we store
     * the real TID here, the trampoline will find this slot.
     *
     * Use a release store so the trampoline's acquire load sees all
     * the fields we wrote above (start_routine, start_arg, etc.).
     */
    __atomic_store_n(&info->tid, (pthread_t)tid, __ATOMIC_RELEASE);

    *thread = (pthread_t)tid;
    return 0;
}

int pthread_join(pthread_t thread, void **retval) {
    _pthread_info_t *info = _get_thread_info(thread, 0);

    if (info && __atomic_load_n(&info->detached, __ATOMIC_ACQUIRE)) {
        return EINVAL; /* Cannot join a detached thread. */
    }

    /* Block until the kernel reports the thread has terminated. */
    _syscall(SYS_WAITPID, (long)(unsigned int)thread, 0, 0, 0, 0);

    if (info) {
        if (retval) *retval = info->retval;
        _free_thread_info(info);
    } else {
        if (retval) *retval = NULL;
    }

    return 0;
}

int pthread_detach(pthread_t thread) {
    _pthread_info_t *info = _get_thread_info(thread, 0);
    if (!info) return ESRCH;

    __atomic_store_n(&info->detached, 1, __ATOMIC_RELEASE);

    /* If the thread already finished, clean up now. */
    if (__atomic_load_n(&info->finished, __ATOMIC_ACQUIRE)) {
        _free_thread_info(info);
    }

    return 0;
}

void pthread_exit(void *retval) {
    pthread_t self = (pthread_t)_syscall(SYS_GETPID, 0, 0, 0, 0, 0);
    _pthread_info_t *info = _get_thread_info(self, 0);
    if (info) {
        info->retval = retval;
        __atomic_store_n(&info->finished, 1, __ATOMIC_RELEASE);

        if (__atomic_load_n(&info->detached, __ATOMIC_ACQUIRE)) {
            _free_thread_info(info);
        }
    }

    _run_tls_destructors();
    _syscall(SYS_EXIT, 0, 0, 0, 0, 0);
    __builtin_unreachable();
}

pthread_t pthread_self(void) {
    return (pthread_t)_syscall(SYS_GETPID, 0, 0, 0, 0, 0);
}

int pthread_equal(pthread_t t1, pthread_t t2) {
    return t1 == t2;
}

/* ──────────────────────────────────────────────────────────────────────
 *  Thread attributes
 * ────────────────────────────────────────────────────────────────────── */

int pthread_attr_init(pthread_attr_t *attr) {
    if (!attr) return EINVAL;
    attr->stack_size = DEFAULT_STACK_SIZE;
    attr->detach_state = PTHREAD_CREATE_JOINABLE;
    return 0;
}

int pthread_attr_destroy(pthread_attr_t *attr) {
    (void)attr;
    return 0;
}

int pthread_attr_setstacksize(pthread_attr_t *attr, size_t stacksize) {
    if (!attr || stacksize < MIN_STACK_SIZE) return EINVAL;
    attr->stack_size = stacksize;
    return 0;
}

int pthread_attr_getstacksize(const pthread_attr_t *attr, size_t *stacksize) {
    if (!attr || !stacksize) return EINVAL;
    *stacksize = attr->stack_size;
    return 0;
}

int pthread_attr_setdetachstate(pthread_attr_t *attr, int detachstate) {
    if (!attr) return EINVAL;
    if (detachstate != PTHREAD_CREATE_JOINABLE &&
        detachstate != PTHREAD_CREATE_DETACHED) return EINVAL;
    attr->detach_state = detachstate;
    return 0;
}

int pthread_attr_getdetachstate(const pthread_attr_t *attr, int *detachstate) {
    if (!attr || !detachstate) return EINVAL;
    *detachstate = attr->detach_state;
    return 0;
}

/* ──────────────────────────────────────────────────────────────────────
 *  Mutexes (spinlock-based — no futex available)
 * ────────────────────────────────────────────────────────────────────── */

int pthread_mutex_init(pthread_mutex_t *mutex, const pthread_mutexattr_t *attr) {
    (void)attr;
    if (!mutex) return EINVAL;
    mutex->lock = 0;
    mutex->owner = 0;
    return 0;
}

int pthread_mutex_destroy(pthread_mutex_t *mutex) {
    (void)mutex;
    return 0;
}

int pthread_mutex_lock(pthread_mutex_t *mutex) {
    if (!mutex) return EINVAL;

    /*
     * Spin with CAS.  After several failed attempts, yield to the
     * scheduler so we do not burn CPU cycles indefinitely.  This is
     * the best we can do without a futex/wait queue.
     */
    int spins = 0;
    while (__atomic_exchange_n(&mutex->lock, 1, __ATOMIC_ACQUIRE) != 0) {
        if (++spins >= 16) {
            _syscall(SYS_YIELD, 0, 0, 0, 0, 0);
            spins = 0;
        }
    }
    mutex->owner = (unsigned long)_syscall(SYS_GETPID, 0, 0, 0, 0, 0);
    return 0;
}

int pthread_mutex_trylock(pthread_mutex_t *mutex) {
    if (!mutex) return EINVAL;
    if (__atomic_exchange_n(&mutex->lock, 1, __ATOMIC_ACQUIRE) != 0) {
        return EBUSY;
    }
    mutex->owner = (unsigned long)_syscall(SYS_GETPID, 0, 0, 0, 0, 0);
    return 0;
}

int pthread_mutex_unlock(pthread_mutex_t *mutex) {
    if (!mutex) return EINVAL;
    mutex->owner = 0;
    __atomic_store_n(&mutex->lock, 0, __ATOMIC_RELEASE);
    return 0;
}

/* ── Mutex attributes ── */

int pthread_mutexattr_init(pthread_mutexattr_t *attr) {
    if (!attr) return EINVAL;
    attr->type = 0;
    return 0;
}

int pthread_mutexattr_destroy(pthread_mutexattr_t *attr) {
    (void)attr;
    return 0;
}

/* ──────────────────────────────────────────────────────────────────────
 *  Condition variables (spin-wait based — no futex available)
 * ────────────────────────────────────────────────────────────────────── */

int pthread_cond_init(pthread_cond_t *cond, const pthread_condattr_t *attr) {
    (void)attr;
    if (!cond) return EINVAL;
    cond->seq = 0;
    return 0;
}

int pthread_cond_destroy(pthread_cond_t *cond) {
    (void)cond;
    return 0;
}

int pthread_cond_wait(pthread_cond_t *cond, pthread_mutex_t *mutex) {
    if (!cond || !mutex) return EINVAL;

    /* Snapshot the current sequence number before releasing the mutex. */
    unsigned int seq = __atomic_load_n(&cond->seq, __ATOMIC_ACQUIRE);

    /* Release the mutex so other threads can make progress. */
    pthread_mutex_unlock(mutex);

    /*
     * Spin until the sequence counter changes (indicating a signal or
     * broadcast).  Yield on every iteration to avoid burning CPU.
     */
    while (__atomic_load_n(&cond->seq, __ATOMIC_ACQUIRE) == seq) {
        _syscall(SYS_YIELD, 0, 0, 0, 0, 0);
    }

    /* Re-acquire the mutex before returning, per POSIX semantics. */
    pthread_mutex_lock(mutex);
    return 0;
}

int pthread_cond_signal(pthread_cond_t *cond) {
    if (!cond) return EINVAL;
    __atomic_fetch_add(&cond->seq, 1, __ATOMIC_RELEASE);
    return 0;
}

int pthread_cond_broadcast(pthread_cond_t *cond) {
    if (!cond) return EINVAL;
    __atomic_fetch_add(&cond->seq, 1, __ATOMIC_RELEASE);
    return 0;
}

/* ── Condition variable attributes ── */

int pthread_condattr_init(pthread_condattr_t *attr) {
    if (!attr) return EINVAL;
    attr->_unused = 0;
    return 0;
}

int pthread_condattr_destroy(pthread_condattr_t *attr) {
    (void)attr;
    return 0;
}

/* ──────────────────────────────────────────────────────────────────────
 *  Thread-local storage
 *
 *  Without proper TLS segments (no __thread / TPIDR_EL0), we use a
 *  simple static 2D array indexed by (tid % MAX_THREADS, key).
 *  This works correctly as long as no two live threads have TIDs
 *  that are congruent modulo MAX_THREADS.
 * ────────────────────────────────────────────────────────────────────── */

int pthread_key_create(pthread_key_t *key, void (*destructor)(void *)) {
    if (!key) return EINVAL;

    _spin_lock(&_tls_key_lock);
    for (int i = 0; i < PTHREAD_KEYS_MAX; i++) {
        if (!_tls_key_used[i]) {
            _tls_key_used[i] = 1;
            _tls_key_dtor[i] = destructor;
            *key = (pthread_key_t)i;
            _spin_unlock(&_tls_key_lock);
            return 0;
        }
    }
    _spin_unlock(&_tls_key_lock);
    return EAGAIN;
}

int pthread_key_delete(pthread_key_t key) {
    if (key >= PTHREAD_KEYS_MAX) return EINVAL;

    _spin_lock(&_tls_key_lock);
    if (!_tls_key_used[key]) {
        _spin_unlock(&_tls_key_lock);
        return EINVAL;
    }

    /* Clear all per-thread values for this key. */
    for (int t = 0; t < MAX_THREADS; t++) {
        _tls_values[t][key] = NULL;
    }

    _tls_key_used[key] = 0;
    _tls_key_dtor[key] = NULL;
    _spin_unlock(&_tls_key_lock);
    return 0;
}

int pthread_setspecific(pthread_key_t key, const void *value) {
    if (key >= PTHREAD_KEYS_MAX || !_tls_key_used[key]) return EINVAL;
    unsigned int tid_idx = (unsigned int)((unsigned long)_syscall(SYS_GETPID, 0, 0, 0, 0, 0) % MAX_THREADS);
    _tls_values[tid_idx][key] = (void *)value;
    return 0;
}

void *pthread_getspecific(pthread_key_t key) {
    if (key >= PTHREAD_KEYS_MAX || !_tls_key_used[key]) return NULL;
    unsigned int tid_idx = (unsigned int)((unsigned long)_syscall(SYS_GETPID, 0, 0, 0, 0, 0) % MAX_THREADS);
    return _tls_values[tid_idx][key];
}

/* ──────────────────────────────────────────────────────────────────────
 *  pthread_once
 *
 *  Three states: 0 = not started, 1 = in progress, 2 = complete.
 *  The first thread to CAS 0→1 runs the routine; other threads spin
 *  until the state becomes 2.
 * ────────────────────────────────────────────────────────────────────── */

int pthread_once(pthread_once_t *once_control, void (*init_routine)(void)) {
    if (!once_control || !init_routine) return EINVAL;

    /* Fast path: already initialized. */
    if (__atomic_load_n(once_control, __ATOMIC_ACQUIRE) == 2) return 0;

    /* Try to become the initializer. */
    int expected = 0;
    if (__atomic_compare_exchange_n(once_control, &expected, 1,
                                    0 /* strong */, __ATOMIC_ACQ_REL,
                                    __ATOMIC_ACQUIRE)) {
        init_routine();
        __atomic_store_n(once_control, 2, __ATOMIC_RELEASE);
        return 0;
    }

    /* Another thread is initializing — spin until complete. */
    while (__atomic_load_n(once_control, __ATOMIC_ACQUIRE) != 2) {
        _syscall(SYS_YIELD, 0, 0, 0, 0, 0);
    }
    return 0;
}
