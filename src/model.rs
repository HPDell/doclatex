//! Document model: the intermediate representation between the TeX parser
//! and the OOXML writer.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontFamily {
    Rm,
    Sf,
    Tt,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontSize {
    Tiny,
    Scriptsize,
    Footnotesize,
    Small,
    Normalsize,
    Large,
    LargeX,
    LargeXX,
    Huge,
    HugeX,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Number,
    Page,
    Name,
}

#[derive(Clone, Debug)]
pub enum Inline {
    Text(String),
    Space,
    Nbsp,
    LineBreak,
    SoftHyphen,
    Bold(Vec<Inline>),
    Italic(Vec<Inline>),
    BoldItalic(Vec<Inline>),
    Underline(Vec<Inline>),
    Strike(Vec<Inline>),
    SmallCaps(Vec<Inline>),
    Slanted(Vec<Inline>),
    Superscript(Vec<Inline>),
    Subscript(Vec<Inline>),
    Family(FontFamily, Vec<Inline>),
    FontSize(FontSize, Vec<Inline>),
    Color {
        color: String,
        content: Vec<Inline>,
    },
    Highlight {
        color: String,
        content: Vec<Inline>,
    },
    Math {
        omml: String,
        display: bool,
    },
    /// Cross-reference: text is the cached field result, anchor the bookmark.
    Ref {
        key: String,
        text: String,
        anchor: Option<String>,
        kind: RefKind,
        counter: String,
    },
    /// Citation rendered as text (may contain multiple keys).
    Cite {
        text: String,
        links: Vec<(String, String)>,
    },
    Link {
        url: String,
        content: Vec<Inline>,
        external: bool,
    },
    Image(Image),
    Footnote(Box<Block>),
    Code(String),
    Group(Vec<Inline>),
    /// A Word comment? reserved.
    Meta(String),
}

#[derive(Clone, Debug)]
pub struct Image {
    pub path: String,
    pub data: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
    pub width_pt: Option<f64>,
    pub height_pt: Option<f64>,
    pub alt: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Bullet,
    Number,
}

#[derive(Clone, Debug)]
pub struct ListItem {
    pub label: Option<Vec<Inline>>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug)]
pub struct Caption {
    pub number: Option<String>,
    pub content: Vec<Inline>,
    pub label: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TableCell {
    pub blocks: Vec<Block>,
    pub align: Align,
    pub colspan: usize,
    pub rowspan: usize,
    pub header: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RuleKind {
    None,
    Thin,
    Thick,
}

#[derive(Clone, Debug)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
    pub rule_top: RuleKind,
    pub rule_bottom: RuleKind,
}

#[derive(Clone, Debug)]
pub struct Table {
    pub rows: Vec<TableRow>,
    pub aligns: Vec<Align>,
    pub width_pt: Option<f64>,
    pub borders: bool,
    pub caption: Option<Caption>,
}

#[derive(Clone, Debug)]
pub struct BibItem {
    pub key: String,
    pub label: String,
    pub content: Vec<Inline>,
}

#[derive(Clone, Debug)]
pub struct GlossItem {
    pub abbr: String,
    pub long: String,
    pub desc: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Block {
    Heading {
        level: u8,
        number: Option<String>,
        title: Vec<Inline>,
        anchor: Option<String>,
        kind: HeadingKind,
    },
    Para {
        inlines: Vec<Inline>,
        align: Option<Align>,
        indent: bool,
    },
    Environment {
        name: String,
        content: Vec<Block>,
    },
    Caption {
        kind: String,
        inlines: Vec<Inline>,
    },
    List {
        kind: ListKind,
        items: Vec<ListItem>,
    },
    DefinitionList(Vec<(Vec<Inline>, Vec<Block>)>),
    AlignBlock(Align, Vec<Block>),
    Verbatim {
        lines: Vec<String>,
        style: String,
    },
    DisplayMath {
        omml: String,
        number: Option<String>,
        tag: Option<String>,
        anchor: Option<String>,
    },
    TableBlock(Table),
    Figure {
        content: Vec<Block>,
        caption: Option<Caption>,
        anchor: Option<String>,
        kind: String, // "figure" | "table" | other float
    },
    BibSection {
        title: String,
        items: Vec<BibItem>,
    },
    GlossSection {
        title: String,
        entries: Vec<GlossItem>,
    },
    TitlePage {
        title: Vec<Inline>,
        author: Vec<Inline>,
        date: Vec<Inline>,
        thanks: Vec<Vec<Inline>>,
    },
    Toc,
    PageBreak,
    HRule,
    /// theorem-like environment
    Theorem {
        name: String,
        number: Option<String>,
        content: Vec<Block>,
        anchor: Option<String>,
    },
    /// A generic \vspace skip, rendered as an empty small paragraph
    Spacer(f64),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HeadingKind {
    Part,
    Chapter,
    Section,
    Other,
}

#[derive(Clone, Debug)]
pub struct Document {
    pub blocks: Vec<Block>,
    pub footnotes: Vec<Block>,
    /// label key, number, title, counter
    pub labels: Vec<(String, String, Option<String>, String)>,
    pub warnings: Vec<String>,
}

impl Document {
    pub fn new() -> Document {
        Document {
            blocks: vec![],
            footnotes: vec![],
            labels: vec![],
            warnings: vec![],
        }
    }
}

/// Plain-text extraction of inlines (for aux/titles).
pub fn inlines_to_text(inls: &[Inline]) -> String {
    let mut s = String::new();
    for i in inls {
        match i {
            Inline::Text(t) => s.push_str(t),
            Inline::Space => s.push(' '),
            Inline::Nbsp => s.push(' '),
            Inline::LineBreak => s.push(' '),
            Inline::Bold(v)
            | Inline::Italic(v)
            | Inline::BoldItalic(v)
            | Inline::Underline(v)
            | Inline::Strike(v)
            | Inline::SmallCaps(v)
            | Inline::Slanted(v)
            | Inline::Superscript(v)
            | Inline::Subscript(v)
            | Inline::Group(v)
            | Inline::Family(_, v)
            | Inline::FontSize(_, v) => s.push_str(&inlines_to_text(v)),
            Inline::Color { content, .. } | Inline::Highlight { content, .. } => {
                s.push_str(&inlines_to_text(content))
            }
            Inline::Math { omml, .. } => {
                // best-effort: strip XML tags
                let mut in_tag = false;
                for c in omml.chars() {
                    if c == '<' {
                        in_tag = true;
                    } else if c == '>' {
                        in_tag = false;
                    } else if !in_tag {
                        s.push(c);
                    }
                }
            }
            Inline::Ref { text, .. } => s.push_str(text),
            Inline::Cite { text, .. } => s.push_str(text),
            Inline::Link { content, .. } => s.push_str(&inlines_to_text(content)),
            Inline::Image(img) => s.push_str(&img.alt),
            Inline::Footnote(_) => {}
            Inline::Code(c) => s.push_str(c),
            Inline::SoftHyphen => {}
            Inline::Meta(_) => {}
        }
    }
    s
}
