// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use logos::Logos;

/// Token and node kinds owned by the nxc frontend, independent of rnix.
#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum SyntaxKind {
    #[regex(r"[ \t\r\n\x0C]+")]
    Whitespace,
    #[regex(r"//[^\r\n]*", allow_greedy = true)]
    #[regex(r"#[^\r\n]*", allow_greedy = true)]
    LineComment,
    #[token("/*", block_comment)]
    BlockComment,
    #[regex(r"[A-Za-z_][A-Za-z0-9_'\-]*")]
    Ident,
    #[regex(r"[0-9]+")]
    Integer,
    // A slash within path-shaped text takes precedence over division.
    #[regex(r"[A-Za-z0-9._+\-]+/[A-Za-z0-9._+\-][A-Za-z0-9._+\-/]*")]
    RelativePath,
    #[regex(r"/[A-Za-z0-9._+\-][A-Za-z0-9._+\-/]*")]
    #[regex(r"~/[A-Za-z0-9._+\-/]*")]
    UnsupportedPath,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token(",")]
    Comma,
    #[token("+")]
    Plus,
    #[token("++")]
    PlusPlus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("!")]
    Bang,
    #[token("==")]
    EqualEqual,
    #[token("!=")]
    NotEqual,
    #[token("<")]
    Less,
    #[token("<=")]
    LessEqual,
    #[token(">")]
    Greater,
    #[token(">=")]
    GreaterEqual,
    #[token("&&")]
    AndAnd,
    #[token("||")]
    OrOr,
    #[token("fn")]
    Fn,
    #[token("=>")]
    Arrow,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("?")]
    Question,
    #[token("...")]
    Ellipsis,
    #[token("@")]
    At,
    #[token(".")]
    Dot,
    #[token("=")]
    Assign,
    #[token(";")]
    Semicolon,
    #[token("rec")]
    Rec,
    #[token("inherit")]
    Inherit,
    #[token("or")]
    Or,
    #[token("let")]
    Let,
    #[token("yield")]
    Yield,
    #[token("with")]
    With,
    #[token("assert")]
    Assert,
    #[token("__nxc_update")]
    UpdateIntrinsic,
    #[token("if")]
    If,
    #[token("then")]
    Then,
    #[token("else")]
    Else,
    #[token("\"")]
    #[token("''")]
    StringStart,
    StringContent,
    StringEnd,
    #[token("${")]
    InterpolationStart,
    InterpolationEnd,
    ErrorToken,
    Root,
    IntegerExpr,
    VariableExpr,
    RelativePathExpr,
    ParenExpr,
    CallExpr,
    NegateExpr,
    NotExpr,
    BinaryExpr,
    LambdaExpr,
    AttrSetExpr,
    LetExpr,
    WithExpr,
    AssertExpr,
    UpdateExpr,
    IfExpr,
    SelectExpr,
    StringExpr,
    ListExpr,
    StringText,
    StringInterpolation,
    AttrPath,
    AttrName,
    AssignBinding,
    InheritBinding,
    InheritSource,
    ErrorBinding,
    IdentPattern,
    AttrPattern,
    Formal,
    PatternBind,
    PatternEllipsis,
    ErrorExpr,
}

fn block_comment(lex: &mut logos::Lexer<'_, SyntaxKind>) -> Result<(), ()> {
    if let Some(end) = lex.remainder().find("*/") {
        lex.bump(end + 2);
        Ok(())
    } else {
        lex.bump(lex.remainder().len());
        Err(())
    }
}

impl SyntaxKind {
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::Whitespace | Self::LineComment | Self::BlockComment
        )
    }

    pub(crate) fn is_expr(self) -> bool {
        matches!(
            self,
            Self::IntegerExpr
                | Self::VariableExpr
                | Self::RelativePathExpr
                | Self::ParenExpr
                | Self::CallExpr
                | Self::NegateExpr
                | Self::NotExpr
                | Self::BinaryExpr
                | Self::LambdaExpr
                | Self::AttrSetExpr
                | Self::LetExpr
                | Self::WithExpr
                | Self::AssertExpr
                | Self::UpdateExpr
                | Self::IfExpr
                | Self::SelectExpr
                | Self::StringExpr
                | Self::ListExpr
                | Self::ErrorExpr
        )
    }
}

impl std::fmt::Display for SyntaxKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Ident => "identifier",
            Self::Integer => "integer",
            Self::RelativePath => "relative path",
            Self::LParen => "'('",
            Self::RParen => "')'",
            Self::LBracket => "'['",
            Self::RBracket => "']'",
            Self::Comma => "','",
            Self::Plus => "'+'",
            Self::PlusPlus => "'++'",
            Self::Minus => "'-'",
            Self::Star => "'*'",
            Self::Slash => "'/'",
            Self::Bang => "'!'",
            Self::EqualEqual => "'=='",
            Self::NotEqual => "'!='",
            Self::Less => "'<'",
            Self::LessEqual => "'<='",
            Self::Greater => "'>'",
            Self::GreaterEqual => "'>='",
            Self::AndAnd => "'&&'",
            Self::OrOr => "'||'",
            Self::Fn => "'fn'",
            Self::Arrow => "'=>'",
            Self::LBrace => "'{'",
            Self::RBrace => "'}'",
            Self::Question => "'?'",
            Self::Ellipsis => "'...'",
            Self::At => "'@'",
            Self::Dot => "'.'",
            Self::Assign => "'='",
            Self::Semicolon => "';'",
            Self::Rec => "'rec'",
            Self::Inherit => "'inherit'",
            Self::Or => "'or'",
            Self::Let => "'let'",
            Self::Yield => "'yield'",
            Self::With => "'with'",
            Self::Assert => "'assert'",
            Self::UpdateIntrinsic => "'__nxc_update'",
            Self::If => "'if'",
            Self::Then => "'then'",
            Self::Else => "'else'",
            Self::StringStart => "opening quote",
            Self::StringContent => "string text",
            Self::StringEnd => "closing quote",
            Self::InterpolationStart => "'${'",
            Self::InterpolationEnd => "interpolation end",
            _ => "unsupported token",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NxcLanguage {}

impl rowan::Language for NxcLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        use SyntaxKind::*;
        const KINDS: &[SyntaxKind] = &[
            Whitespace,
            LineComment,
            BlockComment,
            Ident,
            Integer,
            RelativePath,
            UnsupportedPath,
            LParen,
            RParen,
            LBracket,
            RBracket,
            Comma,
            Plus,
            PlusPlus,
            Minus,
            Star,
            Slash,
            Bang,
            EqualEqual,
            NotEqual,
            Less,
            LessEqual,
            Greater,
            GreaterEqual,
            AndAnd,
            OrOr,
            Fn,
            Arrow,
            LBrace,
            RBrace,
            Question,
            Ellipsis,
            At,
            Dot,
            Assign,
            Semicolon,
            Rec,
            Inherit,
            Or,
            Let,
            Yield,
            With,
            Assert,
            UpdateIntrinsic,
            If,
            Then,
            Else,
            StringStart,
            StringContent,
            StringEnd,
            InterpolationStart,
            InterpolationEnd,
            ErrorToken,
            Root,
            IntegerExpr,
            VariableExpr,
            RelativePathExpr,
            ParenExpr,
            CallExpr,
            NegateExpr,
            NotExpr,
            BinaryExpr,
            LambdaExpr,
            AttrSetExpr,
            LetExpr,
            WithExpr,
            AssertExpr,
            UpdateExpr,
            IfExpr,
            SelectExpr,
            StringExpr,
            ListExpr,
            StringText,
            StringInterpolation,
            AttrPath,
            AttrName,
            AssignBinding,
            InheritBinding,
            InheritSource,
            ErrorBinding,
            IdentPattern,
            AttrPattern,
            Formal,
            PatternBind,
            PatternEllipsis,
            ErrorExpr,
        ];
        KINDS.get(usize::from(raw.0)).copied().unwrap_or(ErrorToken)
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind as u16)
    }
}
