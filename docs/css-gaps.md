# CSS Gaps And Priorities

This document tracks the biggest gaps between `libwebview`'s CSS support and modern CSS modules used by current websites.

## Current Priorities

| Feature | W3C/CSSWG module | Repo status | Priority |
| --- | --- | --- | --- |
| `@container`, `container-type`, `container-name` | CSS Containment Level 3 | Implemented for size queries on named and unnamed ancestor containers | High |
| `:hover`, `:active`, `:focus`, `:focus-visible`, `:focus-within` | Selectors Level 4 | Implemented with runtime selector state and host wiring | High |
| `:has()` descendant matching | Selectors Level 4 | Implemented recursively | High |
| `justify-self`, `place-items`, `place-self`, `place-content` | CSS Box Alignment Level 3 | Implemented in parser/style/grid layout | High |
| `subgrid` | CSS Grid Layout Level 2 | Missing | High |
| Real `@layer` ordering | CSS Cascade Level 5 | Implemented with layer-aware cascade ordering | Medium |
| `transform-origin` | CSS Transforms Level 1 | Implemented for transformed boxes | Medium |
| `object-position` | CSS Images Level 3 | Implemented for replaced elements/image drawing | Medium |
| `mask-image` | CSS Masking Level 1 | Accepted for `@supports`, not rendered | Medium |
| `appearance` | CSS UI Level 4 | Implemented for checkbox/radio/range, single-select, and color `appearance: none` via canvas rendering and hit regions | Medium |
| `background-clip` | CSS Backgrounds and Borders Level 3 | Implemented for `border-box`, `padding-box`, and `content-box` painting | Medium |
| `scroll-behavior` | CSS Overflow Level 3 | Implemented for JS overflow-container scrolling, including smooth animation | Medium |
| `accent-color` | CSS UI Level 4 | Implemented for form control theming and accent-painted widgets | Medium |
| `color-scheme` | CSS Color Adjustment Level 1 | Implemented for light/dark native control fallbacks | Medium |

## Notes

- Dynamic pseudo-classes now round-trip through `WebView` and the surf host, including hover, active, focus, focus-visible, and focus-within.
- `justify-self` currently affects grid item placement. `place-*` shorthands are resolved into the existing alignment model.
- `transform-origin` now affects scaling around non-center origins.
- `object-position` now affects image alignment inside the destination box for `contain`, `cover`, `none`, and `scale-down`.
- `@layer` now participates in cascade ordering, including the reversed precedence rules for `!important`.
- `@container` now resolves against the active ancestor container chain during style resolution, including named containers and inline/block size conditions.
- `accent-color` now flows through parser, computed style, layout and renderer/native control updates.
- `appearance: none` now suppresses native checkbox/radio/range widgets, simple single-select dropdowns, and color inputs, routing them through the canvas-painted control path with preserved interaction, form submission, and reset behavior.
- `background-clip` now affects both background-color and background-image painting rectangles.
- `color-scheme` now influences native control fallback colors and inherited control theming.
- `scroll-behavior` now drives smooth overflow-container scrolling for `scrollTo`/`scrollBy` and `scrollTop`/`scrollLeft` mutations, with explicit JS `behavior` overriding CSS.
