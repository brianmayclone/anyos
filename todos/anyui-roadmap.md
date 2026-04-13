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

## Empfohlene Reihenfolge

### Phase 1

- Clipboard-Permissions und Text-Control-Haertung
- `TextField`/`TextArea`: `read_only`, `max_length`, Cursor, Selection
- Menues: Disabled, Checkmarks, Shortcuts, Keyboard-Navigation

### Phase 2

- `TreeView`/`TableView`/`DataGrid` Interaktionsupgrade
- `ComboBox`
- Framework-Drag-&-Drop-Grundlagen

### Phase 3

- `ListView`
- `CollectionView` / `ItemsControl`
- `Popover` / `Sheet` / `Inspector`

### Phase 4

- `PropertyGrid`
- `BreadcrumbBar`
- `TokenField`
- `RichText`
- Command-System

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
