use alloc::string::String;
use alloc::vec::Vec;

use crate::dom::NodeId;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Display {
    Block,
    Inline,
    InlineBlock,
    ListItem,
    TableRow,
    TableCell,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    FlowRoot,
    Contents,
    None,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextDecorationStyle {
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FontVariantVal {
    Normal,
    SmallCaps,
}

#[derive(Clone, PartialEq)]
pub struct FilterVal {
    pub blur_px: i32,
    pub brightness: i32,
    pub contrast: i32,
    pub grayscale: i32,
    pub saturate: i32,
    pub sepia: i32,
    pub opacity: i32,
    pub hue_rotate: i32,
    pub invert: i32,
}

impl FilterVal {
    pub fn none() -> Self {
        FilterVal {
            blur_px: 0,
            brightness: 10000,
            contrast: 10000,
            grayscale: 0,
            saturate: 10000,
            sepia: 0,
            opacity: 10000,
            hue_rotate: 0,
            invert: 0,
        }
    }

    pub fn is_none(&self) -> bool {
        self.blur_px == 0
            && self.brightness == 10000
            && self.contrast == 10000
            && self.grayscale == 0
            && self.saturate == 10000
            && self.sepia == 0
            && self.opacity == 10000
            && self.hue_rotate == 0
            && self.invert == 0
    }
}

#[derive(Clone, PartialEq)]
pub enum ClipPathVal {
    None,
    Circle { radius: i32, cx: i32, cy: i32 },
    Inset {
        top: i32,
        right: i32,
        bottom: i32,
        left: i32,
        radius: i32,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TimingFunction {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    StepStart,
    StepEnd,
}

#[derive(Clone)]
pub struct TransitionDef {
    pub property: String,
    pub duration_ms: u32,
    pub timing: TimingFunction,
    pub delay_ms: u32,
}

#[derive(Clone)]
pub struct AnimationDef {
    pub name: String,
    pub duration_ms: u32,
    pub timing: TimingFunction,
    pub delay_ms: u32,
    pub iteration_count: u32,
    pub alternate: bool,
}

#[derive(Clone, PartialEq)]
pub enum GridTrackSize {
    Px(i32),
    Fr(i32),
    Percent(i32),
    Auto,
    MinContent,
    MaxContent,
    Minmax {
        min_px: i32,
        max_px: i32,
        max_is_fr: bool,
    },
    AutoFill { min_px: i32 },
    AutoFit { min_px: i32 },
    Subgrid,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GridLine {
    Auto,
    Index(i32),
    Span(i32),
    Named(String),
}

#[derive(Clone, PartialEq)]
pub struct GridArea {
    pub name: String,
    pub row_start: i32,
    pub col_start: i32,
    pub row_end: i32,
    pub col_end: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BoxSizing {
    ContentBox,
    BorderBox,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Hidden,
    Collapse,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ColorSchemeVal {
    Auto,
    Light,
    Dark,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BackgroundClipVal {
    BorderBox,
    PaddingBox,
    ContentBox,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScrollBehaviorVal {
    Auto,
    Smooth,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AppearanceVal {
    Auto,
    None,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ContainerTypeVal {
    Normal,
    InlineSize,
    Size,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FloatVal {
    None,
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClearVal {
    None,
    Left,
    Right,
    Both,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FlexWrap {
    Nowrap,
    Wrap,
    WrapReverse,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JustifyContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AlignItems {
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
    Baseline,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AlignContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    Stretch,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Ltr,
    Rtl,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WritingMode {
    HorizontalTb,
    VerticalLr,
    VerticalRl,
    SidewaysLr,
    SidewaysRl,
}

impl WritingMode {
    pub fn is_vertical(self) -> bool {
        matches!(
            self,
            WritingMode::VerticalLr
                | WritingMode::VerticalRl
                | WritingMode::SidewaysLr
                | WritingMode::SidewaysRl
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InlineAxisAlignment {
    Start,
    End,
    Left,
    Right,
    Center,
    Stretch,
    FirstBaseline,
    LastBaseline,
}

#[derive(Clone, Copy, Default)]
pub struct SelectorState {
    pub hovered_node: Option<NodeId>,
    pub active_node: Option<NodeId>,
    pub focused_node: Option<NodeId>,
    pub focus_visible_node: Option<NodeId>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverflowVal {
    Visible,
    Hidden,
    Scroll,
    Auto,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FontWeight {
    Normal,
    Bold,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FontStyleVal {
    Normal,
    Italic,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextAlignVal {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextDeco {
    None,
    Underline,
    LineThrough,
    Overline,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ListStyle {
    None,
    Disc,
    Circle,
    Square,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerLatin,
    UpperLatin,
    LowerRoman,
    UpperRoman,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ListStylePosition {
    Outside,
    Inside,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WhiteSpace {
    Normal,
    Pre,
    Nowrap,
    PreWrap,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BorderStyleVal {
    None,
    Solid,
    Dashed,
    Dotted,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
    Hidden,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WordBreak {
    Normal,
    BreakAll,
    KeepAll,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OverflowWrapVal {
    Normal,
    BreakWord,
    Anywhere,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextOverflowVal {
    Clip,
    Ellipsis,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ObjectFit {
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlign {
    Baseline,
    Top,
    Middle,
    Bottom,
    TextTop,
    TextBottom,
    Sub,
    Super,
    Length(i32),
}

#[derive(Clone, PartialEq)]
pub struct BoxShadowVal {
    pub offset_x: i32,
    pub offset_y: i32,
    pub blur: i32,
    pub spread: i32,
    pub color: u32,
    pub inset: bool,
}

#[derive(Clone, PartialEq)]
pub struct TextShadowVal {
    pub offset_x: i32,
    pub offset_y: i32,
    pub blur: i32,
    pub color: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BorderSide {
    pub width: i32,
    pub style: BorderStyleVal,
    pub color: u32,
}

impl BorderSide {
    pub fn none() -> Self {
        BorderSide {
            width: 0,
            style: BorderStyleVal::None,
            color: 0xFF000000,
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum BackgroundImageVal {
    None,
    Url(String),
    LinearGradient {
        angle_deg: i32,
        stops: Vec<GradientStop>,
    },
}

#[derive(Clone, PartialEq)]
pub struct GradientStop {
    pub color: u32,
    pub position: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BackgroundSizeVal {
    Auto,
    Cover,
    Contain,
    Explicit(i32, i32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BackgroundRepeatVal {
    Repeat,
    RepeatX,
    RepeatY,
    NoRepeat,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PointerEventsVal {
    Auto,
    None,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UserSelectVal {
    Auto,
    None,
    Text,
    All,
}

#[derive(Clone)]
pub struct ComputedStyle {
    pub display: Display,
    pub color: u32,
    pub background_color: u32,
    pub background_color_is_current: bool,
    pub accent_color: u32,
    pub font_size: i32,
    pub font_weight: FontWeight,
    pub font_style: FontStyleVal,
    pub direction: Direction,
    pub writing_mode: WritingMode,
    pub text_align: TextAlignVal,
    pub text_decoration: TextDeco,
    pub line_height: i32,
    pub margin_top: i32,
    pub margin_right: i32,
    pub margin_bottom: i32,
    pub margin_left: i32,
    pub margin_top_calc: Option<(i32, i32)>,
    pub margin_right_calc: Option<(i32, i32)>,
    pub margin_bottom_calc: Option<(i32, i32)>,
    pub margin_left_calc: Option<(i32, i32)>,
    pub margin_top_auto: bool,
    pub margin_left_auto: bool,
    pub margin_bottom_auto: bool,
    pub margin_right_auto: bool,
    pub padding_top: i32,
    pub padding_right: i32,
    pub padding_bottom: i32,
    pub padding_left: i32,
    pub padding_top_pct: Option<i32>,
    pub padding_right_pct: Option<i32>,
    pub padding_bottom_pct: Option<i32>,
    pub padding_left_pct: Option<i32>,
    pub border_width: i32,
    pub border_color: u32,
    pub border_radius: i32,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub max_width: Option<i32>,
    pub min_width: i32,
    pub max_height: Option<i32>,
    pub min_height: i32,
    pub max_width_calc: Option<(i32, i32)>,
    pub min_width_calc: Option<(i32, i32)>,
    pub max_height_calc: Option<(i32, i32)>,
    pub min_height_calc: Option<(i32, i32)>,
    pub list_style: ListStyle,
    pub list_style_position: ListStylePosition,
    pub white_space: WhiteSpace,
    pub position: Position,
    pub top: Option<i32>,
    pub top_calc: Option<(i32, i32)>,
    pub right_offset: Option<i32>,
    pub right_calc: Option<(i32, i32)>,
    pub bottom_offset: Option<i32>,
    pub bottom_calc: Option<(i32, i32)>,
    pub left_offset: Option<i32>,
    pub left_calc: Option<(i32, i32)>,
    pub z_index: i32,
    pub z_index_auto: bool,
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub align_self: Option<AlignItems>,
    pub align_self_is_normal: bool,
    pub justify_self: Option<AlignItems>,
    pub justify_self_is_normal: bool,
    pub justify_self_inline: Option<InlineAxisAlignment>,
    pub flex_grow: i32,
    pub flex_shrink: i32,
    pub flex_basis: Option<i32>,
    pub flex_basis_pct: Option<i32>,
    pub row_gap: i32,
    pub column_gap: i32,
    pub align_content: AlignContent,
    pub align_content_is_normal: bool,
    pub order: i32,
    pub grid_template_columns: Vec<GridTrackSize>,
    pub grid_template_rows: Vec<GridTrackSize>,
    pub grid_template_areas: Vec<GridArea>,
    pub grid_auto_columns: GridTrackSize,
    pub grid_auto_rows: GridTrackSize,
    pub grid_auto_flow_column: bool,
    pub justify_items: AlignItems,
    pub justify_items_specified: bool,
    pub justify_items_inline: Option<InlineAxisAlignment>,
    pub grid_column_start: GridLine,
    pub grid_column_end: GridLine,
    pub grid_row_start: GridLine,
    pub grid_row_end: GridLine,
    pub box_sizing: BoxSizing,
    pub border_collapse: bool,
    pub border_spacing_x: i32,
    pub border_spacing_y: i32,
    pub table_layout_fixed: bool,
    pub float: FloatVal,
    pub clear: ClearVal,
    pub opacity: i32,
    pub visibility: Visibility,
    pub text_transform: TextTransform,
    pub color_scheme: ColorSchemeVal,
    pub appearance: AppearanceVal,
    pub container_type: ContainerTypeVal,
    pub container_names: Vec<String>,
    pub overflow_x: OverflowVal,
    pub overflow_y: OverflowVal,
    pub scroll_behavior: ScrollBehaviorVal,
    pub width_pct: Option<i32>,
    pub height_pct: Option<i32>,
    pub width_calc: Option<(i32, i32)>,
    pub height_calc: Option<(i32, i32)>,
    pub width_max_content: bool,
    pub width_min_content: bool,
    pub width_fit_content: bool,
    pub font_family: Option<String>,
    pub letter_spacing: i32,
    pub word_spacing: i32,
    pub text_indent: i32,
    pub vertical_align: VerticalAlign,
    pub word_break: WordBreak,
    pub overflow_wrap: OverflowWrapVal,
    pub text_overflow: TextOverflowVal,
    pub border_top: BorderSide,
    pub border_right: BorderSide,
    pub border_bottom: BorderSide,
    pub border_left: BorderSide,
    pub border_top_left_radius: i32,
    pub border_top_right_radius: i32,
    pub border_bottom_right_radius: i32,
    pub border_bottom_left_radius: i32,
    pub outline_width: i32,
    pub outline_style: BorderStyleVal,
    pub outline_color: u32,
    pub outline_offset: i32,
    pub box_shadows: Vec<BoxShadowVal>,
    pub text_shadows: Vec<TextShadowVal>,
    pub background_image: BackgroundImageVal,
    pub mask_image: BackgroundImageVal,
    pub background_size: BackgroundSizeVal,
    pub background_repeat: BackgroundRepeatVal,
    pub background_clip: BackgroundClipVal,
    pub background_position_x: i32,
    pub background_position_y: i32,
    pub mask_size: BackgroundSizeVal,
    pub mask_repeat: BackgroundRepeatVal,
    pub mask_clip: BackgroundClipVal,
    pub mask_origin: BackgroundClipVal,
    pub mask_position_x: i32,
    pub mask_position_x_is_percent: bool,
    pub mask_position_y: i32,
    pub mask_position_y_is_percent: bool,
    pub content: Option<String>,
    pub content_url: Option<String>,
    pub object_fit: ObjectFit,
    pub object_position_x: i32,
    pub object_position_x_is_percent: bool,
    pub object_position_y: i32,
    pub object_position_y_is_percent: bool,
    pub transform_tx: i32,
    pub transform_ty: i32,
    pub transform_origin_x: i32,
    pub transform_origin_x_is_percent: bool,
    pub transform_origin_y: i32,
    pub transform_origin_y_is_percent: bool,
    pub filter: FilterVal,
    pub aspect_ratio: i32,
    pub text_decoration_color: u32,
    pub text_decoration_style: TextDecorationStyle,
    pub text_decoration_thickness: i32,
    pub text_underline_offset: i32,
    pub font_variant: FontVariantVal,
    pub tab_size: i32,
    pub clip_path: ClipPathVal,
    pub clip_rect: Option<[i32; 4]>,
    pub counter_reset: Option<String>,
    pub counter_increment: Option<String>,
    pub transitions: Vec<TransitionDef>,
    pub animations: Vec<AnimationDef>,
    pub pointer_events: PointerEventsVal,
    pub user_select: UserSelectVal,
    pub backdrop_filter: FilterVal,
    pub transform_sx: i32,
    pub transform_sy: i32,
    pub transform_rotate: i32,
}

#[derive(Clone)]
pub struct PseudoStyles {
    pub before: Vec<Option<ComputedStyle>>,
    pub after: Vec<Option<ComputedStyle>>,
}

impl PseudoStyles {
    pub fn empty(count: usize) -> Self {
        let mut before = Vec::with_capacity(count);
        let mut after = Vec::with_capacity(count);
        for _ in 0..count {
            before.push(None);
            after.push(None);
        }
        PseudoStyles { before, after }
    }
}
