# ADR-0006 - ASL boots Linux directly via AVM, without a BIOS

## Status

Accepted

## Date

2026-04-24

## Context

ASL ist als WSL2-artiges Subsystem festgelegt. Die VM soll nicht als
beliebiger PC-Emulator starten, sondern als kontrollierte Linux-Utility-VM unter
anyOS. Damit stellt sich die Bootfrage: Soll ASL ein Minimal-BIOS oder eine
Firmware mitbringen, oder soll der Host den Linux-Kernel direkt starten?

AVM ist das KVM-artige Kernel-Interface fuer VM/vCPU/Memory/Registers. Es ist
kein PC-Emulator und bringt keine BIOS-Services, kein PCI-Modell und keine
Block-/Netzwerkgeraete als Firmware-Vertrag mit.

## Decision

ASL verwendet fuer normale Distributionen einen direkten Linux-Bootpfad in
`asld`:

- `asld` liest das Kernel-Artifact aus
  `/System/var/asl/distros/<name>/boot/vmlinuz`
- optional wird
  `/System/var/asl/distros/<name>/boot/initrd.img` geladen
- `asld` baut die Linux-Zero-Page/`boot_params`
- `asld` schreibt Kernel, Cmdline und initrd in den Guest-Speicher
- `asld` setzt den vCPU-Startzustand auf den 32-bit Protected-Mode
  Linux-Entry
- AVM fuehrt danach den Gast aus

Ein BIOS oder eine PC-Firmware ist fuer ASL kein Bestandteil des
MVP-Bootpfads. Der vorhandene AVM-HLT-Bootstrap bleibt nur als ausdrueckliches
`avm-smoke-test` Profil fuer Hypervisor-Selbsttests erhalten.

## Consequences

### Positive

- keine BIOS-/Firmware-Abhaengigkeit fuer ASL
- weniger Geraeteemulation vor dem ersten Linux-Start erforderlich
- klare KVM-aehnliche Trennung: AVM stellt VM-Primitives, `asld` stellt den
  Userland-VMM/Loader
- Boot-Diagnose kann explizit melden, ob Kernel/initrd-Artefakte vorhanden sind

### Negative

- ASL kann damit keine beliebigen PC-Images booten
- Linux braucht danach trotzdem echte Runtime-Geraete oder Broker-Pfade fuer
  Console, Storage, Netzwerk und Agent-Kommunikation
- der Loader muss Linux-Bootprotokoll-Details selbst korrekt setzen

### Follow-up

- serielle Konsole bzw. virtio-console Exit-Handling an `aslconsoled` anbinden
- Rootfs als initiale initrd oder als blockorientiertes ASL-Rootdevice
  bereitstellen
- Netzwerkpfad zwischen Linux-Gast und `aslnetd` implementieren
- Boot-Probe von "Kernel gestartet" zu "Guest-Agent ready" weiterentwickeln
