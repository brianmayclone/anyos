// String helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Transition / Animation helpers
// ---------------------------------------------------------------------------

/// Parse a CSS timing-function keyword.
pub(crate) fn parse_timing_function(s: &str) -> TimingFunction {
    match s.trim() {
        "linear" => TimingFunction::Linear,
        "ease-in" => TimingFunction::EaseIn,
        "ease-out" => TimingFunction::EaseOut,
        "ease-in-out" => TimingFunction::EaseInOut,
        "step-start" => TimingFunction::StepStart,
        "step-end" => TimingFunction::StepEnd,
        _ => TimingFunction::Ease,
    }
}

/// Apply a timing function: maps progress `t ∈ [0,1]` to `[0,1]`.
/// Input and output are multiplied by 1000 (fixed-point) to avoid floats.
pub(crate) fn apply_timing(timing: TimingFunction, t: i32) -> i32 {
    // t is in [0, 1000].
    match timing {
        TimingFunction::Linear => t,
        TimingFunction::StepStart => {
            if t > 0 {
                1000
            } else {
                0
            }
        }
        TimingFunction::StepEnd => {
            if t >= 1000 {
                1000
            } else {
                0
            }
        }
        // Cubic bezier approximations (sufficient for browser rendering).
        TimingFunction::EaseIn => {
            // cubic-bezier(0.42, 0, 1, 1) ≈ t³
            let f = t as i64;
            ((f * f * f) / (1_000_000)) as i32
        }
        TimingFunction::EaseOut => {
            // cubic-bezier(0, 0, 0.58, 1) ≈ 1 - (1-t)³
            let inv = (1000 - t) as i64;
            (1000 - (inv * inv * inv / 1_000_000)) as i32
        }
        // Ease and EaseInOut use the same cheap approximation: smoothstep.
        TimingFunction::Ease | TimingFunction::EaseInOut => {
            // smoothstep: 3t² - 2t³
            let f = t as i64;
            ((3 * f * f - 2 * f * f * f / 1000) / 1000) as i32
        }
    }
}

/// Parse a CSS time value (`"0.3s"`, `"300ms"`) to milliseconds.
fn parse_time_ms(s: &str) -> u32 {
    let s = s.trim();
    if let Some(v) = s.strip_suffix("ms") {
        return v.trim().parse::<f32>().map(|f| f as u32).unwrap_or(0);
    }
    if let Some(v) = s.strip_suffix('s') {
        return v
            .trim()
            .parse::<f32>()
            .map(|f| (f * 1000.0) as u32)
            .unwrap_or(0);
    }
    // Pure number — assume seconds if ≤ 10, milliseconds otherwise.
    if let Ok(v) = s.parse::<f32>() {
        return if v <= 10.0 {
            (v * 1000.0) as u32
        } else {
            v as u32
        };
    }
    0
}

/// Parse a `transition` shorthand: `property duration timing delay`.
///
/// Comma-separated layers are each parsed into a `TransitionDef`.
fn parse_transition_shorthand(s: &str) -> Vec<TransitionDef> {
    let mut defs = Vec::new();
    for layer in split_comma_respecting_parens(s) {
        let tokens: Vec<&str> = layer.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let mut def = TransitionDef {
            property: String::from("all"),
            duration_ms: 0,
            timing: TimingFunction::Ease,
            delay_ms: 0,
        };
        let mut time_count = 0u32;
        for tok in &tokens {
            if tok.ends_with("ms") || tok.ends_with('s') {
                let ms = parse_time_ms(tok);
                if time_count == 0 {
                    def.duration_ms = ms;
                } else {
                    def.delay_ms = ms;
                }
                time_count += 1;
            } else if matches!(
                *tok,
                "linear"
                    | "ease"
                    | "ease-in"
                    | "ease-out"
                    | "ease-in-out"
                    | "step-start"
                    | "step-end"
            ) {
                def.timing = parse_timing_function(tok);
            } else if *tok != "none" {
                def.property = tok.to_ascii_lowercase();
            }
        }
        defs.push(def);
    }
    defs
}

/// Parse an `animation` shorthand: `name duration timing delay iterations direction fill-mode`.
///
/// Comma-separated layers each become an `AnimationDef`.
fn parse_animation_shorthand(s: &str) -> Vec<AnimationDef> {
    let mut defs = Vec::new();
    for layer in split_comma_respecting_parens(s) {
        let tokens: Vec<&str> = layer.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let mut def = AnimationDef {
            name: String::new(),
            duration_ms: 0,
            timing: TimingFunction::Ease,
            delay_ms: 0,
            iteration_count: 1,
            alternate: false,
        };
        let mut time_count = 0u32;
        for tok in &tokens {
            if tok.ends_with("ms") || tok.ends_with('s') {
                let ms = parse_time_ms(tok);
                if time_count == 0 {
                    def.duration_ms = ms;
                } else {
                    def.delay_ms = ms;
                }
                time_count += 1;
            } else if matches!(
                *tok,
                "linear"
                    | "ease"
                    | "ease-in"
                    | "ease-out"
                    | "ease-in-out"
                    | "step-start"
                    | "step-end"
            ) {
                def.timing = parse_timing_function(tok);
            } else if *tok == "infinite" {
                def.iteration_count = 0;
            } else if *tok == "alternate" || *tok == "alternate-reverse" {
                def.alternate = true;
            } else if matches!(
                *tok,
                "none"
                    | "normal"
                    | "reverse"
                    | "both"
                    | "forwards"
                    | "backwards"
                    | "running"
                    | "paused"
            ) {
                // Ignore direction/fill-mode/play-state keywords — not yet tracked.
            } else if let Ok(n) = tok.parse::<u32>() {
                def.iteration_count = n;
            } else if !tok.is_empty() && def.name.is_empty() {
                def.name = tok.to_ascii_lowercase();
            }
        }
        if !def.name.is_empty() {
            defs.push(def);
        }
    }
    defs
}

fn eq_ignore_ascii_case(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    for i in 0..ab.len() {
        let ca = if ab[i] >= b'A' && ab[i] <= b'Z' {
            ab[i] + 32
        } else {
            ab[i]
        };
        let cb = if bb[i] >= b'A' && bb[i] <= b'Z' {
            bb[i] + 32
        } else {
            bb[i]
        };
        if ca != cb {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
