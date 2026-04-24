# ADR-0007 - ASL uses SeaBIOS as userspace firmware for PC boot profiles

## Status

Accepted

## Date

2026-04-24

## Context

ASL soll wie WSL2 ein Subsystem mit einem schlanken, kontrollierten
Userland-VMM sein. Gleichzeitig brauchen bestimmte Images und Bootpfade ein
PC-kompatibles Firmware-Modell: Reset Vector, Real Mode Startzustand und
spaeter klassische Bootgeraete. Das darf nicht in den Kernel wandern, weil AVM
das Pendant zu KVM bleiben soll: VM/vCPU/Memory/Register/Exit-Primitives, aber
kein BIOS und kein PC-Emulator im Kernel.

ADR-0006 beschreibt weiterhin den direkten Linux-Bootpfad fuer Utility-VMs.
Dieser Pfad ist nuetzlich, wenn `asld` Kernel und initrd selbst laedt. Er ist
aber nicht ausreichend fuer ASL-Profile, die ein firmwaregetriebenes PC-Booten
brauchen.

## Decision

ASL fuehrt ein separates Kernel-Profil `seabios-x86_64` ein. Dieses Profil
bootet nicht ueber einen Kernel-Loader im Kernel, sondern ueber `asld` als
Userspace-VMM:

- SeaBIOS wird im Quellbaum unter `third_party/seabios/seabios.bin`
  versioniert und vom Build nach `/System/var/asl/firmware/seabios.bin`
  installiert
- `asld` prueft dieses Artefakt im Bootplan und meldet es in der Diagnose als
  `boot_firmware`
- `asld` kopiert das Firmware-Image an das obere Ende des ersten MiB
  Guest-Speicher
- `asld` setzt die vCPU auf den x86 Reset Vector `f000:fff0`
- AVM fuehrt danach nur den Gast aus und liefert VM-Exits an `asld`

Damit liegt die Firmware analog zu KVM/QEMU oder WSL im Userspace. Der Kernel
erhaelt keine SeaBIOS-spezifische Logik und keine BIOS-Services.

`aslctl create` kann das Profil explizit setzen:

```text
aslctl create <name> <image-ref> <owner> --kernel-profile seabios-x86_64
```

## Consequences

### Positive

- ASL bekommt einen realistischen PC-kompatiblen Bootpfad, ohne AVM
  aufzublaehen
- Firmware-Aktualisierung und Packaging bleiben Userspace-Aufgaben
- der direkte Linux-Bootpfad und der firmwarebasierte Bootpfad koennen
  nebeneinander getestet und diagnostiziert werden
- die KVM-aehnliche Trennung bleibt klar: Kernel stellt Virtualisierung,
  `asld` stellt Firmware, Geraetemodell und Bootpolitik

### Negative

- SeaBIOS allein reicht nicht fuer vollstaendige PC-Images; `asld` braucht
  noch ein tragfaehiges Geraetemodell fuer Block, Netzwerk, Timer und Konsole
- das SeaBIOS-Artefakt muss beim Systembau oder bei der Installation nach
  `/System/var/asl/firmware/seabios.bin` gelangen
- echte Hardwaretests muessen bestaetigen, dass der gesetzte Real-Mode
  vCPU-Zustand von AVM/VMX akzeptiert und stabil ausgefuehrt wird

### Follow-up

- SeaBIOS-Artefakt bei Updates bewusst gegen eine neue vendored Version
  tauschen und die Herkunft dokumentieren
- Boot-Disk oder virtio-blk/aehnlichen Blockpfad fuer firmwarebasierte Boots
  anbinden
- Netzwerk- und Konsolenpfade fuer den firmwarebasierten Bootpfad vollstaendig
  an `aslnetd` und `aslconsoled` koppeln
- VM-Exit-Kompatibilitaet mit SeaBIOS auf echter AVM-Hardware testen
