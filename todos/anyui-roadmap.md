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

Es fehlen aber vor allem:

- ausgereifte Text- und Menu-Interaktion
- datengetriebene Collection-Controls
- Drag & Drop und Command-Patterns
- fortgeschrittene Desktop-Controls wie ComboBox, BreadcrumbBar, PropertyGrid
- konsistente Interaktionsmodelle fuer Auswahl, Fokus, Editing, Validation

Bereits angelaufen oder teilweise umgesetzt:

- `TextField`/`TextArea`: `read_only`, Cursor-/Selection-Grundlagen
- `ContextMenu`: Disabled-Items und Keyboard-Bedienung
- Framework-Drag-&-Drop-Grundlagen
- Finder und anyOS Code als erste DnD-Referenz-Apps

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

Fehlend:

- `read_only`
- `max_length`
- Cursor get/set API
- Selection range get/set API
- Undo/Redo
- Word wrap fuer `TextArea`
- Zeilen-/Spaltenposition sauber querybar
- scroll-to-caret
- einheitliche Clipboard-/Selection-Semantik

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

Fehlend:

- Submenues
- Disabled items
- Checkmarks
- Icons
- Shortcut-Anzeige
- Keyboard-Navigation
- Accelerator-Verarbeitung
- bessere Fokus-/Dismiss-Logik

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

Fehlend:

- Keyboard-Navigation
- Multi-Selection
- Scroll-to-item
- Sort callbacks / Sort API
- Inline editing
- Drag & Drop hooks
- Virtualisierung bei grossen Datenmengen

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

### 4. Echte ComboBox bauen

Heute vorhanden:

- `DropDown`
- `AutoCompleteTextField`

Es fehlt:

- ein gemeinsames `ComboBox`-Control mit
- optional editierbarer Eingabe
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

### 7. Drag & Drop als Systemmodell

Nicht nur ein einzelnes Control, sondern ein Framework-Subsystem.

Fehlend:

- Drag source
- Drop target
- Drag payloads (`text`, `uri-list`, intern)
- Hover-Feedback
- copy/move/link negotiation
- auto-scroll bei ScrollView/ListView/TreeView

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

- `ComboBox`
- `ListView`
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

1. `ComboBox`
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

- Text-Controls weiter vervollstaendigen: `max_length`, Undo/Redo, Wrap, scroll-to-caret
- Menues weiter ausbauen: Checkmarks, Icons, Shortcut-Anzeige, Submenues
- `TreeView`/`TableView`/`DataGrid` auf Multi-Selection, bessere Keyboard-Navigation und Auto-scroll ziehen
- DnD von reinem Text auf typisierte Payloads wie `uri-list` und Copy/Move-Aushandlung erweitern

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
