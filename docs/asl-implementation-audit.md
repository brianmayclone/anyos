# ASL Implementation Audit (2026-05-07)

Vergleich der ASL-Spezifikationen gegen die tatsaechliche Implementierung.
Grundlage fuer den Dev-Plattform-Implementierungsplan in
[../todos/asl-anyos-subsystem-linux.md](../todos/asl-anyos-subsystem-linux.md).

Audit-Quellen:
- [asld-control-plane-api.md](asld-control-plane-api.md) ↔ `system/daemons/asld/src/`
- [aslctl-cli.md](aslctl-cli.md) ↔ `bin/aslctl/src/`
- [asl-config-schema.md](asl-config-schema.md), [asl-confd-manifest.md](asl-confd-manifest.md) ↔ `system/daemons/asld/src/{schema,config,model}.rs`

---

## 1. Control-Plane API (28 Operationen)

**Stand: 24 IMPLEMENTED, 1 STUB, 3 MISSING, 1 PARTIAL — ~86% Abdeckung.**

| Operation | Status | Datei:Zeile | Anmerkung |
|-----------|--------|-------------|-----------|
| **Discovery** | | | |
| ListDistros | IMPLEMENTED | ipc.rs:98 | runtime.list() |
| GetDistroConfig | IMPLEMENTED | ipc.rs:209 | runtime.config_lines() |
| GetDistroStatus | IMPLEMENTED | ipc.rs:114 | runtime.status() |
| **Lifecycle** | | | |
| CreateDistro | IMPLEMENTED | ipc.rs:147 | runtime.create_with_kernel_profile() |
| StartDistro | IMPLEMENTED | ipc.rs:293 | runtime.start() |
| StopDistro | IMPLEMENTED | ipc.rs:309 | runtime.stop() |
| RestartDistro | IMPLEMENTED | ipc.rs:301 | runtime.restart() |
| DeleteDistro | IMPLEMENTED | ipc.rs:169 | runtime.delete() |
| **Import/Export** | | | |
| ImportBaseImage | IMPLEMENTED (anyOS) / STUB (Linux-Host) | ipc.rs:245 / runtime.rs:1135 | Auf anyOS funktional. Linux-Host-Build hat `NotImplemented`-Stub — beabsichtigt (asld auf Linux nur fuer Host-Tests). Bestaetigt 2026-05-07. |
| ExportDistro | IMPLEMENTED | ipc.rs:200 | runtime.export_lines() |
| **Config Mutation** | | | |
| UpdateResources | IMPLEMENTED | ipc.rs:218 | |
| SetNetworkMode | IMPLEMENTED | ipc.rs:264 | |
| **Mount Management** | | | |
| ListMounts | IMPLEMENTED | ipc.rs:493 | |
| AddMount | IMPLEMENTED | ipc.rs:512 | |
| RemoveMount | IMPLEMENTED | ipc.rs:522 | |
| ValidateMounts | IMPLEMENTED | ipc.rs:532 | |
| **Port Management** | | | |
| ListPortForwards | IMPLEMENTED | ipc.rs:541 | |
| AddPortForward | IMPLEMENTED | ipc.rs:550 | |
| RemovePortForward | IMPLEMENTED | ipc.rs:560 | |
| **Console and Exec** | | | |
| OpenShellSession | IMPLEMENTED | ipc.rs:406 | |
| ExecCommand | IMPLEMENTED | ipc.rs:451 | |
| **Agent** | | | |
| GetAgentStatus | IMPLEMENTED | ipc.rs:313 | |
| RestartAgent | IMPLEMENTED | ipc.rs:317 | |
| **Diagnostics** | | | |
| GetLogs | MISSING | — | Kein Dispatcher, keine Implementierung |
| RunDoctor | MISSING | — | CLI hat `doctor`, asld-Endpoint fehlt |
| ListEvents | MISSING | — | aslobsd sammelt, API exponiert nichts |
| InspectDistro | PARTIAL | ipc.rs:387 / runtime.rs:853 | `diagnose()` deckt Spec-Felder unvollstaendig |

### Top 5 Luecken nach User-Value
1. **GetLogs** — essentiell fuer Debugging.
2. **RunDoctor** — Health-Check-Framework (VM, Agent, Storage, Network).
3. **ListEvents** — Audit-Trail.
4. ~~**ImportBaseImage** unter Linux~~ — geklaert 2026-05-07: kein Bug, Linux-
   Stub ist beabsichtigt. Auf anyOS funktioniert der Import. Stufe-2/3
   Image-Trust ist eigener TODO (siehe ADR-0011).
5. **InspectDistro** Spec-Gap.

---

## 2. aslctl CLI

**Stand: ~60% Abdeckung. Kernpfade da, Diagnose- und Config-Subcommands fehlen.**

### Fehlende Subcommands
- `show` (Doc Z.52) — komplett MISSING.
- `run --cwd <path> --env KEY=VALUE -- <cmd>` (Doc Z.99) — nur eingeschraenktes
  `exec` existiert. Wichtig fuer IDE-Build-Tasks.
- `config edit` (Doc Z.190) — MISSING.
- `profile list`, `profile show` (Doc Z.192-193) — MISSING.
- `logs`, `logs --follow` (Doc Z.206-207) — MISSING.
- `inspect` (Doc Z.209) — MISSING.
- `events` (Doc Z.210) — als `vm-events` umgesetzt, anderer Name.

### Namensdiskrepanzen
- Doc: `aslctl import` → Code: `aslctl storage import` (`bin/aslctl/src/lib.rs:371`).
- Doc: `aslctl events` → Code: `aslctl vm-events`.

### Vermeintlicher Bug (geklaert 2026-05-07)
- `port validate` sendet `NETWORK_VALIDATE` (`bin/aslctl/src/lib.rs:1194`).
  Das ist **kein Bug**: asld hat nur einen kombinierten Validator
  `validate_network_set()` (`runtime.rs:611`), der Network-Policy + alle
  Port-Forwards gemeinsam prueft. `network validate` und `port validate` sind
  CLI-Aliase fuer denselben Wire-Command. Akzeptabel als UX.

### Globale Flags fehlen komplett
Doc Z.35-44: `--json`, `--quiet`, `--verbose`, `--timeout`, `--user` werden
nicht geparst. `--json` ist Voraussetzung fuer aslmanager-Backend und Skripte.

### Korrekt als reserved
- `suspend` / `resume` (Doc Z.76).

### Implementiert (zur Vollstaendigkeit)
list, status, create, delete, restart, start, stop, export, clone, shell
(`--fallback-console`, `--session`), exec, mount {list,add,remove,show,validate},
port {list,add,remove}, network {show,set,validate}, config {get,set-resources},
doctor, diagnose (Alias), agent {status,restart}.

---

## 3. confd-Schema

**Stand: ~99% Kongruenz. Alle 42 dokumentierten Felder sind registriert und
werden geparst.**

### Eine Anomalie
`seed_image_path` existiert in:
- `system/daemons/asld/src/config.rs:222-224`
- `system/daemons/asld/src/model.rs:137`

…ist aber in `docs/asl-config-schema.md` und `docs/asl-confd-manifest.md` nicht
dokumentiert. Entweder Doc nachziehen oder klarstellen, dass es ein internes
Feld ist (Storage-Sektion, Z.142-171).

### Geprueft (alle PARSED)

**Top-Level**: schema_version, id, name, owner, base_image_ref, kernel_profile.

**Resources**: memory_mb, vcpu_count, autostart.

**Storage**: layout, base_image_path, overlay_image_path, state_image_path,
state_image_enabled. (+ undokumentiertes seed_image_path)

**Network**: mode, dns_mode, allow_outbound.

**Mounts (Array)**: host_path, guest_path, mode, metadata_mode, case_mode,
exec_policy, watch_policy, description.

**Port-Forwards (Array)**: listen_address, listen_port, guest_port, protocol,
description.

**Agent**: enabled, required_for_rich_integration, fallback_console_enabled.

**Lifecycle**: restart_on_failure, shutdown_timeout_ms, boot_timeout_ms.

**Metadata**: distro_family, distro_version, notes.

---

## Konsolidierte Luecken-Liste (sortiert nach User-Value)

### Hoher Wert
1. **GetLogs + `aslctl logs [--follow]`** — Diagnose-Pfad.
2. **ListEvents + `aslctl events`** — aslobsd-Daten exponieren, CLI-Naming
   fixen.
3. **`aslctl run --cwd --env -- <cmd>`** — Build-Workflow.
4. **`aslctl show`** — Doc-Pflichtbefehl.
5. **ImportBaseImage** unter Linux.

### Mittlerer Wert
6. **InspectDistro** Spec-Vervollstaendigung.
7. **Globale CLI-Flags** `--json` (Pflicht fuer aslmanager-Backend), `--quiet`,
   `--verbose`, `--timeout`.
8. **`config edit`** — interaktiver Config-Editor.

### Niedriger Wert / Doku
10. **`profile list/show`** — erst sinnvoll wenn mehr Kernel-Profile existieren.
11. **`seed_image_path`** in asl-config-schema.md ergaenzen.
12. **CLI-Naming** `vm-events` → `events` (Alias).
13. **`docs/asld-scaffolding-plan.md`** archivieren (ueberholt).
