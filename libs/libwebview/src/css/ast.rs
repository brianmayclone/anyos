#[derive(Clone, Debug)]
enum CssSyntaxNode {
    AtRule(CssAtRuleNode),
    QualifiedRule(CssQualifiedRuleNode),
}

#[derive(Clone, Debug)]
struct CssStylesheetAst {
    items: Vec<CssSyntaxNode>,
}

#[derive(Clone, Debug)]
struct CssAtRuleNode {
    name: String,
    prelude: String,
    block: Option<CssBlockNode>,
}

#[derive(Clone, Debug)]
struct CssQualifiedRuleNode {
    prelude: String,
    block: CssBlockNode,
}

#[derive(Clone, Debug)]
struct CssBlockNode {
    source: String,
    items: Vec<CssSyntaxNode>,
}

#[derive(Clone, Debug)]
struct CssDeclarationAst {
    name: String,
    value: CssValueAst,
    important: bool,
}

#[derive(Clone, Debug)]
struct CssValueAst {
    raw: String,
    components: Vec<CssValueComponentAst>,
}

#[derive(Clone, Debug)]
enum CssValueComponentAst {
    Ident(String),
    Number(String),
    Dimension(String),
    String(String),
    Hash(String),
    Delim(char),
    Comma,
    Slash,
    Function {
        name: String,
        args: Vec<CssValueAst>,
    },
}

#[derive(Clone, Copy, Debug)]
enum CssAttrOpAst {
    Exists,
    Exact,
    Contains,
    Prefix,
    Suffix,
    Substring,
    DashMatch,
}

#[derive(Clone, Debug)]
struct CssAttrSelectorAst {
    name: String,
    op: CssAttrOpAst,
    value: Option<String>,
}

#[derive(Clone, Copy, Debug)]
enum CssPseudoElementAst {
    Before,
    After,
    Unknown,
}

#[derive(Clone, Debug)]
enum CssPseudoClassAst {
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
    Not(Vec<CssSimpleSelectorAst>),
    Is(Vec<CssSimpleSelectorAst>),
    Where(Vec<CssSimpleSelectorAst>),
    Has(Box<CssSimpleSelectorAst>),
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
    Unsupported,
}

#[derive(Clone, Debug)]
struct CssSimpleSelectorAst {
    explicit_universal: bool,
    tag_name: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<CssAttrSelectorAst>,
    pseudo_classes: Vec<CssPseudoClassAst>,
    pseudo_element: Option<CssPseudoElementAst>,
}

#[derive(Clone, Debug)]
enum CssCombinatorAst {
    Descendant,
    Child,
    AdjacentSibling,
    GeneralSibling,
}

#[derive(Clone, Debug)]
struct CssSelectorAst {
    first: CssSimpleSelectorAst,
    rest: Vec<(CssCombinatorAst, CssSimpleSelectorAst)>,
}
