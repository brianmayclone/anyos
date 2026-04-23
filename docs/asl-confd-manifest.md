# ASL `confd` Manifest v1

## Ziel

Dieses Dokument konkretisiert, wie ASL seinen Konfigurationsvertrag in `confd`
registrieren soll.

Es ist die Bruecke zwischen:

- den ASL-ADRs
- dem logischen Konfigschema in [asl-config-schema.md](/daten1/development/brian/anyos/docs/asl-config-schema.md)
- der spaeteren `asld`-Implementierung mit `libconf_schema`

## Grundsatz

ASL soll einen eigenen systemweiten Namespace unter `confd` belegen:

```text
system/platform/asl/...
```

`asld` ist der fachliche Owner dieses Namespace und registriert den Manifest-
Vertrag beim Start.

## Namespace Layout

V1 trennt logisch zwischen:

- Image-Katalog
- Distro-Konfiguration
- optionalen Profilen oder Defaults

Empfohlene Struktur:

```text
system/platform/asl/
  images/
    <image-ref>/
      family
      version
      arch
      kernel_profile
      import_path
      trust_level
  distros/
    <name>/
      id
      owner
      base_image_ref
      kernel_profile
      resources/
        memory_mb
        vcpu_count
        autostart
      storage/
        layout
        base_image_path
        overlay_image_path
        state_image_path
        state_image_enabled
      network/
        mode
        dns_mode
        allow_outbound
      agent/
        enabled
        required_for_rich_integration
        fallback_console_enabled
      lifecycle/
        restart_on_failure
        shutdown_timeout_ms
        boot_timeout_ms
      mounts/
        <mount-id>/
          host_path
          guest_path
          mode
          metadata_mode
          case_mode
          exec_policy
          watch_policy
          description
      port_forwards/
        <rule-id>/
          listen_address
          listen_port
          guest_port
          protocol
          description
      metadata/
        distro_family
        distro_version
        notes
```

## Registration Strategy

V1 sollte den ASL-Vertrag in zwei Schichten registrieren:

### 1. Root Manifest

Ein statischer Root-Manifest-Vertrag fuer:

- `platform/asl`
- `platform/asl/images`
- `platform/asl/distros`

Dieser Teil legt die stabilen Wurzeln und globale Defaults fest.

### 2. Per-Distro Materialization

Einzelne Distributionen werden zur Laufzeit durch `asld` materialisiert, wenn:

- eine Distribution erzeugt wird
- ein Import erfolgt
- eine bestehende Distribution migriert oder validiert wird

Das heisst:

- der Root-Manifest-Vertrag ist statisch
- Distro-spezifische Subtrees entstehen dynamisch unter einer kontrollierten
  Wurzel

## Recommended Root Manifest

### Namespace

```text
platform/asl
```

### Scope

`RegistryScope::System`

### Version

`1`

### Directories

```text
images
distros
profiles
```

### Optional Root Defaults

V1 kann sparsam mit Root-Defaults sein. Empfehlenswert sind nur wirklich globale
Vorgaben:

- `profiles/default/network_mode = "nat"`
- `profiles/default/dns_mode = "host-broker"`
- `profiles/default/memory_mb = 2048`
- `profiles/default/vcpu_count = 2`
- `profiles/default/agent_enabled = true`
- `profiles/default/fallback_console_enabled = true`

## Example `libconf_schema` Skeleton

```rust
use libconf_schema::{
    default_bool, default_int, default_string, manifest, RegistryScope, ServiceSchema,
};

const ASL_DIRS: &[&str] = &[
    "images",
    "distros",
    "profiles",
    "profiles/default",
];

const ASL_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_string("profiles/default/network_mode", "nat"),
    default_string("profiles/default/dns_mode", "host-broker"),
    default_int("profiles/default/memory_mb", 2048),
    default_int("profiles/default/vcpu_count", 2),
    default_bool("profiles/default/agent_enabled", true),
    default_bool("profiles/default/fallback_console_enabled", true),
];

const ASL_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];

const ASL_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "platform/asl",
    RegistryScope::System,
    1,
    ASL_DIRS,
    ASL_DEFAULTS,
    ASL_MIGRATIONS,
);

const ASL_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("asld", &ASL_MANIFEST);
```

## Dynamic Distro Materialization

Da `libconf_schema` fuer statische Namespace-Defaults optimiert ist, sollte
`asld` pro Distribution die konkreten Keys danach explizit schreiben oder
ensuren.

Beispiel fuer `ubuntu-dev`:

```text
system/platform/asl/distros/ubuntu-dev/id
system/platform/asl/distros/ubuntu-dev/owner
system/platform/asl/distros/ubuntu-dev/base_image_ref
system/platform/asl/distros/ubuntu-dev/resources/memory_mb
...
```

Empfehlung:

- `register_manifest()` fuer die Root-Wurzel
- `ensure_distro_tree(name, config)` fuer neue oder reparierte Distros

## Type Guidance

Empfohlene `confd`-Wertetypen:

- Strings fuer Namen, Pfade, Modi, Profile
- Ints fuer RAM, Ports, Zeitlimits, CPU-Anzahl
- Bools fuer Schalter wie `enabled`, `autostart`, `allow_outbound`

Keine serialisierten JSON-Blobs als Regelfall.

Der Registry-Baum soll browsebar, watchbar und auditierbar bleiben.

## Ownership Rules

### `asld`

Darf schreiben unter:

```text
system/platform/asl/...
```

und ist Owner fuer:

- Distro-Konfiguration
- Image-Katalog
- Profile-Defaults

### Other Components

Andere Komponenten wie `aslfsd`, `aslnetd`, `aslconsoled` sollen ihre
Runtime-Diagnose nicht blind in denselben Konfigbaum mischen.

Wenn sie Konfigurationsnahe Unterbereiche brauchen, dann nur ueber klar von
`asld` definierte Ownership.

## Audit Implications

Durch `confd` entstehen automatisch bessere Audit- und Watch-Moeglichkeiten fuer:

- Distro-Erzeugung
- Ressourcenaenderungen
- Mount-Aenderungen
- Port-Forward-Regeln
- Agent-Policy-Aenderungen

Das ist ein wichtiger Grund, ASL nicht dateizentriert zu bauen.

## Migration Strategy

Falls ASL initial mit materialisierten Distro-Snapshots oder Importdateien
arbeitet, gilt:

- Import liest externe Beschreibung
- `asld` validiert sie
- `asld` schreibt die autoritative Endkonfiguration nach `confd`
- Laufzeit liest ab dann nur noch aus `confd`

## Open Points

- ob `profiles/` in v1 schon real genutzt oder nur reserviert wird
- wie Distro-Loeschung und Garbage Collection mit `confd DEL` modelliert wird
- ob `images/<image-ref>/...` komplett in `confd` oder teilweise aus einem
  separaten Katalog gespiegelt wird
- ob Mount- und Port-Eintraege besser ueber stabile IDs oder ueber Pfad-/
  Port-Keys adressiert werden
