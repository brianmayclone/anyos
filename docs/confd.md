# confd — Central Configuration Registry

`confd` is the central configuration daemon for anyOS. It is started directly
by the kernel, immediately after `amid`, and before the compositor or text-mode
stack. That makes it available to both early-boot services and user-space apps
without routing configuration through `init` or loose files in `/System/etc`.

## Design Goals

- Registry-like, but not chaotic
- Strong separation between machine-wide and per-user settings
- Auditable writes
- Watchable keys and folders
- Predictable namespace ownership
- Migration path away from ad-hoc config files

## Hives

`confd` deliberately exposes only two first-class hives:

- `system/<path>`
- `user/<path>` — resolved by `confd` against the caller's `uid`

Internally, user paths are canonicalized as `user/<uid>/<path>`, so each user
gets an isolated virtual tree while clients still program against a clean
two-scope API.

Examples:

- `system/network/interfaces/default`
- `system/services/networkd/policy`
- `user/shell/theme`
- `user/apps/finder/sidebar_width`

## Current Protocol Surface

The daemon currently supports:

- `HELLO`
- `PING`
- `REGISTER`
- `MKDIR`
- `SET`
- `GET`
- `DEL`
- `LIST`
- `WATCH`
- `UNWATCH`

Transport is named-pipe request/reply with a per-client reply pipe:

- request pipe: `confd`
- reply pipe: `confd-<tid>`

## Persistence Model

`confd` persists into `/System/sysdb/config.db` via `libdb`.

Current tables:

- `registry`
- `audit`
- `schemas`

`registry` stores canonical path, logical path, scope, owner, node kind,
typed value, version, timestamps, and last writer metadata.

`audit` stores append-only write history with:

- sequence number
- actor uid
- actor name
- tid
- action
- scope
- logical path
- status
- detail
- version
- timestamp

`schemas` stores the declared namespace manifests for services and apps:

- scope
- owner uid
- namespace root
- declared schema version
- applied migration version
- manifest payload
- writer metadata
- timestamp

## Programmatic Registration Model

Clients do not rely on loose sysroot templates anymore. Instead, they register
their configuration contract on startup through `libconf`.

The intended pattern is:

1. Embed the manifest in the binary.
2. Connect to `confd` during early startup.
3. Call `register_manifest(...)`.
4. Read effective values back from `confd`.
5. Keep file-based parsing only as a temporary fallback while migrating.

That means the source of truth for defaults and patch steps lives with the
owning binary or bundle resource, not in a disconnected `/System/etc` file.

For common service/app use, anyOS now provides `libconf_schema` as a thin
declarative layer on top of `libconf`. It supplies:

- const-friendly manifest builders
- typed default and migration helpers
- a `ServiceSchema` helper for registration and typed reads

### Manifest Contents

A manifest declares:

- `namespace`
- `scope`
- `schema version`
- directory nodes
- default key/value pairs
- versioned migration steps

`confd` applies it idempotently:

- directories are ensured
- defaults are written only if missing
- migrations are applied once when the registered schema version advances

### Payload Semantics

Current manifest operations:

- `D` — ensure directory
- `K` — ensure default key if missing
- `M` — apply versioned migration write
- `X` — delete a key or subtree
- `R` — rename a key or subtree
- `C` — copy a key or subtree

This stays intentionally declarative: safe, deterministic, easy to audit, and
shipped together with the owning binary or bundle resource.

## Compatibility `/etc` View

A virtual `/etc` projection is a good compatibility layer, but it should not be
the source of truth.

Recommended model:

- `confd` remains authoritative
- a synthetic read-only filesystem view is mounted at `/etc`
- legacy readers can still consume generated config files
- writes are rejected at the VFS boundary and must go through `confd`

That gives us Unix-style discoverability without reopening the old problem of
disconnected mutable files in the sysroot.

For anyOS, the best implementation is likely a small pseudo filesystem or
FUSE-like projection that renders selected namespaces from `confd` on demand:

- system services from `system/services/<name>/...`
- user-readable app settings from `user/apps/<app-id>/...`
- optional policy-controlled exports for diagnostics

Important constraint: the compatibility mount should be read-only, generated,
and explicitly scoped. Two-way sync would make schema ownership, audit trails,
and migrations much messier again.

## Current Adopters

The following components already use the embedded registration path:

- `dnsd`
- `networkd`
- `searchd`

Each of them:

1. embeds its registry manifest in code,
2. registers it with `confd` during startup,
3. reads effective values from `confd`,
4. uses `/System/etc/...` only as a migration fallback.

## Namespace Conventions

To keep the registry predictable, clients should reserve a clear subtree:

- system daemons: `system/services/<name>/...`
- system components: `system/platform/<component>/...`
- user apps: `user/apps/<bundle-or-app-id>/...`
- shared user features: `user/profile/<feature>/...`

In the current protocol the client passes only the logical namespace root
without the leading hive token because the scope is supplied separately.

Examples:

- `services/networkd`
- `services/dnsd`
- `apps/finder`
- `profile/shell`

## Binary-Embedded Defaults

The preferred implementation style is:

- Rust constants in code for small manifests
- `include_str!()` or `include_bytes!()` for larger attached resources

Either way, the owning package ships its defaults and migrations together with
the binary. Sysroot files may still exist during the migration window, but they
are fallback input only, not the long-term configuration authority.

## Security Model

Current baseline:

- `system/*` is writable only by `uid 0`
- `user/*` resolves to the caller's own user hive
- reads are scoped by canonical path resolution

This is intentionally simple for v1 and keeps services on the system plane
while applications live on the user plane.

## Why This Is Only “Enterprise Foundation” Today

The current implementation is structured like an enterprise service, but it is
not yet feature-complete enterprise configuration management. The next steps are:

1. Namespace ACLs beyond root-vs-user, for delegated admin and service tenancy.
2. Schema/policy validation per subtree, so services can reject malformed writes.
3. Transaction support for multi-key updates.
4. Audit query/export API for admin tooling and Event Viewer integration.
5. Replication/import/export tooling for backup, imaging, and profile roaming.
6. Change journals and startup replay support for deterministic migrations.
7. Explicit `user@<uid>` admin access for diagnostics and management tools.
8. Conditional patching and validation hooks on top of the richer migration actions.

## Intended Migration Path

1. Keep legacy files in `/System/etc` as boot defaults.
2. Move service-specific parsing into each service's initialization code.
3. On first start, each service imports its legacy file into `confd`.
4. New writes go to `confd` only.
5. Legacy file readers are retired once migration is stable.

That gives anyOS a controlled move from file-centric configuration to a real
central registry without a risky flag day.
