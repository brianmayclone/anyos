# anyUI Roadmap

## Ziel

Diese Datei sammelt die fehlenden anyUI-Bausteine fuer:

- Entwickler-Dogfooding
- Endnutzer-taugliche Desktop-Apps
- eine reifere Desktop-Interaktion auf dem Niveau klassischer GUI-Toolkits

Der Fokus liegt nicht nur auf "mehr Controls", sondern auf wiederverwendbaren
Interaktionsmustern, Datenbindung, Keyboard-/Pointer-UX und Desktop-typischer
Produktivitaet.

---

## Statusbild

anyUI hat bereits eine breite Basis mit 44 Controls, Dialogen, Menubar,
TrayIcon, Windowing, Accessibility und Popup-/Modal-Grundlagen.

Status-Update 2026-04-23 nach Code-Verifikation: ~35% der Roadmap umgesetzt.
Stark: Text-Controls (Cursor, Selection, read_only, max_length, Clipboard).
Kollection-Controls (Multi-Selection, Sort-API, Scroll-to-item in DataGrid).
ComboBox existiert bereits. Schwach: Undo/Redo, Submenues, Accelerators,
Popover/Sheet/Drawer, generisches Drag&Drop, Virtualisierung, RichText,
Command-System, Validation.

Es fehlen aber vor allem:

- Undo/Redo in Text-Controls, Word-Wrap, public scroll-to-caret
- Submenues, Checkmarks, Accelerator-Anzeige und -Verarbeitung in Menues
- Virtualisierung und Inline-Editing in Kollection-Controls
- generisches Drag & Drop-Framework (nur DataGrid-interne DragMode vorhanden)
- fortgeschrittene Desktop-Controls: `ListView`, `CollectionView`, `Popover`,
  `Sheet`, `Drawer`, `PropertyGrid`, `BreadcrumbBar`, `RichText`
- konsistente Interaktionsmodelle fuer Fokus, Editing, Validation
- Command-System (zentrale Registry, Shortcut-Bindings)

Bereits umgesetzt (aus "angelaufen" inzwischen fertig):

- `TextField`/`TextArea`/`TextEditor`: `read_only`, `max_length`, Cursor
  get/set, Selection-Range get/set (public API)
- `ContextMenu`: Disabled-Items (`\x1D` Prefix), Icons (`\x1F` Separator)
- `ComboBox`: eigenes Control in libanyui + libanyui_client vorhanden
- `DataGrid`: Multi-Selection, Sort-API (`sort_by`, `set_column_sort_type`),
  `scroll_to_row`, Column-Reorder/Resize
- `ListBox`: Multi-Select-Flag, Keyboard-Nav (Up/Down/Space/Enter)
- `TreeView`: Keyboard-Navigation (Up/Down)
- einheitliche Clipboard-Semantik via `compositor::clipboard_set/get`

Noch nicht angefangen (trotz Roadmap-Erwähnung):

- (keine — Drag & Drop-Framework ist inzwischen implementiert, siehe P1 §7)

---

## P0

Diese Punkte sind der schnellste Hebel fuer alle bestehenden Apps.

### 1. Text Controls fertig machen

Betroffene Controls:

- `TextField`
- `TextArea`
- `TextEditor`
- `SearchField`
- `AutoCompleteTextField`

Status:

- ✅ `read_only` (TextField/TextArea/TextEditor)
- ✅ `max_length` (TextField/TextArea)
- ✅ Cursor get/set API (public)
- ✅ Selection range get/set API (public)
- ✅ einheitliche Clipboard-Semantik (`compositor::clipboard_*`)
- ❌ Undo/Redo
- ❌ Word wrap fuer `TextArea`
- ❌ Zeilen-/Spaltenposition sauber querybar
- ⚠️ scroll-to-caret — `ensure_cursor_visible()` intern vorhanden, nicht public

Apps mit Sofortnutzen:

- anyOS Code
- Notepad
- anyMail
- Surf
- Settings
- File dialogs

Warum P0:

- Das hebt sofort Editor, Formulare, Dialoge und Suchfelder.
- `max_length` ist zugleich auch eine Sicherheits- und Stabilitaetsmassnahme.

### 2. Menues desktop-tauglich machen

Betroffene Controls:

- `MenuBar`
- `ContextMenu`
- popup-basierte Menues

Status:

- ✅ Disabled items (ContextMenu mit `\x1D` Prefix)
- ✅ Icons (ContextMenu mit `\x1F` Separator)
- ✅ Keyboard-Navigation (Up/Down in TreeView/Menus)
- ❌ Submenues
- ❌ Checkmarks
- ❌ Shortcut-Anzeige (Accelerator-Text rechts)
- ❌ Accelerator-Verarbeitung (Alt+Key / globale Shortcuts)
- ❌ bessere Fokus-/Dismiss-Logik

Apps mit Sofortnutzen:

- Finder
- anyOS Code
- Surf
- Notepad
- Taskmanager
- Store

Warum P0:

- Menues sind in Desktop-Systemen kein Luxus, sondern Kerninteraktion.
- Viele Apps koennen dadurch Funktionen exponieren, die heute nur Toolbar-
  oder Sonderlogik sind.

### 3. Listen, Trees und Grids interaktiv aufwerten

Betroffene Controls:

- `TreeView`
- `TableView`
- `DataGrid`
- `ListBox`

Status:

- ✅ Keyboard-Navigation (TreeView/ListBox: Up/Down/Space/Enter)
- ✅ Multi-Selection (DataGrid `SelectionMode::Multi`, ListBox multi-flag)
- ✅ Scroll-to-item (DataGrid `scroll_to_row`)
- ✅ Sort API (DataGrid `sort_by`, `set_column_sort_type`)
- ⚠️ Drag & Drop hooks — nur DataGrid-interne DragMode (Reorder/Resize)
- ❌ Inline editing
- ❌ Virtualisierung bei grossen Datenmengen
- ❌ generischer Drag&Drop mit Payloads

Apps mit Sofortnutzen:

- Finder
- anyOS Code
- anyMail
- Store
- Updater
- Taskmanager
- Event Viewer

Warum P0:

- Diese Controls tragen einen grossen Teil klassischer Desktop-Apps.
- Ohne diese Features fuehlen viele Anwendungen "demoartig" statt produktiv.

---

## P1

Diese Punkte sind die naechste Schicht fuer eine moderne und skalierbare GUI.

### 4. ComboBox (✅ Control existiert, Feinschliff offen)

Heute vorhanden:

- `DropDown`
- `AutoCompleteTextField`
- ✅ `ComboBox` (libanyui/combobox.rs + libanyui_client/combobox.rs)

Noch zu pruefen / haerten:

- optional editierbare Eingabe
- Filtering
- Selection + freier Eingabe
- Keyboard-Navigation
- Popup-Owner-Logik
- Validation

Apps mit Nutzen:

- Settings
- Installer
- Mail
- Browser-Formulare
- Such- und Filterpanels

### 5. Explorer-/Desktop-ListView

Neues Kern-Control:

- `ListView`

Ansichten:

- icon view
- list view
- details/columns
- optional tiles

Fehlend:

- Auswahlrahmen
- In-place Rename
- Multi-Selection
- Sortierung
- Column headers
- Keyboard-Typeselektion
- Virtualisierung
- Drag & Drop

Apps mit Nutzen:

- Finder
- anyzilla
- Store
- Dateidialoge
- Fontviewer/Iconview

### 6. CollectionView / ItemsControl / Repeater

Neues Framework-Primitive fuer datengetriebene Oberflaechen.

Ziel:

- grosse Datenmengen oder Karten-/Listen-Layouts nicht mehr manuell per
  `FlowPanel` und Einzelsubcontrols bauen
- wiederverwendbares Item-Rendering
- spaeter Virtualisierung
- Selektion, Fokus und Commands frameworkweit vereinheitlichen

Apps mit Nutzen:

- Store
- Mail
- Finder
- Notifications
- Launcher / Runner
- Settings-Listen

### 7. Drag & Drop als Systemmodell ✅

Framework-Subsystem, inzwischen vorhanden.

Implementiert:

- ✅ Drag source (`Control::set_draggable`)
- ✅ Drop target (`Control::set_drop_target`, `set_drop_formats`)
- ✅ Typisierte Payloads (`DND_FORMAT_TEXT`/`URI_LIST`/`FILES`/`CUSTOM`,
  `drag_set_payload` / `drag_get_payload`)
- ✅ Hover-Feedback (framework-gesetztes `drop_hover`, blaue Umrandung auf
  aktivem Drop-Target)
- ✅ Copy/Move/Link-Negotiation (`drag_accept(effects)`, Ctrl=Copy /
  Shift=Move / Ctrl+Shift=Link, Auflösung in `dnd::negotiate_effect`)
- ✅ Auto-Scroll bei ScrollView (Edge-Zone 24 px, linearer Ramp via
  `dnd::autoscroll_delta`, Trait-Hook `Control::drag_autoscroll`)
- ✅ Cursor-Feedback (Compositor `CursorShape::Move` während aktiver Drag)
- ✅ Host-Testcrate (`libs/libanyui_dnd_tests/`, 22 Tests für Payload-Mask,
  Effect-Negotiation und Auto-Scroll-Mathematik)
- ✅ Referenz-Demo (`apps/demo_anyui` Tab "DnD" mit 4 Sektionen: Text-Drag,
  List-Reorder, URI-List, Effect-Negotiation)

Noch offen:

- Drag-Image-Ghost (visuelles Preview-Rendering am Cursor während Drag)
- Cross-Window-DnD über Compositor-Protokoll
- Horizontales Auto-Scroll (derzeit nur vertikal)

Apps mit Nutzen:

- Finder
- anyOS Code
- anyMail
- Paint
- Surf

### 8. Window-/Popup-/Popover-Familie

Vorhanden:

- Fenster
- Modals
- Popup-Fenster fuer Menues

Fehlend:

- `Popover`
- `Sheet`
- `Drawer`
- `Inspector`
- ankergestutzte Popup-Positionierung
- Escape-/Outside-click-/Focus-loss-Semantik als Standard

Apps mit Nutzen:

- Settings
- Finder
- anyOS Code
- Browser
- VM Manager

---

## P2

Diese Punkte machen das Toolkit "reich" und langfristig konkurrenzfaehig.

### 9. PropertyGrid / Inspector

Neues Desktop-Control fuer Eigenschaftseditoren.

Faehigkeiten:

- Name/Wert-Zeilen
- Typabhaengige Editoren
- Gruppen/Sektionen
- Inline-Validation
- Expand/Collapse

Apps mit Nutzen:

- VM Manager
- Diagnostics
- anyTrace
- spaeter Designer/Devtools

### 10. BreadcrumbBar / PathBar / TokenField

Apple-/Desktop-typische Produktivitaetscontrols.

Fehlend:

- `BreadcrumbBar`
- `PathBar`
- `TokenField` / `TagPicker`

Apps mit Nutzen:

- Finder
- anyzilla
- anyMail
- Store/Settings-Filter

### 11. RichText / AttributedText

Zwischen `Label` und `WebView` fehlt ein leichtgewichtiges Rich-Text-Control.

Faehigkeiten:

- mehrere Styles in einem Block
- Links
- Inline-Icons
- Selection / Copy
- Markup-light

Apps mit Nutzen:

- Markdown Viewer
- Mail
- Browser-Chrome
- Hilfetexte
- Notifications

### 12. Command-/Action-System

Das ist kein sichtbares Control, aber ein wichtiges Architekturteil.

Fehlend:

- zentrale Commands
- Shortcut-Bindings
- Enabled/Disabled-Zustand aus einer Quelle
- Menu + Toolbar + ContextMenu auf denselben Command mappen

Apps mit Nutzen:

- anyOS Code
- Finder
- Notepad
- Surf
- Taskmanager

### 13. Validation / Forms / Error Presentation

Fehlend:

- Validatoren fuer Eingaben
- Inline-Error-Zustand
- Form-level summary
- async validation hooks

Apps mit Nutzen:

- Installer
- Mail
- Settings
- Login
- Netzwerkdialoge

---

## Fehlende Smarte Controls im Vergleich zu WinForms / WPF / AppKit

### Klar fehlend

- `ListView` (nicht mit `ListBox` zu verwechseln)
- `PropertyGrid`
- `BreadcrumbBar`
- `TokenField`
- `Popover`
- `Sheet`
- `Inspector`
- `ItemsControl` / `CollectionView`
- `RichText` / `TextBlock`

### Vorhanden, aber noch zu schwach

- `TextField`
- `TextArea`
- `TreeView`
- `TableView`
- `DataGrid`
- `DropDown`
- `MenuBar`
- `ContextMenu`
- `Toolbar`
- `ProgressBar`
- `Slider`
- `Label`
- `Window`

### Als Komposition moeglich, aber besser als First-Class-Control

- Command Palette
- SearchableList
- File picker mit Sidebar/Breadcrumbs
- Inspector panel
- Tag editor
- validation forms

---

## Interaktivitaetsluecken

Diese Punkte fehlen quer ueber viele Controls:

- konsistente Keyboard-Navigation
- konsistente Fokus-Ringe und Tab-Order
- Drag & Drop
- Multi-Selection
- Inline-Editing
- Hover-/Pressed-/Selected-/Disabled-State-Design
- Accessibility-Rollen jenseits des Grundmodells
- Virtualisierung bei grossen Collections
- Commands statt nur nackte Callbacks
- Validation-Feedback

---

## Konkrete Prioritaeten

Diese Reihenfolge ist an zwei Dingen ausgerichtet:

- maximaler Sofortnutzen fuer bestehende Apps
- moeglichst wenig Wegwerf-Arbeit fuer spaetere groessere Controls

### Jetzt als naechstes

1. ~~`ComboBox`~~ ✅ erledigt -- nur noch Feinschliff (Filter/Validation)
2. `ListView`
3. `CollectionView` / `ItemsControl`
4. `Popover` / `Sheet`
5. `PropertyGrid`
6. `BreadcrumbBar` / `PathBar`
7. `RichText`

### Warum genau diese Reihenfolge

#### 1. `ComboBox`

Warum zuerst:

- kleinster fehlender Desktop-Baustein mit sehr hohem Wiederverwendungswert
- kann auf vorhandenem `DropDown`-, `TextField`- und Popup-Code aufbauen
- schliesst eine sichtbare Luecke gegenueber WinForms, WPF und AppKit schnell

Direkter Nutzen:

- Settings
- Installer
- Mail
- Browser-Formulare
- Such- und Filterleisten

Technische Vorarbeit:

- Popup-Owner-/Dismiss-Logik weiter haerten
- Auswahlmodell zwischen Text und Listen-Popup vereinheitlichen
- Validation-Hooks vorsehen

#### 2. `ListView`

Warum direkt danach:

- bringt Finder, Dateidialoge und mehrere System-Apps sofort auf ein erwachseneres Niveau
- nutzt das neue DnD-System unmittelbar weiter
- verhindert, dass jede App ihre Explorer-Ansicht weiter ad hoc zusammensetzt

Direkter Nutzen:

- Finder
- Store
- anyzilla
- Dateidialoge
- Medien- und Asset-Browser

Technische Vorarbeit:

- Multi-Selection fuer Collections
- Column-/Header-Modell
- Type-to-select
- In-place Rename
- Auto-scroll waehrend Drag

#### 3. `CollectionView` / `ItemsControl`

Warum vor weiteren Spezial-Controls:

- ist das eigentliche Framework-Primitive fuer datengetriebene UIs
- entlastet `FlowPanel`-Bastelei und schafft eine Basis fuer spaetere Controls
- hilft sofort bei Store-, Settings-, Mail- und Launcher-Oberflaechen

Direkter Nutzen:

- Store
- Settings
- anyMail
- Notifications
- Launcher / Runner

Technische Vorarbeit:

- Item-Adapter oder Data-Source-Modell
- Selektion und Fokus fuer wiederholte Items
- spaetere Virtualisierung vorbereiten

#### 4. `Popover` / `Sheet`

Warum erst hier:

- sehr wertvoll fuer moderne Desktop-Interaktion, aber weniger zentral als Auswahl- und Collection-Controls
- sollte auf bereits verbesserter Popup-, Fokus- und Command-Logik aufsetzen

Direkter Nutzen:

- Finder
- anyOS Code
- Browser-Chrome
- Settings
- VM Manager

Technische Vorarbeit:

- ankerbasierte Positionierung
- standardisierte Dismiss-Regeln
- Fokus-Rueckgabe an den Ausloeser

#### 5. `PropertyGrid`

Warum danach:

- sehr stark fuer Entwickler- und Admin-Tools
- haengt von den vorherigen Editing- und Selection-Verbesserungen ab

Direkter Nutzen:

- VM Manager
- anyTrace
- Diagnostics
- spaetere Designer und Devtools

#### 6. `BreadcrumbBar` / `PathBar`

Warum nicht frueher:

- hohe UX-Wirkung, aber geringere Breitenwirkung als `ComboBox`, `ListView` und `CollectionView`
- profitiert von Popover- und Command-Verbesserungen

Direkter Nutzen:

- Finder
- Dateidialoge
- anyzilla

#### 7. `RichText`

Warum zuletzt in diesem Block:

- wichtig, aber weniger unblockend fuer Desktop-Produktivitaet als die Controls davor
- wird sauberer, wenn Selection-, Clipboard- und Command-Semantik vorher vereinheitlicht sind

Direkter Nutzen:

- anyMail
- Hilfetexte
- Notifications
- Markdown-/Dokumenten-Viewer

### Was parallel weiterlaufen sollte

- Text-Controls weiter vervollstaendigen: Undo/Redo, Word-Wrap (TextArea),
  scroll-to-caret als public API (`max_length` ist bereits fertig)
- Menues weiter ausbauen: Checkmarks, Shortcut-Anzeige, Submenues, Accelerators
  (Icons und Disabled Items sind bereits fertig)
- `TreeView`/`TableView`/`DataGrid`: Inline-Editing, Virtualisierung,
  Auto-scroll waehrend Drag (Multi-Selection, Sort und Keyboard-Nav fertig)
- DnD-Framework von Null aufbauen: Drag-source/Drop-target, typisierte
  Payloads (`text`, `uri-list`), Copy/Move-Aushandlung

---

## Definition of Done pro neuem Control

Ein Control gilt erst dann als "fertig", wenn es mindestens hat:

- Maus-Interaktion
- Keyboard-Navigation
- Fokus-Verhalten
- Disabled-State
- Theming
- Accessibility-Tree-Unterstuetzung
- sinnvolle Public API im Client-Wrapper
- mindestens eine produktive Referenz-App

---

## Referenz-Apps fuer Dogfooding

Diese Apps sollten als Messlatte dienen:

- Finder
- anyOS Code
- Notepad
- Surf
- anyMail
- Store
- Settings
- Taskmanager

Wenn eine neue anyUI-Funktion in mindestens zwei dieser Apps sofort klaren
Mehrwert schafft, ist sie sehr wahrscheinlich die richtige Prioritaet.
