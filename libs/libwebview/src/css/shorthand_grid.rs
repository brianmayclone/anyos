fn expand_grid_template_shorthand(value_str: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();
    let s = value_str.trim();

    if let Some(slash_pos) = find_grid_template_slash(s) {
        let rows_str = s[..slash_pos].trim();
        let cols_str = s[slash_pos + 1..].trim();

        // If rows_str contains quoted strings, it's the interleaved areas+rows format:
        // "area col1 col2" row-track-size ...
        if rows_str.contains('\'') || rows_str.contains('"') {
            let (area_rows, row_tracks) = parse_interleaved_areas_rows(rows_str);
            if !area_rows.is_empty() {
                // Join quoted area strings into one grid-template-areas value.
                let mut areas_val = String::new();
                for (i, r) in area_rows.iter().enumerate() {
                    if i > 0 {
                        areas_val.push(' ');
                    }
                    areas_val.push_str(r);
                }
                decls.push(Declaration {
                    property: Property::GridTemplateAreas,
                    value: CssValue::Keyword(areas_val),
                    important: false,
                });
            }
            if !row_tracks.is_empty() {
                let tracks_val = row_tracks.join(" ");
                decls.push(Declaration {
                    property: Property::GridTemplateRows,
                    value: CssValue::Keyword(tracks_val),
                    important: false,
                });
            }
        } else if !rows_str.is_empty() {
            decls.push(Declaration {
                property: Property::GridTemplateRows,
                value: CssValue::Keyword(String::from(rows_str)),
                important: false,
            });
        }
        if !cols_str.is_empty() {
            decls.push(Declaration {
                property: Property::GridTemplateColumns,
                value: CssValue::Keyword(String::from(cols_str)),
                important: false,
            });
        }
    } else {
        // No slash — might be just rows or areas.
        if s.contains('\'') || s.contains('"') {
            return expand_grid_template_areas(s);
        }
        decls.push(Declaration {
            property: Property::GridTemplateRows,
            value: CssValue::Keyword(String::from(s)),
            important: false,
        });
    }
    decls
}

/// Parse the interleaved `grid-template` rows format:
/// `"area col1 col2" track-size "area col1 col2" track-size ...`
///
/// Returns (Vec of quoted area row strings, Vec of row track size strings).
/// Area rows without an explicit track size get "auto" inserted.
fn parse_interleaved_areas_rows(s: &str) -> (Vec<String>, Vec<String>) {
    let mut area_rows: Vec<String> = Vec::new();
    let mut row_tracks: Vec<String> = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Skip whitespace and newlines.
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        if bytes[i] == b'\'' || bytes[i] == b'"' {
            // Quoted area row: collect including the quotes.
            let quote = bytes[i];
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            } // skip closing quote
            area_rows.push(String::from(&s[start..i]));

            // Look ahead: is the next non-whitespace token a track size (not a quote)?
            let mut j = i;
            while j < bytes.len() && matches!(bytes[j], b' ' | b'\t' | b'\n' | b'\r') {
                j += 1;
            }
            if j < bytes.len() && bytes[j] != b'\'' && bytes[j] != b'"' {
                // Consume the track size token (respects parentheses).
                let track_start = j;
                let mut depth: u32 = 0;
                while j < bytes.len() {
                    match bytes[j] {
                        b'(' => depth += 1,
                        b')' => {
                            if depth > 0 {
                                depth -= 1;
                            }
                        }
                        b' ' | b'\t' | b'\n' | b'\r' if depth == 0 => break,
                        _ => {}
                    }
                    j += 1;
                }
                if track_start < j {
                    row_tracks.push(String::from(&s[track_start..j]));
                } else {
                    row_tracks.push(String::from("auto"));
                }
                i = j;
            } else {
                // No explicit row track size — use auto.
                row_tracks.push(String::from("auto"));
            }
        } else {
            // Non-quoted token outside an area row — skip (e.g. line names [...]).
            while i < bytes.len() && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
                i += 1;
            }
        }
    }

    (area_rows, row_tracks)
}

/// Find the '/' in a grid-template value that separates rows from columns.
/// Must skip '/' inside parentheses (e.g. `minmax(0,1fr)`).
fn find_grid_template_slash(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth: u32 = 0;
    let mut in_quote = false;
    let mut quote_char: u8 = 0;
    for i in 0..bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => {
                if !in_quote {
                    in_quote = true;
                    quote_char = bytes[i];
                } else if bytes[i] == quote_char {
                    in_quote = false;
                }
            }
            b'(' if !in_quote => depth += 1,
            b')' if !in_quote => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b'/' if !in_quote && depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Parse `grid-template-areas` value.
/// Example: `'header header' 'sidebar content' 'footer footer'`
/// Each quoted string defines one row. Area names map to grid positions.
/// Emits a GridTemplateAreas keyword value that the style resolver will parse.
fn expand_grid_template_areas(value_str: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();
    // Store the raw areas string as a keyword — the grid layout engine will parse it.
    decls.push(Declaration {
        property: Property::GridTemplateAreas,
        value: CssValue::Keyword(String::from(value_str.trim())),
        important: false,
    });
    decls
}

fn expand_font_shorthand(value_str: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    if parts.is_empty() {
        return decls;
    }

    let style_weight_keywords = [
        "normal",
        "italic",
        "oblique", // font-style
        "bold",
        "bolder",
        "lighter",    // font-weight
        "small-caps", // font-variant
        "100",
        "200",
        "300",
        "400",
        "500",
        "600",
        "700",
        "800",
        "900",
    ];

    let mut font_size_idx = None;
    for (i, part) in parts.iter().enumerate() {
        let lower = part.to_ascii_lowercase();
        // font-style / font-weight / font-variant keywords → skip
        if style_weight_keywords.contains(&lower.as_str()) {
            // Emit font-weight if bold
            if lower == "bold" || lower == "bolder" {
                decls.push(Declaration {
                    property: Property::FontWeight,
                    value: CssValue::Keyword(String::from("bold")),
                    important: false,
                });
            } else if lower == "italic" || lower == "oblique" {
                decls.push(Declaration {
                    property: Property::FontStyle,
                    value: CssValue::Keyword(lower),
                    important: false,
                });
            }
            continue;
        }
        // This must be the font-size (possibly with /line-height)
        font_size_idx = Some(i);
        break;
    }

    if let Some(si) = font_size_idx {
        let size_part = parts[si];
        // Handle size/line-height (e.g. "14px/1.5")
        let (size_str, lh_str) = if let Some(slash) = size_part.find('/') {
            (&size_part[..slash], Some(&size_part[slash + 1..]))
        } else {
            (size_part, None)
        };

        let size_val = parse_property_value_ast(&Property::FontSize, size_str);
        decls.push(Declaration {
            property: Property::FontSize,
            value: size_val,
            important: false,
        });

        if let Some(lh) = lh_str {
            let lh_val = parse_property_value_ast(&Property::LineHeight, lh);
            decls.push(Declaration {
                property: Property::LineHeight,
                value: lh_val,
                important: false,
            });
        }

        // Everything after the font-size is the font-family
        if si + 1 < parts.len() {
            let family = parts[si + 1..].join(" ");
            decls.push(Declaration {
                property: Property::FontFamily,
                value: CssValue::Keyword(family),
                important: false,
            });
        }
    }

    decls
}

// ---------------------------------------------------------------------------
