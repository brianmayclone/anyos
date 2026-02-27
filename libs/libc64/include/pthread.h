/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — POSIX threads (pthreads) interface for anyOS.
 *
 * Provides thread creation/join, spinlock-based mutexes (no futex syscall),
 * spin-wait condition variables, thread-local storage, and once semantics.
 */

#ifndef _PTHREAD_H
#define _PTHREAD_H

#include <stddef.h>
#include <stdint.h>

/* ── Thread handle ── */

/** Thread identifier — stores the kernel TID. */
typedef unsigned long pthread_t;

/* ── Thread attributes ── */

/** Detach state constants. */
#define PTHREAD_CREATE_JOINABLE 0
#define PTHREAD_CREATE_DETACHED 1

/** Thread attributes: configurable stack size and detach state. */
typedef struct {
    size_t          stack_size;    /**< Stack size in bytes (0 = default 64 KiB). */
    int             detach_state;  /**< PTHREAD_CREATE_JOINABLE or PTHREAD_CREATE_DETACHED. */
} pthread_attr_t;

/* ── Mutex ── */

/**
 * Spinlock-based mutex (no futex available).
 * The lock word is atomically exchanged; owner stores the locking TID
 * for debugging purposes.
 */
typedef struct {
    volatile int    lock;   /**< 0 = unlocked, 1 = locked. */
    unsigned long   owner;  /**< TID of the thread holding the lock (informational). */
} pthread_mutex_t;

/** Mutex attributes (reserved for future use). */
typedef struct {
    int             type;   /**< Mutex type (currently unused). */
} pthread_mutexattr_t;

/** Static initializer for pthread_mutex_t. */
#define PTHREAD_MUTEX_INITIALIZER { 0, 0 }

/* ── Condition variable ── */

/**
 * Spin-wait condition variable.
 * Waiters observe an atomic sequence counter; signal/broadcast increments it
 * to wake spinners.
 */
typedef struct {
    volatile unsigned int seq;  /**< Monotonically increasing sequence number. */
} pthread_cond_t;

/** Condition variable attributes (reserved for future use). */
typedef struct {
    int             _unused;
} pthread_condattr_t;

/** Static initializer for pthread_cond_t. */
#define PTHREAD_COND_INITIALIZER { 0 }

/* ── Once ── */

/**
 * One-time initialization control.
 * States: 0 = not started, 1 = in progress, 2 = complete.
 */
typedef volatile int pthread_once_t;

/** Static initializer for pthread_once_t. */
#define PTHREAD_ONCE_INIT 0

/* ── Thread-local storage ── */

/** TLS key — index into the per-thread value array. */
typedef unsigned int pthread_key_t;

/* ── Thread management ── */

/**
 * Create a new thread.
 *
 * Allocates a stack via SYS_MMAP, sets up a trampoline that calls
 * start_routine(arg) and then exits, and invokes SYS_THREAD_CREATE.
 *
 * @param thread        Receives the new thread's ID on success.
 * @param attr          Thread attributes, or NULL for defaults.
 * @param start_routine Entry point for the new thread.
 * @param arg           Argument passed to start_routine.
 * @return 0 on success, or an errno value on failure.
 */
int pthread_create(pthread_t *thread, const pthread_attr_t *attr,
                   void *(*start_routine)(void *), void *arg);

/**
 * Wait for a thread to terminate and retrieve its exit status.
 *
 * @param thread  The thread to join.
 * @param retval  If non-NULL, receives the value passed to pthread_exit().
 * @return 0 on success, or an errno value on failure.
 */
int pthread_join(pthread_t thread, void **retval);

/**
 * Mark a thread as detached — its resources are freed automatically on exit.
 *
 * @param thread  The thread to detach.
 * @return 0 on success, or an errno value on failure.
 */
int pthread_detach(pthread_t thread);

/**
 * Terminate the calling thread.
 *
 * @param retval  Exit value made available to pthread_join().
 */
void pthread_exit(void *retval) __attribute__((noreturn));

/**
 * Return the calling thread's ID.
 *
 * @return The pthread_t of the calling thread.
 */
pthread_t pthread_self(void);

/**
 * Compare two thread IDs.
 *
 * @return Non-zero if t1 and t2 refer to the same thread, 0 otherwise.
 */
int pthread_equal(pthread_t t1, pthread_t t2);

/* ── Thread attributes ── */

/**
 * Initialize a thread attributes object with default values.
 *
 * @return 0 on success.
 */
int pthread_attr_init(pthread_attr_t *attr);

/**
 * Destroy a thread attributes object.
 *
 * @return 0 on success.
 */
int pthread_attr_destroy(pthread_attr_t *attr);

/**
 * Set the stack size in a thread attributes object.
 *
 * @param stacksize  Desired stack size in bytes (minimum 4096).
 * @return 0 on success, EINVAL if stacksize is too small.
 */
int pthread_attr_setstacksize(pthread_attr_t *attr, size_t stacksize);

/**
 * Get the stack size from a thread attributes object.
 *
 * @param stacksize  Receives the configured stack size.
 * @return 0 on success.
 */
int pthread_attr_getstacksize(const pthread_attr_t *attr, size_t *stacksize);

/**
 * Set the detach state in a thread attributes object.
 *
 * @param detachstate  PTHREAD_CREATE_JOINABLE or PTHREAD_CREATE_DETACHED.
 * @return 0 on success, EINVAL if detachstate is invalid.
 */
int pthread_attr_setdetachstate(pthread_attr_t *attr, int detachstate);

/**
 * Get the detach state from a thread attributes object.
 *
 * @param detachstate  Receives the configured detach state.
 * @return 0 on success.
 */
int pthread_attr_getdetachstate(const pthread_attr_t *attr, int *detachstate);

/* ── Mutexes ── */

/**
 * Initialize a mutex.
 *
 * @return 0 on success.
 */
int pthread_mutex_init(pthread_mutex_t *mutex, const pthread_mutexattr_t *attr);

/**
 * Destroy a mutex.
 *
 * @return 0 on success.
 */
int pthread_mutex_destroy(pthread_mutex_t *mutex);

/**
 * Lock a mutex, spinning with yield on contention.
 *
 * @return 0 on success.
 */
int pthread_mutex_lock(pthread_mutex_t *mutex);

/**
 * Try to lock a mutex without blocking.
 *
 * @return 0 if the lock was acquired, EBUSY if already locked.
 */
int pthread_mutex_trylock(pthread_mutex_t *mutex);

/**
 * Unlock a mutex.
 *
 * @return 0 on success.
 */
int pthread_mutex_unlock(pthread_mutex_t *mutex);

/* ── Mutex attributes ── */

/**
 * Initialize a mutex attributes object.
 *
 * @return 0 on success.
 */
int pthread_mutexattr_init(pthread_mutexattr_t *attr);

/**
 * Destroy a mutex attributes object.
 *
 * @return 0 on success.
 */
int pthread_mutexattr_destroy(pthread_mutexattr_t *attr);

/* ── Condition variables ── */

/**
 * Initialize a condition variable.
 *
 * @return 0 on success.
 */
int pthread_cond_init(pthread_cond_t *cond, const pthread_condattr_t *attr);

/**
 * Destroy a condition variable.
 *
 * @return 0 on success.
 */
int pthread_cond_destroy(pthread_cond_t *cond);

/**
 * Atomically unlock the mutex, wait for a signal on cond, then re-lock.
 *
 * Uses spin-waiting with yield since no futex syscall is available.
 *
 * @return 0 on success.
 */
int pthread_cond_wait(pthread_cond_t *cond, pthread_mutex_t *mutex);

/**
 * Wake at least one thread waiting on the condition variable.
 *
 * @return 0 on success.
 */
int pthread_cond_signal(pthread_cond_t *cond);

/**
 * Wake all threads waiting on the condition variable.
 *
 * @return 0 on success.
 */
int pthread_cond_broadcast(pthread_cond_t *cond);

/* ── Condition variable attributes ── */

/**
 * Initialize a condition variable attributes object.
 *
 * @return 0 on success.
 */
int pthread_condattr_init(pthread_condattr_t *attr);

/**
 * Destroy a condition variable attributes object.
 *
 * @return 0 on success.
 */
int pthread_condattr_destroy(pthread_condattr_t *attr);

/* ── Thread-local storage ── */

/**
 * Create a TLS key with an optional destructor.
 *
 * @param key         Receives the new key index.
 * @param destructor  Called with the key's value when a thread exits (may be NULL).
 * @return 0 on success, EAGAIN if no keys are available.
 */
int pthread_key_create(pthread_key_t *key, void (*destructor)(void *));

/**
 * Delete a TLS key.
 *
 * @return 0 on success, EINVAL if key is invalid.
 */
int pthread_key_delete(pthread_key_t key);

/**
 * Set the calling thread's value for a TLS key.
 *
 * @return 0 on success, EINVAL if key is invalid.
 */
int pthread_setspecific(pthread_key_t key, const void *value);

/**
 * Get the calling thread's value for a TLS key.
 *
 * @return The thread-specific value, or NULL if not set or key is invalid.
 */
void *pthread_getspecific(pthread_key_t key);

/* ── Once ── */

/**
 * Ensure init_routine is called exactly once across all threads.
 *
 * @param once_control  Control variable (must be statically initialized to PTHREAD_ONCE_INIT).
 * @param init_routine  Function to call once.
 * @return 0 on success.
 */
int pthread_once(pthread_once_t *once_control, void (*init_routine)(void));

#endif /* _PTHREAD_H */
