include!("at_rules_media/rules.rs");
include!("at_rules_media/media_query.rs");
include!("at_rules_media/lengths.rs");

/// Result of parsing a @supports block: plain rules + nested @media rules.
struct SupportsResult {
    rules: Vec<Rule>,
    media_rules: Vec<MediaRule>,
}
