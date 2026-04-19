# Async Foundation

This branch explores the runtime and kernel groundwork needed to move anyOS
toward a real async/await-based model for IPC-heavy services and responsive UIs.

## Goals

- Remove polling-heavy request loops from hot paths.
- Make `confd`, `amid`, and similar daemons event-driven.
- Keep GUI threads responsive while backend work continues in the background.
- Establish a small anyOS-native async runtime instead of layering syntax over
  blocking primitives.

## Base Work Packages

### 1. Kernel Wait Primitives

Introduce a wait model that can sleep until one or more objects become ready:

- pipe readable / writable
- event channel readable
- child process exit
- timer deadline

Target direction:

- `wait_one(handle, flags, timeout)`
- `wait_many(handles, timeout)`

### 2. Stable Handle Model

Define a compact handle type for waitable kernel objects so userland can treat
pipes, timers, event channels, and process waits uniformly.

### 3. libsyscall Surface

Add safe wrappers for:

- wait registration / waiting
- non-blocking readiness checks
- timer-backed wakeups

### 4. Userland Executor

Build a small runtime crate that provides:

- task spawning
- wakers
- timer queue
- local executor for UI threads
- optional multithreaded executor for services

### 5. Async Core Clients

Port the hot IPC clients first:

- `libconf`
- `libami`

Target shape:

- `client.get(...).await`
- `watch.next().await`

### 6. UI Integration

Connect the runtime to `libanyui` so background tasks can marshal UI updates
without blocking the main event loop.

## Near-Term Branch Milestones

1. Add a kernel-facing design for waitable handles and `wait_many`.
2. Prototype a tiny runtime crate with timer + wake support.
3. Build an async version of one IPC client (`libconf` first).
4. Move `Service Manager` to background snapshot refresh using the new model.

## Non-Goals For The First Slice

- Full Tokio-like ecosystem compatibility
- Network stack rewrite
- Broad conversion of all apps at once

## Success Criteria

- `Service Manager` opens without multi-second stalls.
- `Config Explorer` can watch and refresh without blocking the UI thread.
- `confd` and `amid` clients no longer depend on sleep-based polling loops for
  normal request/response flow.
