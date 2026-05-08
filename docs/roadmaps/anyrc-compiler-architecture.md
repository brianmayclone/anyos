# anyrc Compiler Architecture

`anyrc` soll ein richtiger Compiler sein, kein wachsender Satz aus
Sonderfaellen. Dieses Dokument beschreibt die Zielarchitektur fuer den nativen
anyOS-Self-Hosting-Pfad mit `crust` und `ccargo`.

ASL gehoert nicht zu diesem Pfad. `crust` und `ccargo` laufen nativ unter
anyOS und benutzen die anyOS-System-APIs direkt.

## Zielbild

Die Pipeline bleibt klassisch und strikt getrennt:

1. **Lexer und Parser**
   - erzeugen Tokens und AST
   - fuehren keine Namens-, Typ- oder Trait-Entscheidungen aus
   - melden Syntaxfehler als Diagnostics, keine Panics

2. **Cfg und Macro Expansion**
   - entfernt deaktivierte AST-Teile vor der Semantik
   - expandiert deklarative Makros in AST
   - baut keine Typannahmen in Makrocode ein

3. **HIR Lowering**
   - normalisiert Syntax
   - vergibt stabile HIR-IDs und Def-IDs
   - bleibt semantisch arm

4. **Name Resolution**
   - erzeugt eine Def-Map fuer alle Pfade
   - kennt Module, Imports, Crates, Extern-Crates und Prelude
   - unterscheidet Type-, Value- und Macro-Namespace
   - kennt compiler-eigene Language Items nur ueber eine zentrale Tabelle

5. **Type Database**
   - speichert alle Typdefinitionen, Type Aliases, Funktionen, Impl-Bloecke,
     Trait-Definitionen, Associated Items und Crate-Interfaces
   - ist die gemeinsame Datenquelle fuer Typeck, Trait-Solver und MIR-Build

6. **Type Checking**
   - erzeugt Typen fuer Ausdruecke, Patterns und Items
   - erzeugt und loest Inferenzvariablen
   - fuehrt Coercions ueber einen eigenen Coercion-Pfad aus
   - kennt keine Codegen-Symbole

7. **Trait- und Method-Solver**
   - loest Methoden ueber Autoref/Autoderef
   - loest Trait-Impls, Associated Types und Bounds
   - behandelt `Deref`, `Index`, `IntoIterator`, `Iterator`, `From`, `AsRef`
     als Traits, nicht als harte Namen

8. **MIR Build**
   - benutzt nur HIR, ResolveResult und TypeckResult
   - baut Calls auf konkrete Def-IDs
   - fuehrt keine neue Namensauflosung aus

9. **Borrow Checking und MIR Opt**
   - arbeiten auf MIR
   - benutzen TyKind und Def-IDs, keine Quelltextnamen

10. **Codegen und Linker**
    - erhalten MIR mit konkreten Call-Zielen
    - kennen nur Backend-Intrinsics und Runtime-Lang-Items
    - erzeugen ELF fuer anyOS oder Test-ABI

## Was Ab jetzt Nicht Mehr Passieren Soll

- Kein neuer `if method_str == "Vec::..."`-Wildwuchs.
- Kein neues `path.ends_with("::new")` als Typregel.
- Keine Typentscheidungen im Parser.
- Keine Codegen-Symbolnamen in Typeck.
- Keine Stdlib-Fixes, nur damit Compiler-Luecken unsichtbar werden.

Bestehende Kompatibilitaetsstellen duerfen fuer den Moment bleiben, damit der
Compiler weiter testbar ist. Jede neue Arbeit muss sie aber entweder
zentralisieren, durch Crate-Metadaten ersetzen oder als echtes Intrinsic
begründen. Der Fortschritt ist nicht "noch ein Sonderfall", sondern "eine
Schicht weniger, die Sonderfaelle braucht".

## Parser-Regel

Ein richtiger Rust-Parser entscheidet nicht, was `Vec`, `String`, `Iterator`
oder `Deref` bedeuten. Er erkennt nur Syntax:

- Pfade bleiben Pfade.
- Method Calls bleiben Method Calls.
- Generics bleiben Generic Args.
- Attribute bleiben Attribute.
- Fehler werden als Syntax-Diagnostics gemeldet.

Alles Semantische passiert spaeter. Ob ein Pfad ein Typ, Wert, Modul,
Associated Item oder Macro ist, entscheidet die Namensauflösung. Ob ein Call
typisch gueltig ist, entscheidet Typeck. Ob `.foo()` ueber Deref, Trait-Bound
oder inherent impl gefunden wird, entscheidet Method Probe.

## Zulassige Compiler-Knowledge

Ein echter Compiler braucht trotzdem einige bekannte Dinge. Diese muessen aber
zentral und klassifiziert sein:

- Primitive Typen: `u8`, `usize`, `bool`, `str`, ...
- echte Compiler-Intrinsics: `size_of`, Atomics, volatile, inline asm
- Runtime-Lang-Items: Panic/Alloc/Entry glue, wenn noch nicht als normale
  Crate-Symbole modelliert
- Prelude/Lang-Items als Uebergang, solange `core` und `alloc` noch nicht als
  echte Interface-Crates verfuegbar sind

In `anyrc` ist dafuer `libs/anyrc/src/lang_items.rs` die Grenze. Alles, was
dort landet, muss spaeter entweder in echte Crate-Metadaten wandern oder als
echtes Compiler-Intrinsic begruendet bleiben.

## Interne Compiler-APIs

Die naechsten Schnitte sollen diese APIs sichtbar machen:

- `LangItems`: zentrale Tabelle fuer primitive Typen, Prelude-Uebergang und
  echte Compiler-Intrinsics.
- `TyDb`: kanonische Typdatenbank fuer ADTs, Aliases, Traits, Impls,
  Associated Items, Funktionssignaturen und Crate-Interfaces.
- `Coercions`: Klassifikation von Ref/RawPtr, Array/Slice, Unsizing und
  spaeter Trait-Object-Coercions.
- `MethodProbe`: Autoref/Autoderef, inherent impls, Trait-Impls und
  Associated Function Lookup.
- `TraitSolver`: Bounds, Projections und minimale Obligations fuer die
  Rust-Untermenge, die anyOS braucht.
- `Callable`: das Ergebnis von Name/Method/Trait-Aufloesung, das MIR und
  Codegen konsumieren, ohne erneut Quelltextnamen zu interpretieren.

## Naechste Architektur-Meilensteine

0. **Compatibility Tables Einsammeln**
   - verstreute Primitive/Prelude/Runtime-Namen in `lang_items` sammeln
   - echte Runtime-Symbole von normalen Bibliothekssymbolen trennen
   - neue Regeln nur noch hinter `LangItems`, `Coercions` oder spaeter `TyDb`

1. **Crate Interface Loader**
   - `.rlib`-Metadaten duerfen nicht nur gerenderte Rust-Interface-Strings sein.
   - Ziel: strukturierte Symboltabellen fuer Types, Traits, Impls, Aliases,
     Associated Items und Funktionssignaturen.
   - Aktueller Stand: `.rlib` speichert neben dem kompatiblen
     `interface_source` eine strukturierte Interface-Tabelle mit Name, Kind und
     Signatur pro exportiertem Item. Resolver und Typeck konsumieren noch den
     alten Source-Pfad; die Tabelle ist der neue Contract, der jetzt
     schrittweise ausgebaut wird.

2. **Type Database Extrahieren**
   - aus `typeck.rs` eine eigene `tydb`-Schicht machen
   - Resolver und Typeck schreiben/lesen ueber klare APIs

3. **Coercion Engine**
   - Ref zu RawPtr
   - Array zu Slice
   - String zu `str`
   - Autoref fuer Method Calls
   - spaeter Unsizing und Trait Objects

4. **Method Probe**
   - Receiver-Typ normalisieren
   - Autoderef-Kandidaten bilden
   - inherent impls pruefen
   - Trait-Impls pruefen
   - Ergebnis als konkrete Def-ID plus Substitutionen speichern

5. **Trait Solver Minimal**
   - `Index`, `Deref`, `Iterator`, `IntoIterator`, `AsRef`, `From`
   - Associated Type Projection wie `<T as Iterator>::Item`
   - generische Bounds nicht nur speichern, sondern verwenden

6. **Core/Alloc als Normale Crates**
   - `Option`, `Result`, `Vec`, `String`, `Box` kommen aus Interface-Metadaten
   - Methoden und Trait-Impls werden aus Crates geladen
   - Codegen-Intrinsics bleiben nur fuer echte Runtime- oder Maschinenoperationen

## Self-Hosting Gate

Der native Self-Hosting-Pfad gilt erst dann als tragfaehig, wenn diese Gates
gruen sind:

- `libs/anyrc_tests` komplett gruen
- `ccargo build libs/stdlib`
- `ccargo build bin/acargo`
- `ccargo build bin/anyrc`
- `crust` baut eine reduzierte Compiler-Library
- `crust` baut `crust` selbst
