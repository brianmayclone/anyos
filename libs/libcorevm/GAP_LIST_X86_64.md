# CoreVM x86_64 Gap-Liste (Boot Linux/Windows)

Stand: 2026-03-05

## Bereits geschlossen (in diesem Schritt)

- `0F 1F /0` Multi-Byte NOP
- `XGETBV`, `XSETBV`, `RDTSCP`
- `CMPXCHG8B`, `CMPXCHG16B`
- `FXSAVE`, `FXRSTOR`
- CPUID-Erweiterungen (Leaf `7`, `0xD`, `0x80000008`, Featurebits inkl. `CX16`, `RDTSCP`)
- Default-MSR `IA32_APIC_BASE (0x1B)` auf `0xFEE00900`
- Erste `0F 38`/`0F 3A`-Basis: `CRC32`, `PSHUFB`, `PALIGNR`
- `SYSENTER`/`SYSEXIT` (inkl. 32-bit und 64-bit Returnpfad)
- P1-Teile: `MOVBE`, `POPCNT`
- Protected-Interrupt/IRET-Framebreite für 16-bit Gates korrigiert
- SMP-Grundlagen: konfigurierbare vCPU-Anzahl (`corevm_create_ex`), CPUID-Topologie-Reporting (Leaf `1`/`0x0B`)

## P0 (kritisch für stabile Linux/Windows-Bootpfade)

1. `0F 38`/`0F 3A` Decoder + Executor weiter ausbauen (über Basis hinaus)
- Insbesondere weit verbreitete SSE4.1/SSE4.2-Instruction-Familien
- Minimale Abdeckung für Kernel/Bootloader-Runtime und C-Libraries

2. SYSENTER/SYSEXIT härten
- Feinschliff bei Corner-Cases, Segment- und Fault-Semantik nahe SDM

3. FPU/SSE Präzisierung
- Vollständigere `FXSAVE/FXRSTOR`-Layoutgenauigkeit (Tag-Handling, Pointer-Felder)
- Fehlermodell bei ungültigen MXCSR/CR0/CR4-Kombinationen

4. Exception-/Privilege-Semantik härten
- `#GP/#UD/#PF`-Bedingungen näher an SDM
- Ring-Checks für systemnahe Instruktionen und I/O-Pfade
- Task-Gate-/Task-Switch-Pfade für Exceptions (u.a. #DF-Fallback) vervollständigen

## P1 (hohe Relevanz für moderne Userlands)

1. `LZCNT`, `TZCNT` (inkl. CPUID-Gating)
3. `PCLMULQDQ` (häufig in Crypto-Pfaden)
4. `RDRAND` optional (mit klarer CPUID-Policy)

## P2 (Performance/Kompatibilität weiter erhöhen)

1. AVX-Grundgerüst (VEX-Decode, YMM-State)
2. XSAVE/XRSTOR (über FXSAVE/FXRSTOR hinaus)
3. TSC-Ökonomie/Virtualisierung verfeinern (`TSC_AUX`, Skalierung/Offset)

## Geräte-/Plattform-Themen parallel tracken

1. APIC/IOAPIC Timer-/Interruptpfade gegen reale Gastanforderungen prüfen
1.5 SMP-Ausbau: reale Mehr-vCPU-Ausführung + LAPIC pro vCPU + IPI-Routing
2. PCI/ACPI-Tabellenkonsistenz für Windows-Installpfade härten
3. IDE/E1000-Fallbackpfade robust halten (fehlende Option-ROMs etc.)

## Teststrategie (verbindlich)

1. Host-Integrationstests (Linux): BIOS laden, Plattform aufsetzen, POST über Instruktionsbudget laufen lassen
2. Instruktions-Smokes pro Lücke (kleine handkodierte Sequenzen)
3. Später: Golden-Trace-Vergleich gegen Referenzemulator für kritische Instruktionsgruppen
