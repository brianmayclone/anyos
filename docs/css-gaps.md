# CSS Gaps And Priorities

This document tracks the biggest gaps between `libwebview`'s CSS support and modern CSS modules used by current websites.

## Current Priorities

| Feature | W3C/CSSWG module | Repo status | Priority |
| --- | --- | --- | --- |
| `@container`, `container-type`, `container-name` | CSS Containment Level 3 | Missing | High |
| `:hover`, `:active`, `:focus`, `:focus-visible`, `:focus-within` | Selectors Level 4 | Implemented with runtime selector state and host wiring | High |
| `:has()` descendant matching | Selectors Level 4 | Implemented recursively | High |
| `justify-self`, `place-items`, `place-self`, `place-content` | CSS Box Alignment Level 3 | Implemented in parser/style/grid layout | High |
| `subgrid` | CSS Grid Layout Level 2 | Missing | High |
| Real `@layer` ordering | CSS Cascade Level 5 | Implemented with layer-aware cascade ordering | Medium |
| `transform-origin` | CSS Transforms Level 1 | Implemented for transformed boxes | Medium |
| `object-position` | CSS Images Level 3 | Implemented for replaced elements/image drawing | Medium |
| `mask-image` | CSS Masking Level 1 | Accepted for `@supports`, not rendered | Medium |
| `scroll-behavior` | CSS Overflow Level 3 | Parsed but not applied | Medium |
| `accent-color` | CSS UI Level 4 | Implemented for form control theming and accent-painted widgets | Medium |
| `color-scheme` | CSS Color Adjustment Level 1 | Implemented for light/dark native control fallbacks | Medium |

## Notes

- Dynamic pseudo-classes now round-trip through `WebView` and the surf host, including hover, active, focus, focus-visible, and focus-within.
- `justify-self` currently affects grid item placement. `place-*` shorthands are resolved into the existing alignment model.
- `transform-origin` now affects scaling around non-center origins.
- `object-position` now affects image alignment inside the destination box for `contain`, `cover`, `none`, and `scale-down`.
- `@layer` now participates in cascade ordering, including the reversed precedence rules for `!important`.
- `accent-color` now flows through parser, computed style, layout and renderer/native control updates.
- `color-scheme` now influences native control fallback colors and inherited control theming.
