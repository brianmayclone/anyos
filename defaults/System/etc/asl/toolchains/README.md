# ASL Toolchain-Profile

Diese Skripte richten in einer ASL-Distro (Debian) eine
Sprach-/Build-Toolchain ein. Sie sind in der Distro unter
`/mnt/aslctl/toolchains/` o. ae. erreichbar (ueber Shared-Mount oder
einmaligen Push), aber **gestartet werden sie aus anyOS** ueber:

```sh
aslctl run <distro> --cwd /tmp -- bash /pfad/zu/<profil>.sh
```

Die Datei wird je nach Distro-Mount-Layout entweder kopiert oder ueber
einen Shared-Folder durchgereicht. Die Skripte selbst sind **idempotent**:
ein zweiter Aufruf ueberschreibt nichts und meldet "already installed".

## Profile

| Profil       | Inhalt                                                |
|--------------|-------------------------------------------------------|
| `dev-c.sh`   | gcc, g++, clang, gdb, make, cmake, ninja, pkg-config  |
| `dev-rust.sh`| rustup mit stable toolchain, cargo                    |
| `dev-node.sh`| nvm + Node.js LTS, npm                                |
| `dev-java.sh`| OpenJDK 21, Maven, Gradle                             |

## Voraussetzungen

- Distro ist Debian-basiert (testet `which apt-get`).
- Outbound-Netzwerk funktioniert (DNS-Broker via aslnetd, ADR-0003).
- Skripte werden mit Privilegien gestartet, die `apt`/Schreibrechte in
  `$HOME/.cargo` o. ae. haben.

## Konventionen

- Jedes Skript beginnt mit `set -euo pipefail` damit eine Fehlerquelle
  sofort sichtbar wird (ADR-0010 Stabilitaet).
- Stdout zeigt Fortschritt, stderr enthaelt Fehler. Exit-Code 0 = Erfolg.
- Vor dem ersten apt-Befehl wird `apt-get update` einmal gemacht.
- Skripte sind kleiner als 100 Zeilen — keine versteckte Logik.

## Aktualisierung

Wenn Debian seine Paketnamen aendert oder eine Toolchain wechselt
(z. B. OpenJDK Version), bleiben die Skripte hier am gleichen Ort. Sie
sind versionsgebunden mit dem ASL-Release; Manuelles Anpassen reicht.
