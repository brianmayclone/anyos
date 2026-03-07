# Forger — Minecraft-Klon Design

## Überblick

Forger ist ein Minecraft-Klon im Creative-Modus für anyOS. Software-Rendering über libgl mit adaptiver Sichtweite (Nebel-basiert) für 50+ FPS. Separate Physics-Engine als `libphysics`.

## Entscheidungen

- **Spielmodus**: Creative only (Fliegen, unbegrenzte Blöcke, kein Health/Hunger)
- **Blöcke**: 20 Typen mit prozeduralen Texturen
- **Weltgenerierung**: Perlin-Noise Terrain mit Höhlen und Bäumen
- **Max Sichtweite**: 12 Chunks (192 Blöcke), adaptiv runter bis 4
- **Tag/Nacht**: Dynamischer Zyklus (~10 Min = 1 Tag)
- **Audio**: Keins
- **Architektur**: libphysics als separate Library

## 1. Physics-Engine (`libs/libphysics/`)

### API (extern "C", .so via dynlink)

```
physics_init(world_query_fn: fn(x,y,z) -> bool)
physics_create_player(x, y, z, width, height) -> BodyId
physics_set_velocity(body, vx, vy, vz)
physics_get_position(body) -> (x, y, z)
physics_get_velocity(body) -> (vx, vy, vz)
physics_is_on_ground(body) -> bool
physics_step(dt: f32)
physics_set_gravity(g: f32)  // default -9.81 * 4
```

### Interna

- **AABB-Kollision** gegen Voxel-Grid: Swept-AABB, achsen-separate Auflösung (X→Y→Z)
- **Gravitation**: Konstante Beschleunigung, terminale Velocity ~78 Blöcke/s
- **Boden-Erkennung**: Raycast 0.01 unter Füße → `on_ground` Flag
- **Springen**: Impuls-Velocity wenn `on_ground`
- **Fliegen**: Gravitation deaktivierbar, Doppel-Space Toggle

## 2. Welt-System

### Chunks

- 16×256×16 Blöcke, Block = 1 Byte (ID)
- HashMap<(cx, cz), Chunk>
- Laden/Generieren im Radius um Spieler

### Weltgenerierung

- Simplex Noise (eigene Implementierung), 2 Oktaven
- Höhe 60-100, Meeresspiegel 64
- Bäume: Zufällig auf Gras, Stamm 3-5 + Blätterkrone
- Höhlen: 3D-Noise mit Threshold
- Erze: Kohle<80, Eisen<64, Gold<32, Diamant<16

### Block-Typen (20)

1. Gras, 2. Erde, 3. Stein, 4. Sand, 5. Kies, 6. Holz, 7. Blätter, 8. Wasser, 9. Bedrock, 10. Kohle-Erz, 11. Eisen-Erz, 12. Gold-Erz, 13. Diamant-Erz, 14. Holzplanken, 15. Ziegel, 16. Cobblestone, 17. Schnee, 18. Glas, 19. Crafting Table, 20. Fackel

### Prozedurale Texturen

- 16×16 RGBA, zur Laufzeit generiert
- Noise + Farbpalette pro Block
- Top/Side/Bottom-Varianten (Gras)

## 3. Rendering-Pipeline

### Greedy Meshing

- Nur sichtbare Flächen (zwischen solid und Luft/transparent)
- Benachbarte gleiche Flächen zu größeren Quads zusammenfassen
- Mesh bei Block-Änderung neu bauen, gecacht

### Vertex-Format

- Position (3×f32), UV (2×f32), Normal (3×f32), Light (1×f32) = 36 Bytes
- Textur-Atlas: 4×5 Grid à 16×16 = 64×80 Pixel

### Frustum Culling

- Chunk-AABB gegen View-Frustum, unsichtbare Chunks überspringen

### Distanz-Nebel

- Fragment-Shader: `fog_factor = smoothstep(fog_start, fog_end, distance)`
- `final = mix(block_color, sky_color, fog_factor)`
- fog_start/fog_end dynamisch basierend auf Sichtweite
- Sanfter Übergang, kein hartes Abschneiden

### Adaptive Sichtweite

- FPS über letzte 10 Frames mitteln
- avg < 50 → Sichtweite -0.5 Chunks (min 4)
- avg > 55 → Sichtweite +0.5 Chunks (max 12)
- Fog-Distanz folgt smooth (lerp pro Frame)

### Himmel & Tag/Nacht

- Fullscreen-Quad Gradient (dunkelblau → hellblau, Nacht: dunkelblau → schwarz)
- Sonne/Mond: heller Kreis via Shader
- Zyklus 10 Min real = 1 Tag
- Directional Light rotiert, Ambient interpoliert (Tag 0.6, Nacht 0.15)

## 4. Spieler-Interaktion & UI

### Steuerung

- WASD: Bewegung, Maus: Kamera, Space: Springen/Auf, Shift: Ab
- Doppel-Space: Flugmodus Toggle
- Linksklick: Block abbauen (DDA Raycast, max 5 Blöcke)
- Rechtsklick: Block platzieren
- Mausrad/1-9: Block wählen

### HUD

- Crosshair (Zentrum), Hotbar (unten, 9 Slots)
- FPS + Position + Sichtweite oben links
- Drahtrahmen um angepeilten Block

## 5. Projekt-Struktur

```
libs/libphysics/
  Cargo.toml, src/lib.rs, src/aabb.rs, src/body.rs, src/world_query.rs

libs/libphysics_client/
  Cargo.toml, src/lib.rs

apps/forger/
  Cargo.toml, build.rs, Info.conf
  src/main.rs      — Entry, Event-Loop, Fenster
  src/world.rs     — Chunk-System, Weltgenerierung
  src/noise.rs     — Simplex Noise
  src/mesh.rs      — Greedy Meshing
  src/render.rs    — GL-Calls, Shader, Kamera, Fog, Himmel
  src/player.rs    — Spieler-Logik, Steuerung, Raycast
  src/textures.rs  — Prozedurale Texturen
  src/ui.rs        — HUD, Hotbar, Crosshair
  src/block.rs     — Block-Definitionen
```

### Abhängigkeiten

- forger → libgl_client, libphysics_client, libanyui_client, anyos_std, dynlink
- libphysics → libsyscall, libm
