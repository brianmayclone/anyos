// CSS tokenizer + parser for surf browser
// no_std compatible, uses alloc for String/Vec

use alloc::boxed::Box;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

use crate::dom::Tag;

include!("types.rs");

include!("ast.rs");
include!("lexer.rs");
include!("parser_core.rs");
include!("parser_ast.rs");
include!("stylesheet.rs");
include!("at_rules_media.rs");
include!("at_rules.rs");
include!("selectors.rs");
include!("declarations.rs");
include!("values.rs");
include!("shorthand.rs");
include!("shorthand_grid.rs");
include!("color.rs");
include!("color_named.rs");
include!("primitives.rs");
