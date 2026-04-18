# anyOS AMI v1 Protocol

AMI v1 defines the Anywhere Management Interface as a small shared runtime state service with change notifications. Unlike the current collector-oriented `amid`, AMI v1 is designed around active publication by system services.

**Binary:** `/System/bin/amid`  
**Source:** `system/daemons/amid/`  
**Main Pipe:** `ami`  
**Reply Pipe:** `ami-{tid}`  
**Role:** Shared runtime state store + prefix watch service

## Overview

AMI v1 is intended to be the default mechanism for sharing live runtime state between daemons, tools, and UI components.

Examples:

- `dnsd` publishes `dns.status`, `dns.cache.entries`, `dns.last_updated_dns`
- `init` publishes `system.boot.phase`, `system.upgrade.pending`
- `svc` publishes `svc.<name>.state`, `svc.<name>.tid`
- a settings page reads `LIST dns.` and then subscribes with `WATCH dns.`
- `AMI Console` renders a live graphical view of namespaces, keys, and change highlights

AMI v1 is **not** meant to replace every private daemon cache or every persistent configuration store. It is specifically for:

- shared runtime state
- health and status information
- progress and counters
- last-known values that other processes may need
- live updates via watch subscriptions

## Design Goals

1. Services publish their own state instead of having AMI rediscover it
2. Readers can fetch a snapshot and then receive incremental updates
3. Writer access is scoped by namespace / key prefix
4. The external protocol is simple and explicit, not free-form SQL
5. Internal storage remains replaceable

## Key Model

Keys are always fully qualified and ASCII-only in v1.

Examples:

```text
dns.status
dns.cache.entries
dns.last_updated_dns
system.boot.phase
system.upgrade.pending
svc.dnsd.state
svc.dnsd.tid
update.target.version
```

Rules:

- segments are separated by `.`
- writers should use stable prefixes
- prefix operations are lexical prefix matches
- readers use `LIST <prefix>` for snapshots and `WATCH <prefix>` for deltas

## Value Types

AMI v1 supports three external value types:

| Type | Meaning | Example |
|------|---------|---------|
| `string` | UTF-8 / text value without tabs or newlines in protocol form | `ready` |
| `int` | signed integer | `42` |
| `bool` | boolean flag | `true` |

Internal storage may use numeric type tags:

| Code | Type |
|------|------|
| `1` | `string` |
| `2` | `int` |
| `3` | `bool` |

## Ownership Model

Every writer is associated with a logical service name.

The client must identify itself with:

```text
HELLO <service>
```

The daemon maps services to writable key prefixes. Example policy:

```text
dnsd=dns.
init=system.
updater=update.
svc=svc.
```

Writes outside the allowed prefix must be rejected with `ERR forbidden`.

### Recommended Rule

- if more than one process should read the value, it usually belongs in AMI
- if only the daemon itself needs the value, it should usually remain private

## Tooling

AMI v1 is consumed by multiple layers:

- `ami` CLI (`/System/bin/ami`) for shell access
- `AMI Console.app` for graphical inspection of keys, namespaces, and live updates
- `svc` for readiness tracking via `svc.<name>.*`
- daemon-side `libsvc`, which wraps `libami` for lifecycle publication

### Service Lifecycle Convention

AMI-aware services publish lifecycle state below:

```text
svc.<name>.state
svc.<name>.ready
svc.<name>.error
svc.<name>.health
svc.<name>.tid
svc.<name>.started_at
```

Typical values:

- `state = starting | ready | failed | stopping`
- `ready = true | false`
- `health = ready | configured | degraded | disabled | waiting-network`

This convention is what allows `svc` to wait for actual readiness instead of only checking whether a thread exists.

## IPC Protocol

AMI v1 uses a request pipe plus per-client reply pipes, following the same pattern used by several anyOS daemons.

### Pipes

- request pipe: `ami`
- reply pipe: `ami-{tid}`

### Request Format

Each request is one line:

```text
{tid}\t{command}\n
```

Examples:

```text
42\tHELLO dnsd\n
42\tSET dns.status string ready\n
42\tGET dns.status\n
42\tLIST dns.\n
42\tWATCH dns.\n
```

### General Response Rules

- line-oriented text protocol
- synchronous command responses are written to the caller's reply pipe
- watch notifications are also written to the caller's reply pipe
- `LIST` responses end with `END`
- errors start with `ERR`

## Commands

### `HELLO <service>`

Associates the calling client with a service identity used for write authorization.

Request:

```text
{tid}\tHELLO dnsd\n
```

Response:

```text
OK hello dnsd
```

### `SET <key> <type> <value>`

Creates or updates one value.

Request:

```text
{tid}\tSET dns.status string ready\n
{tid}\tSET dns.cache.entries int 42\n
{tid}\tSET dns.cache.enabled bool true\n
```

Success response:

```text
OK set dns.status 5 123456
```

Fields:

- key
- new version
- updated_at timestamp

Rules:

- `SET` is upsert
- version increases on each real change
- identical rewrite should not emit a watch event

### `GET <key>`

Reads a single key.

Request:

```text
{tid}\tGET dns.status\n
```

Success response:

```text
VALUE dns.status string ready 5 123456
```

Miss response:

```text
ERR not_found
```

### `DEL <key>`

Deletes one key.

Request:

```text
{tid}\tDEL dns.last_error\n
```

Success response:

```text
OK del dns.last_error 6 123500
```

Missing keys may either return `ERR not_found` or be treated as idempotent success. The preferred v1 behavior is idempotent success only if the implementation can still produce coherent version semantics.

### `LIST <prefix>`

Returns a point-in-time snapshot of all keys that start with a prefix.

Request:

```text
{tid}\tLIST dns.\n
```

Response:

```text
ITEM dns.cache.enabled bool true 1 120000
ITEM dns.cache.entries int 42 2 120010
ITEM dns.status string ready 5 123456
END
```

Rules:

- sort results by key
- do not interleave unrelated output before `END`

### `WATCH <prefix>`

Registers a watch for all changes below a prefix.

Request:

```text
{tid}\tWATCH dns.\n
```

Success response:

```text
OK watch 7
```

Where `7` is the watcher id.

### `UNWATCH <id>`

Removes a watch.

Request:

```text
{tid}\tUNWATCH 7\n
```

Success response:

```text
OK unwatch 7
```

### `PING`

Liveness probe.

Request:

```text
{tid}\tPING\n
```

Response:

```text
PONG
```

## Watch Events

Events are delivered asynchronously to the same reply pipe used for command responses.

Format:

```text
EVENT <watch_id> <kind> <key> <type> <value> <version> <updated_at>
```

Kinds:

- `set`
- `delete`
- optional future: `expire`

Examples:

```text
EVENT 7 set dns.status string resolving 6 123700
EVENT 7 set dns.status string ready 7 123900
EVENT 7 delete dns.last_error string - 8 124000
```

For deleted values, the recommended protocol form is:

```text
EVENT 7 delete dns.last_error string - 8 124000
```

## Recommended Client Pattern

Clients that need live updates should follow this sequence:

1. `HELLO <service>`
2. `LIST <prefix>` to get a snapshot
3. `WATCH <prefix>` to receive deltas
4. merge subsequent `EVENT` messages into local state

This avoids polling loops and guarantees a clear snapshot-plus-delta flow.

## Internal Storage

The external protocol should not expose SQL directly. AMI may still use libdb internally.

Suggested state table:

```sql
CREATE TABLE state (
  key TEXT PRIMARY KEY,
  type INTEGER,
  value_text TEXT,
  value_int INTEGER,
  value_bool INTEGER,
  version INTEGER,
  updated_at INTEGER,
  owner TEXT
)
```

Suggested in-memory watch structure:

```text
watch_id
client_tid
prefix
reply_pipe_name
```

## Timestamps and Versions

- `updated_at` should use a monotonic timestamp such as `uptime_ms()`
- `version` is per-key, not global
- each successful state change increments the key's version

## Rollout Guidance

Recommended first writers:

1. `dnsd`
2. `init`
3. `updater`
4. `svc`

Recommended first readers:

1. `ami` CLI
2. settings pages
3. status / monitoring tools

## Non-Goals for v1

AMI v1 intentionally does **not** include:

- arbitrary SQL writes from clients
- generic joins or relational modeling in the external protocol
- large binary payloads
- complex transactions
- unrestricted write access

These can be revisited later if real use cases justify them.
