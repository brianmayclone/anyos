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
    value: String,
    important: bool,
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
    first: String,
    rest: Vec<(CssCombinatorAst, String)>,
}
