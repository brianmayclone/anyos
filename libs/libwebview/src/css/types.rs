#[derive(Clone)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub layer_order: Vec<String>,
    pub media_rules: Vec<MediaRule>,
    pub keyframes: Vec<KeyframeSet>,
    pub imports: Vec<String>,
    pub font_faces: Vec<FontFaceRule>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FontDisplay {
    Auto,
    Block,
    Swap,
    Fallback,
    Optional,
}

#[derive(Clone)]
pub struct FontFaceRule {
    pub family: String,
    pub src_url: String,
    pub weight: u32,
    pub italic: bool,
    pub display: FontDisplay,
}

#[derive(Clone)]
pub struct KeyframeSet {
    pub name: String,
    pub stops: Vec<KeyframeStop>,
}

#[derive(Clone)]
pub struct KeyframeStop {
    pub offset: i32,
    pub declarations: Vec<Declaration>,
}

#[derive(Clone)]
pub struct MediaRule {
    pub query: MediaQuery,
    pub rules: Vec<Rule>,
}

#[derive(Clone)]
pub struct ContainerQuery {
    pub name: Option<String>,
    pub conditions: Vec<ContainerCondition>,
}

#[derive(Clone)]
pub enum ContainerCondition {
    MinWidth(i32),
    MaxWidth(i32),
    MinHeight(i32),
    MaxHeight(i32),
    Width(i32),
    Height(i32),
    MinInlineSize(i32),
    MaxInlineSize(i32),
    MinBlockSize(i32),
    MaxBlockSize(i32),
    InlineSize(i32),
    BlockSize(i32),
}

#[derive(Clone)]
pub struct MediaQuery {
    pub conditions: Vec<MediaCondition>,
    pub media_type: MediaType,
}

#[derive(Clone, PartialEq)]
pub enum MediaType {
    All,
    Screen,
    Print,
    Not(Box<MediaType>),
}

#[derive(Clone)]
pub enum MediaCondition {
    MinWidth(i32),
    MaxWidth(i32),
    MinHeight(i32),
    MaxHeight(i32),
    PrefersColorScheme(String),
    Known(bool),
    Unsupported,
}

#[derive(Clone)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
    pub layer_name: Option<String>,
    pub layer_index: Option<usize>,
    pub container_query: Option<ContainerQuery>,
}

#[derive(Clone)]
pub enum Selector {
    Simple(SimpleSelector),
    Descendant(Box<Selector>, SimpleSelector),
    Child(Box<Selector>, SimpleSelector),
    AdjacentSibling(Box<Selector>, SimpleSelector),
    GeneralSibling(Box<Selector>, SimpleSelector),
    Universal,
}

#[derive(Clone)]
pub struct SimpleSelector {
    pub tag: Option<Tag>,
    pub custom_tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrSelector>,
    pub pseudo_classes: Vec<PseudoClass>,
    pub pseudo_element: Option<PseudoElement>,
}

#[derive(Clone)]
pub struct AttrSelector {
    pub name: String,
    pub op: AttrOp,
    pub value: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AttrOp {
    Exists,
    Exact,
    Contains,
    Prefix,
    Suffix,
    Substring,
    DashMatch,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PseudoElement {
    Before,
    After,
    Unknown,
}

#[derive(Clone)]
pub enum PseudoClass {
    Hover,
    Active,
    Focus,
    Visited,
    FirstChild,
    LastChild,
    NthChild(i32),
    NthLastChild(i32),
    FirstOfType,
    LastOfType,
    Not(Vec<SimpleSelector>),
    Is(Vec<SimpleSelector>),
    Where(Vec<SimpleSelector>),
    Has(Box<SimpleSelector>),
    Empty,
    Checked,
    Disabled,
    Enabled,
    Root,
    FocusVisible,
    FocusWithin,
    PlaceholderShown,
    Required,
    Optional,
    ReadOnly,
    ReadWrite,
    Valid,
    Invalid,
    InRange,
    OutOfRange,
    Default,
    Indeterminate,
}

#[derive(Clone)]
pub struct Declaration {
    pub property: Property,
    pub value: CssValue,
    pub important: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub enum Property {
    Display,
    Color,
    BackgroundColor,
    Background,
    FontSize,
    FontWeight,
    FontStyle,
    Direction,
    TextAlign,
    TextDecoration,
    TextIndent,
    LineHeight,
    VerticalAlign,
    Width,
    Height,
    MaxWidth,
    MinWidth,
    MaxHeight,
    MinHeight,
    Margin,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    Padding,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    Border,
    BorderTop,
    BorderRight,
    BorderBottom,
    BorderLeft,
    BorderColor,
    BorderWidth,
    BorderStyle,
    BorderRadius,
    ListStyleType,
    ListStylePosition,
    WhiteSpace,
    Overflow,
    OverflowX,
    OverflowY,
    Position,
    Top,
    Right,
    Bottom,
    Left,
    ZIndex,
    FlexDirection,
    FlexWrap,
    JustifyContent,
    AlignItems,
    AlignSelf,
    JustifySelf,
    PlaceItems,
    PlaceSelf,
    PlaceContent,
    AlignContent,
    FlexGrow,
    FlexShrink,
    FlexBasis,
    Flex,
    Gap,
    RowGap,
    ColumnGap,
    Order,
    BoxSizing,
    Float,
    Clear,
    Opacity,
    Visibility,
    TextTransform,
    Cursor,
    FontFamily,
    LetterSpacing,
    WordSpacing,
    WordBreak,
    OverflowWrap,
    TextOverflow,
    BorderTopWidth,
    BorderRightWidth,
    BorderBottomWidth,
    BorderLeftWidth,
    BorderTopColor,
    BorderRightColor,
    BorderBottomColor,
    BorderLeftColor,
    BorderTopStyle,
    BorderRightStyle,
    BorderBottomStyle,
    BorderLeftStyle,
    BorderTopLeftRadius,
    BorderTopRightRadius,
    BorderBottomRightRadius,
    BorderBottomLeftRadius,
    Outline,
    OutlineColor,
    OutlineStyle,
    OutlineWidth,
    OutlineOffset,
    BoxShadow,
    TextShadow,
    BackgroundImage,
    BackgroundPosition,
    BackgroundRepeat,
    BackgroundSize,
    Transform,
    TransformOrigin,
    Content,
    ObjectFit,
    Filter,
    AspectRatio,
    Inset,
    ClipPath,
    Clip,
    TextDecorationColor,
    TextDecorationStyle,
    TextDecorationThickness,
    TextUnderlineOffset,
    FontVariant,
    TabSize,
    CounterReset,
    CounterIncrement,
    BorderCollapse,
    BorderSpacing,
    TableLayout,
    Transition,
    TransitionProperty,
    TransitionDuration,
    TransitionTimingFunction,
    TransitionDelay,
    Animation,
    AnimationName,
    AnimationDuration,
    AnimationTimingFunction,
    AnimationDelay,
    AnimationIterationCount,
    AnimationDirection,
    AnimationFillMode,
    AnimationPlayState,
    GridTemplateColumns,
    GridTemplateRows,
    GridTemplateAreas,
    GridTemplate,
    GridAutoColumns,
    GridAutoRows,
    GridAutoFlow,
    JustifyItems,
    GridColumn,
    GridColumnStart,
    GridColumnEnd,
    GridRow,
    GridRowStart,
    GridRowEnd,
    GridArea,
    MaskImage,
    MaskPosition,
    MaskRepeat,
    MaskSize,
    MaskClip,
    MaskOrigin,
    PointerEvents,
    UserSelect,
    BackdropFilter,
    PaddingInline,
    PaddingBlock,
    MarginInline,
    MarginBlock,
    Appearance,
    AccentColor,
    BackgroundClip,
    ColorScheme,
    ContainerType,
    ContainerName,
    ScrollBehavior,
    Resize,
    ObjectPosition,
    CustomProperty(String),
}

#[derive(Clone, PartialEq)]
pub enum CssValue {
    Keyword(String),
    Color(u32),
    Length(i32, Unit),
    Percentage(i32),
    Number(i32),
    Auto,
    None,
    Inherit,
    Var(String, Option<Box<CssValue>>),
    Calc(i32, i32),
    CurrentColor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Px,
    Em,
    Rem,
    In,
    Cm,
    Mm,
    Pt,
    Pc,
    Q,
    Percent,
    Fr,
    Vw,
    Vh,
    Vmin,
    Vmax,
}

impl Selector {
    pub fn specificity(&self) -> (u32, u32, u32) {
        match self {
            Selector::Universal => (0, 0, 0),
            Selector::Simple(s) => s.specificity(),
            Selector::Descendant(ancestor, leaf)
            | Selector::Child(ancestor, leaf)
            | Selector::AdjacentSibling(ancestor, leaf)
            | Selector::GeneralSibling(ancestor, leaf) => {
                let (a1, b1, c1) = ancestor.specificity();
                let (a2, b2, c2) = leaf.specificity();
                (a1 + a2, b1 + b2, c1 + c2)
            }
        }
    }

    pub fn pseudo_element(&self) -> Option<PseudoElement> {
        match self {
            Selector::Simple(s) => s.pseudo_element,
            Selector::Descendant(_, leaf)
            | Selector::Child(_, leaf)
            | Selector::AdjacentSibling(_, leaf)
            | Selector::GeneralSibling(_, leaf) => leaf.pseudo_element,
            Selector::Universal => None,
        }
    }
}

impl SimpleSelector {
    fn specificity(&self) -> (u32, u32, u32) {
        let ids = if self.id.is_some() { 1 } else { 0 };
        let classes =
            self.classes.len() as u32 + self.attrs.len() as u32 + self.pseudo_classes.len() as u32;
        let tags = if self.tag.is_some() { 1 } else { 0 }
            + if self.pseudo_element.is_some() { 1 } else { 0 };
        (ids, classes, tags)
    }
}
