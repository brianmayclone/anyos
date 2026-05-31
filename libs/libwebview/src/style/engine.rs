//! Style resolution: takes a DOM tree + CSS stylesheets and computes
//! the final `ComputedStyle` for every node.
//!
//! Cascade order: initial values -> UA defaults -> author rules (by
//! specificity) -> inline styles.  Inheritable properties that are not
//! explicitly set by any declaration are inherited from the parent node.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use super::initial::default_style;
use super::lengths::{parse_transform_translate_component, resolve_length, set_viewport_size};
use super::types::*;
use crate::css::{
    AttrOp, ContainerCondition, ContainerQuery, CssValue, Declaration, Property, PseudoClass,
    PseudoElement, Rule, Selector, SimpleSelector, Stylesheet, Unit,
};
use crate::dom::{Dom, NodeId, NodeType, Tag};

// Bitflags for tracking which inheritable properties were explicitly set.
const SET_COLOR: u32 = 1 << 0;
const SET_FONT_SIZE: u32 = 1 << 1;
const SET_FONT_WEIGHT: u32 = 1 << 2;
const SET_FONT_STYLE: u32 = 1 << 3;
const SET_FONT_FAMILY: u32 = 1 << 4;
const SET_DIRECTION: u32 = 1 << 5;
const SET_TEXT_ALIGN: u32 = 1 << 6;
const SET_LINE_HEIGHT: u32 = 1 << 7;
const SET_WHITE_SPACE: u32 = 1 << 8;
const SET_LIST_STYLE: u32 = 1 << 9;
const SET_TEXT_DECO: u32 = 1 << 10;
const SET_VISIBILITY: u32 = 1 << 11;
const SET_TEXT_TRANSFORM: u32 = 1 << 12;
const SET_LETTER_SPACING: u32 = 1 << 13;
const SET_WORD_SPACING: u32 = 1 << 14;
const SET_WORD_BREAK: u32 = 1 << 15;
const SET_OVERFLOW_WRAP: u32 = 1 << 16;
const SET_LIST_STYLE_POS: u32 = 1 << 17;
const SET_ACCENT_COLOR: u32 = 1 << 18;
const SET_COLOR_SCHEME: u32 = 1 << 19;
const SET_WRITING_MODE: u32 = 1 << 20;
const SET_POINTER_EVENTS: u32 = 1 << 21;

include!("engine/ua.rs");
include!("engine/prepared.rs");
include!("engine/selectors.rs");
include!("engine/resolve.rs");
include!("engine/inheritance.rs");
include!("engine/declarations.rs");
include!("engine/grid_values.rs");
include!("engine/animation_values.rs");
include!("engine/borders_shadows.rs");
include!("engine/backgrounds.rs");
include!("engine/filters.rs");
include!("engine/clip_paths.rs");
include!("engine/grid_areas.rs");
