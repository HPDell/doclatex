//! The parser: converts the expanded token stream into the document model.
//! Handles document structure (sections, lists, floats, tables), inline
//! formatting, cross-references, citations, acronyms and bibliography hooks.

use crate::aux::AuxData;
use crate::bib::{self, BibState};
use crate::engine::*;
use crate::math;
use crate::model::*;
use crate::token::*;

#[derive(Clone, Default)]
struct Style {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    smallcaps: bool,
    slanted: bool,
    mono: bool,
    sans: bool,
    size: Option<FontSize>,
    color: Option<String>,
}

impl Style {
    fn is_plain(&self) -> bool {
        !self.bold
            && !self.italic
            && !self.underline
            && !self.strike
            && !self.smallcaps
            && !self.slanted
            && !self.mono
            && !self.sans
            && self.size.is_none()
            && self.color.is_none()
    }
}

pub struct Parser<'a> {
    st: &'a mut State,
    cur: Cursor,
    aux: &'a AuxData,
    bib: &'a mut BibState,
    pub labels: Vec<(String, String, Option<String>, String)>, // key, number, title, counter
    blocks: Vec<Block>,
    footnotes: Vec<Block>,
    list_depth: usize,
    pending_align: Option<Align>,
    pending_indent: bool,
    title: Option<Vec<Token>>,
    author: Option<Vec<Token>>,
    date: Option<Vec<Token>>,
    thanks: Vec<Vec<Token>>,
    in_preamble: bool,
    stop_document: bool,
    /// paragraph collector state
    para: Vec<Inline>,
    para_align: Option<Align>,
    para_indent: bool,
}

pub struct ParseResult {
    pub doc: Document,
    pub title: Option<Vec<Inline>>,
}

pub fn parse_document(
    st: &mut State,
    toks: Vec<Token>,
    aux: &AuxData,
    bib: &mut BibState,
) -> ParseResult {
    let mut p = Parser {
        st,
        cur: Cursor::new(toks),
        aux,
        bib,
        labels: vec![],
        blocks: vec![],
        footnotes: vec![],
        list_depth: 0,
        pending_align: None,
        pending_indent: true,
        title: None,
        author: None,
        date: None,
        thanks: vec![],
        in_preamble: true,
        stop_document: false,
        para: vec![],
        para_align: None,
        para_indent: true,
    };
    p.run();
    let mut doc = Document::new();
    doc.blocks = std::mem::take(&mut p.blocks);
    doc.footnotes = std::mem::take(&mut p.footnotes);
    doc.labels = std::mem::take(&mut p.labels);
    doc.warnings = p.st.warnings.clone();
    let title = p
        .title
        .take()
        .map(|t| p.parse_inlines_toks(t, &mut Style::default()));
    ParseResult { doc, title }
}

/// Tokens that terminate an inline run (block-level starters).
fn has_visible_content(inlines: &[Inline]) -> bool {
    inlines.iter().any(|inline| match inline {
        Inline::Text(text) => !text.trim().is_empty(),
        Inline::Space | Inline::LineBreak => false,
        Inline::Bold(content)
        | Inline::Italic(content)
        | Inline::BoldItalic(content)
        | Inline::Underline(content)
        | Inline::Strike(content)
        | Inline::SmallCaps(content)
        | Inline::Slanted(content)
        | Inline::Superscript(content)
        | Inline::Subscript(content)
        | Inline::Family(_, content)
        | Inline::FontSize(_, content)
        | Inline::Group(content) => has_visible_content(content),
        Inline::Color { content, .. } | Inline::Highlight { content, .. } => {
            has_visible_content(content)
        }
        _ => true,
    })
}

fn is_block_start(t: &Token) -> bool {
    match t {
        Token::CtrlSeq(n) => matches!(
            n.as_str(),
            "par"
                | "section"
                | "subsection"
                | "subsubsection"
                | "paragraph"
                | "subparagraph"
                | "chapter"
                | "part"
                | "begin"
                | "end"
                | "item"
                | "maketitle"
                | "tableofcontents"
                | "listoffigures"
                | "listoftables"
                | "bibliography"
                | "printglossaries"
                | "printglossary"
                | "printacronyms"
                | "newpage"
                | "clearpage"
                | "cleardoublepage"
                | "pagebreak"
                | "vspace"
                | "smallskip"
                | "medskip"
                | "bigskip"
                | "appendix"
                | "bibliographystyle"
                | "caption"
                | "captionof"
                | "["
        ),
        _ => false,
    }
}

impl<'a> Parser<'a> {
    fn run(&mut self) {
        loop {
            self.skip_junk();
            if self.cur.at_end() || self.stop_document {
                break;
            }
            self.parse_block();
        }
        self.flush_para();
    }

    fn parse_blocks_inner(&mut self, stop: &dyn Fn(&Token) -> bool) {
        loop {
            self.skip_junk();
            if self.cur.at_end() || self.stop_document {
                break;
            }
            if self.cur.peek().is_some_and(|t| stop(t)) {
                break;
            }
            self.parse_block();
            if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "par") {
                self.flush_para();
            }
        }
    }

    fn skip_junk(&mut self) {
        loop {
            match self.cur.peek() {
                Some(Token::CtrlSeq(n)) if n == "par" => {
                    self.cur.pos += 1;
                    self.flush_para();
                }
                Some(Token::Space) => {
                    self.cur.pos += 1;
                }
                _ => break,
            }
        }
    }

    fn peek_is_end(&self, env: &str) -> bool {
        // lookahead: \end { env }
        let mut k = self.cur.pos;
        match self.cur.toks.get(k) {
            Some(Token::CtrlSeq(n)) if n == "end" => {}
            _ => return false,
        }
        k += 1;
        while matches!(self.cur.toks.get(k), Some(Token::Space)) {
            k += 1;
        }
        if matches!(self.cur.toks.get(k), Some(t) if t.is_bgroup()) {
            k += 1;
            let mut name = String::new();
            while let Some(t) = self.cur.toks.get(k) {
                if t.is_egroup() {
                    break;
                }
                name.push_str(&t.to_text());
                k += 1;
            }
            name.trim() == env
        } else {
            false
        }
    }

    // -----------------------------------------------------------------------
    // Blocks
    // -----------------------------------------------------------------------

    fn parse_block(&mut self) {
        let t = match self.cur.peek().cloned() {
            Some(t) => t,
            None => return,
        };
        match &t {
            Token::CtrlSeq(n) => {
                let name = n.clone();
                match name.as_str() {
                    "par" => {
                        self.cur.pos += 1;
                    }
                    "part" | "chapter" | "section" | "subsection" | "subsubsection"
                    | "paragraph" | "subparagraph" => {
                        self.heading(&name);
                    }
                    "appendix" => {
                        self.cur.pos += 1;
                        self.st.appendix = true;
                        if let Some(c) = self.st.counters.get_mut("section") {
                            c.value = 0;
                        }
                    }
                    "maketitle" => {
                        self.cur.pos += 1;
                        self.make_title();
                    }
                    "tableofcontents" => {
                        self.cur.pos += 1;
                        self.blocks.push(Block::Toc);
                    }
                    "listoffigures" | "listoftables" => {
                        self.cur.pos += 1;
                        self.st
                            .warn("\\listoffigures/\\listoftables not supported; skipped");
                    }
                    "bibliography" => {
                        self.cur.pos += 1;
                        let g = self.cur.read_group();
                        let files = detokenize(&g);
                        if !files.is_empty() {
                            self.bib.files = files
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                        }
                        self.flush_para();
                        self.render_bibliography();
                    }
                    "bibliographystyle" => {
                        self.cur.pos += 1;
                        let g = self.cur.read_group();
                        let sname = detokenize(&g);
                        self.bib.style_name = sname.clone();
                        self.bib.style = crate::bib::BibStyle::from_name(&sname);
                    }
                    "printglossaries" | "printglossary" | "printacronyms" => {
                        self.cur.pos += 1;
                        if name == "printglossary" {
                            if let Some(_opts) = self.cur.read_optional() {}
                        }
                        self.flush_para();
                        self.render_glossary(&name);
                    }
                    "newpage" | "clearpage" | "cleardoublepage" | "pagebreak" => {
                        self.cur.pos += 1;
                        self.flush_para();
                        self.blocks.push(Block::PageBreak);
                    }
                    "vspace" | "vspace*" => {
                        self.cur.pos += 1;
                        let mut c = Cursor::new(self.cur.read_group());
                        let pt = scan_dimen(self.st, &mut c);
                        self.flush_para();
                        self.blocks.push(Block::Spacer(pt));
                    }
                    "smallskip" => {
                        self.cur.pos += 1;
                        self.flush_para();
                        self.blocks.push(Block::Spacer(6.0));
                    }
                    "medskip" => {
                        self.cur.pos += 1;
                        self.flush_para();
                        self.blocks.push(Block::Spacer(12.0));
                    }
                    "bigskip" => {
                        self.cur.pos += 1;
                        self.flush_para();
                        self.blocks.push(Block::Spacer(24.0));
                    }
                    "noindent" => {
                        self.cur.pos += 1;
                        self.para_indent = false;
                    }
                    "indent" => {
                        self.cur.pos += 1;
                        self.para_indent = true;
                    }
                    "centering" | "raggedright" | "raggedleft" => {
                        self.cur.pos += 1;
                        self.para_align = Some(match name.as_str() {
                            "centering" => Align::Center,
                            "raggedright" => Align::Left,
                            _ => Align::Right,
                        });
                    }
                    "begin" => {
                        self.cur.pos += 1;
                        let g = self.cur.read_group();
                        let env = detokenize(&g);
                        self.environment(env.trim().to_string());
                    }
                    "end" => {
                        self.cur.pos += 1;
                        let env = detokenize(&self.cur.read_group());
                        if env.trim() == "document" {
                            self.flush_para();
                            self.stop_document = true;
                        }
                    }
                    "[" => {
                        // display math \[ ... \]
                        self.cur.pos += 1;
                        self.display_math_until_ctrl("]");
                    }
                    "hline" => {
                        self.cur.pos += 1;
                        self.st.warn("stray \\hline ignored");
                    }
                    _ => {
                        self.parse_paragraph();
                    }
                }
            }
            Token::Char('$', CatCode::MathShift) => {
                self.cur.pos += 1;
                if matches!(self.cur.peek(), Some(Token::Char('$', CatCode::MathShift))) {
                    self.cur.pos += 1;
                    self.display_math_until_dollar_dollar();
                } else {
                    // shouldn't happen: inline handled in paragraphs
                    self.parse_paragraph();
                }
            }
            _ => self.parse_paragraph(),
        }
    }

    fn parse_paragraph(&mut self) {
        // collect inlines until \par / block start / end
        let style = &mut Style::default();
        let inls = self.parse_inlines(style);
        self.para.extend(inls);
        // paragraph ends only at \par/block boundary; the loop in run()
        // continues collecting until flush.
    }

    fn flush_para(&mut self) {
        if self.in_preamble || !has_visible_content(&self.para) {
            self.para.clear();
            self.para_align = None;
            self.para_indent = true;
            return;
        }
        // trim leading/trailing spaces
        while matches!(self.para.first(), Some(Inline::Space)) {
            self.para.remove(0);
        }
        while matches!(
            self.para.last(),
            Some(Inline::Space) | Some(Inline::LineBreak)
        ) {
            self.para.pop();
        }
        if !self.para.is_empty() {
            let align = self.para_align.take();
            let indent = std::mem::replace(&mut self.para_indent, true);
            self.blocks.push(Block::Para {
                inlines: std::mem::take(&mut self.para),
                align,
                indent,
            });
        }
        self.para_align = None;
        self.para_indent = true;
    }

    // -----------------------------------------------------------------------
    // Inline parsing
    // -----------------------------------------------------------------------

    /// Parse inlines until a block boundary (\par, block-start token, \end).
    fn parse_inlines(&mut self, style: &mut Style) -> Vec<Inline> {
        self.parse_inlines_until(style, &|_| false)
    }

    fn parse_inlines_until(
        &mut self,
        style: &mut Style,
        extra_stop: &dyn Fn(&Token) -> bool,
    ) -> Vec<Inline> {
        let mut out: Vec<Inline> = vec![];
        let mut buf = String::new();
        let mut buf_style = style.clone();

        macro_rules! flush {
            () => {
                if !buf.is_empty() {
                    out.push(wrap_style(
                        &buf_style,
                        Inline::Text(std::mem::take(&mut buf)),
                    ));
                }
            };
        }
        macro_rules! set_style {
            ($f:expr) => {{
                flush!();
                $f(style);
                buf_style = style.clone();
            }};
        }

        while let Some(t) = self.cur.peek().cloned() {
            if extra_stop(&t) || is_block_start(&t) {
                break;
            }
            match t {
                Token::Space => {
                    self.cur.pos += 1;
                    buf.push(' ');
                }
                Token::Char('{', CatCode::BGroup) => {
                    self.cur.pos += 1;
                    flush!();
                    let saved = style.clone();
                    let inner = self.parse_inlines_until(style, &|tk| tk.is_egroup());
                    if matches!(self.cur.peek(), Some(x) if x.is_egroup()) {
                        self.cur.pos += 1;
                    }
                    *style = saved;
                    buf_style = style.clone();
                    out.push(Inline::Group(inner));
                }
                Token::Char('}', CatCode::EGroup) => break,
                Token::Char('$', CatCode::MathShift) => {
                    self.cur.pos += 1;
                    if matches!(self.cur.peek(), Some(Token::Char('$', CatCode::MathShift))) {
                        // display: stop; push back both dollars
                        self.cur.pos -= 1;
                        break;
                    }
                    // inline math
                    flush!();
                    let mut depth = 1;
                    let mut mtoks = vec![];
                    while let Some(t2) = self.cur.next() {
                        match t2 {
                            Token::Char('$', CatCode::MathShift) => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                                mtoks.push(t2);
                            }
                            other => mtoks.push(other),
                        }
                    }
                    match math::parse_math(self.st, mtoks, false) {
                        Ok(omml) => out.push(Inline::Math {
                            omml,
                            display: false,
                        }),
                        Err(e) => self.st.warn(e),
                    }
                }
                Token::Char(c, CatCode::Alignment) => {
                    self.cur.pos += 1;
                    buf.push(c);
                }
                Token::Char(c, _) => {
                    self.cur.pos += 1;
                    buf.push(c);
                }
                Token::CtrlSeq(name) => {
                    self.cur.pos += 1;
                    self.inline_command(&name, &mut out, &mut buf, style, &mut buf_style);
                }
            }
            // apply text-ligature fixes on the buffer tail cheaply later
        }
        flush!();
        fixup_text(&mut out);
        out
    }

    /// Parse inlines from a detached token list (for arguments/titles).
    fn parse_inlines_toks(&mut self, toks: Vec<Token>, style: &mut Style) -> Vec<Inline> {
        let saved = std::mem::replace(&mut self.cur, Cursor::new(toks));
        let r = self.parse_inlines(style);
        self.cur = saved;
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn inline_command(
        &mut self,
        name: &str,
        out: &mut Vec<Inline>,
        buf: &mut String,
        style: &mut Style,
        buf_style: &mut Style,
    ) {
        macro_rules! flush {
            () => {
                if !buf.is_empty() {
                    out.push(wrap_style(buf_style, Inline::Text(std::mem::take(buf))));
                }
            };
        }
        match name {
            "documentclass" | "LoadClass" => {
                let opts = self
                    .cur
                    .read_optional()
                    .map(|o| detokenize(&o))
                    .unwrap_or_default();
                let class = detokenize(&self.cur.read_group());
                self.st.class = match class.trim() {
                    "report" => ClassKind::Report,
                    "book" => ClassKind::Book,
                    _ => ClassKind::Article,
                };
                self.st.chinese = class.starts_with("ctex");
                for opt in opts.split(',') {
                    if let Some(size) = opt
                        .trim()
                        .strip_suffix("pt")
                        .and_then(|v| v.parse::<f64>().ok())
                    {
                        self.st.font_size_pt = size;
                    }
                    if opt.trim() == "a4paper" {
                        self.st.paper = "a4".into();
                    }
                }
            }
            "usepackage" | "RequirePackage" => {
                let _ = self.cur.read_optional();
                let packages = detokenize(&self.cur.read_group());
                for package in packages.split(',').map(|p| p.trim()) {
                    match package {
                        "hyperref" => self.st.hyperref_loaded = true,
                        "natbib" | "apalike" => self.bib.style = crate::bib::BibStyle::Authoryear,
                        "ctex" | "xeCJK" => self.st.chinese = true,
                        _ => {}
                    }
                }
            }
            "title" | "author" | "date" => {
                let value = self.cur.read_group();
                match name {
                    "title" => self.title = Some(value),
                    "author" => self.author = Some(value),
                    _ => self.date = Some(value),
                }
            }
            "thanks" => {
                self.thanks.push(self.cur.read_group());
            }
            "geometry"
            | "hypersetup"
            | "pagestyle"
            | "thispagestyle"
            | "pagenumbering"
            | "setmainfont"
            | "setsansfont"
            | "setmonofont"
            | "setCJKmainfont"
            | "linespread"
            | "addtocontents"
            | "markboth"
            | "markright"
            | "bibliographystyle"
            | "bibpunct"
            | "citestyle"
            | "ExecuteOptions"
            | "DeclareOption"
            | "PassOptionsToPackage" => {
                let _ = self.cur.read_group();
                if matches!(
                    name,
                    "geometry"
                        | "hypersetup"
                        | "setmainfont"
                        | "setsansfont"
                        | "setmonofont"
                        | "setCJKmainfont"
                ) {
                    let _ = self.cur.read_optional();
                }
            }
            "setlength" | "addtolength" | "addcontentsline" => {
                let _ = self.cur.read_group();
                let _ = self.cur.read_group();
                if name == "addcontentsline" {
                    let _ = self.cur.read_group();
                }
            }
            "newacronym" | "newglossaryentry" | "gls" | "Gls" | "glspl" | "Glspl" | "glslink"
            | "acrshort" | "acrlong" | "acrfull" | "Acrshort" | "Acrlong" | "Acrfull" | "ac"
            | "acp" | "Ac" | "Acp" | "acs" | "acl" | "acf" | "acrshortpl" | "acrlongpl"
            | "acrfullpl" | "glsadd" | "glsreset" | "glsunset" | "makeglossaries" => {
                self.gloss_command(name, out, buf)
            }
            "textbf" => {
                flush!();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Bold(v));
            }
            "textit" => {
                flush!();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Italic(v));
            }
            "emph" => {
                flush!();
                let g = self.cur.read_group();
                let mut sub = Style::default();
                sub.italic = !style.italic;
                let v = self.parse_inlines_toks(g, &mut sub);
                out.push(Inline::Italic(v));
            }
            "textsl" => {
                flush!();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Slanted(v));
            }
            "textsc" => {
                flush!();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::SmallCaps(v));
            }
            "textsf" => {
                flush!();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Family(FontFamily::Sf, v));
            }
            "textrm" => {
                flush!();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Group(v));
            }
            "texttt" => {
                flush!();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Family(FontFamily::Tt, v));
            }
            "textmd" | "textup" => {
                flush!();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Group(v));
            }
            "underline" => {
                flush!();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Underline(v));
            }
            "overline" => {
                flush!();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Underline(v));
            }
            "sout" | "st" => {
                flush!();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Strike(v));
            }
            "textcolor" => {
                flush!();
                let c = detokenize(&self.cur.read_group());
                let color = color_to_rgb(&c);
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Color { color, content: v });
            }
            "colorbox" => {
                flush!();
                let c = detokenize(&self.cur.read_group());
                let color = color_to_rgb(&c);
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Highlight { color, content: v });
            }
            "fcolorbox" => {
                flush!();
                let _border = detokenize(&self.cur.read_group());
                let c = detokenize(&self.cur.read_group());
                let color = color_to_rgb(&c);
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Highlight { color, content: v });
            }
            "color" => {
                flush!();
                let c = detokenize(&self.cur.read_group());
                style.color = Some(color_to_rgb(&c));
                *buf_style = style.clone();
            }
            "rmfamily" => {
                flush!();
                style.mono = false;
                style.sans = false;
                *buf_style = style.clone();
            }
            "sffamily" => {
                flush!();
                style.sans = true;
                style.mono = false;
                *buf_style = style.clone();
            }
            "ttfamily" => {
                flush!();
                style.mono = true;
                *buf_style = style.clone();
            }
            "bfseries" => {
                flush!();
                style.bold = true;
                *buf_style = style.clone();
            }
            "mdseries" => {
                flush!();
                style.bold = false;
                *buf_style = style.clone();
            }
            "itshape" => {
                flush!();
                style.italic = true;
                style.slanted = false;
                *buf_style = style.clone();
            }
            "slshape" => {
                flush!();
                style.slanted = true;
                style.italic = false;
                *buf_style = style.clone();
            }
            "scshape" => {
                flush!();
                style.smallcaps = true;
                *buf_style = style.clone();
            }
            "upshape" => {
                flush!();
                style.italic = false;
                style.slanted = false;
                style.smallcaps = false;
                *buf_style = style.clone();
            }
            "bf" => {
                flush!();
                style.bold = true;
                *buf_style = style.clone();
            }
            "it" => {
                flush!();
                style.italic = true;
                *buf_style = style.clone();
            }
            "rm" => {
                flush!();
                style.mono = false;
                style.sans = false;
                *buf_style = style.clone();
            }
            "sf" => {
                flush!();
                style.sans = true;
                *buf_style = style.clone();
            }
            "tt" => {
                flush!();
                style.mono = true;
                *buf_style = style.clone();
            }
            "sc" => {
                flush!();
                style.smallcaps = true;
                *buf_style = style.clone();
            }
            "sl" => {
                flush!();
                style.slanted = true;
                *buf_style = style.clone();
            }
            "cal" => {}
            "em" => {
                flush!();
                style.italic = !style.italic;
                *buf_style = style.clone();
            }
            "tiny" => {
                flush!();
                style.size = Some(FontSize::Tiny);
                *buf_style = style.clone();
            }
            "scriptsize" => {
                flush!();
                style.size = Some(FontSize::Scriptsize);
                *buf_style = style.clone();
            }
            "footnotesize" => {
                flush!();
                style.size = Some(FontSize::Footnotesize);
                *buf_style = style.clone();
            }
            "small" => {
                flush!();
                style.size = Some(FontSize::Small);
                *buf_style = style.clone();
            }
            "normalsize" => {
                flush!();
                style.size = None;
                *buf_style = style.clone();
            }
            "large" => {
                flush!();
                style.size = Some(FontSize::Large);
                *buf_style = style.clone();
            }
            "Large" => {
                flush!();
                style.size = Some(FontSize::LargeX);
                *buf_style = style.clone();
            }
            "LARGE" => {
                flush!();
                style.size = Some(FontSize::LargeXX);
                *buf_style = style.clone();
            }
            "huge" => {
                flush!();
                style.size = Some(FontSize::Huge);
                *buf_style = style.clone();
            }
            "Huge" => {
                flush!();
                style.size = Some(FontSize::HugeX);
                *buf_style = style.clone();
            }
            "quad" => buf.push(' '),
            "qquad" => buf.push_str("  "),
            "," | ";" | ":" => buf.push(' '),
            "!" => {}
            " " | "space" | "@" => buf.push(' '),
            "nobreakspace" => {
                flush!();
                out.push(Inline::Nbsp);
            }
            "~" => {
                buf.push(' ');
            }
            "newline" | "linebreak" | "tabularnewline" => {
                flush!();
                out.push(Inline::LineBreak);
            }
            "\\" => {
                // row separator used in text: treat as line break
                flush!();
                out.push(Inline::LineBreak);
            }
            "hspace" | "hspace*" => {
                let _ = self.cur.read_group();
                buf.push(' ');
            }
            "hfill" | "dotfill" | "hrulefill" => buf.push(' '),
            "textasciitilde" => buf.push('~'),
            "textasciicircum" => buf.push('^'),
            "textbackslash" => buf.push('\\'),
            "textbar" => buf.push('|'),
            "textquotedbl" => buf.push('"'),
            "textquotesingle" => buf.push('\''),
            "textless" => buf.push('<'),
            "textgreater" => buf.push('>'),
            "textemdash" => buf.push('—'),
            "textendash" => buf.push('–'),
            "textexclamdown" => buf.push('¡'),
            "textquestiondown" => buf.push('¿'),
            "textsterling" => buf.push('£'),
            "textcurrency" => buf.push('¤'),
            "textperiodcentered" => buf.push('·'),
            "S" => buf.push('§'),
            "P" => buf.push('¶'),
            "copyright" | "textcopyright" => buf.push('©'),
            "textregistered" => buf.push('®'),
            "texttrademark" => buf.push('™'),
            "textbullet" => buf.push('•'),
            "dag" => buf.push('†'),
            "ddag" => buf.push('‡'),
            "dots" | "ldots" | "textellipsis" => buf.push('…'),
            "deg" | "textdegree" => buf.push('°'),
            "ae" => buf.push('æ'),
            "AE" => buf.push('Æ'),
            "oe" => buf.push('œ'),
            "OE" => buf.push('Œ'),
            "aa" => buf.push('å'),
            "AA" => buf.push('Å'),
            "o" => buf.push('ø'),
            "O" => buf.push('Ø'),
            "l" => buf.push('ł'),
            "L" => buf.push('Ł'),
            "ss" => buf.push('ß'),
            "i" => buf.push('ı'),
            "j" => buf.push('ȷ'),
            "lq" => buf.push('‘'),
            "rq" => buf.push('’'),
            "not" => buf.push('¬'),
            "sb" | "textsubscript" => {
                flush!();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Subscript(v));
            }
            "sp" | "textsuperscript" => {
                flush!();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Superscript(v));
            }
            "mbox" | "makebox" | "fbox" | "framebox" => {
                flush!();
                // framebox may have args before content
                if name == "framebox" {
                    let _ = self.cur.read_optional();
                    let w = self.cur.read_group();
                    if !w.is_empty() && !w[0].is_bgroup() {
                        // width was given as single token group; content next
                    }
                }
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Group(v));
            }
            "raisebox" => {
                flush!();
                let _ = self.cur.read_group();
                let _ = self.cur.read_optional();
                let _ = self.cur.read_optional();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Group(v));
            }
            "phantom" | "hphantom" => {
                flush!();
                let _ = self.cur.read_group();
            }
            "ensuremath" => {
                flush!();
                let g = self.cur.read_group();
                match math::parse_math(self.st, g, false) {
                    Ok(omml) => out.push(Inline::Math {
                        omml,
                        display: false,
                    }),
                    Err(e) => self.st.warn(e),
                }
            }
            "footnote" => {
                flush!();
                let g = self.cur.read_group();
                self.st.refstep("footnote");
                let n = self.st.the_counter("footnote");
                let blocks = {
                    let saved = std::mem::replace(&mut self.cur, Cursor::new(g));
                    let saved_blocks = std::mem::take(&mut self.blocks);
                    let saved_para = std::mem::take(&mut self.para);
                    self.parse_blocks_inner(&|_| false);
                    self.flush_para();
                    let b = std::mem::take(&mut self.blocks);
                    self.blocks = saved_blocks;
                    self.para = saved_para;
                    self.cur = saved;
                    let _ = n;
                    b
                };
                let fb = if blocks.len() == 1 {
                    blocks.into_iter().next().unwrap()
                } else {
                    Block::AlignBlock(Align::Justify, blocks)
                };
                self.footnotes.push(fb.clone());
                out.push(Inline::Footnote(Box::new(fb)));
            }
            "footnotemark" => {
                flush!();
                self.st.step_counter("footnote");
            }
            "footnotetext" => {
                flush!();
                let g = self.cur.read_group();
                let blocks = {
                    let saved = std::mem::replace(&mut self.cur, Cursor::new(g));
                    let saved_blocks = std::mem::take(&mut self.blocks);
                    let saved_para = std::mem::take(&mut self.para);
                    self.parse_blocks_inner(&|_| false);
                    self.flush_para();
                    let b = std::mem::take(&mut self.blocks);
                    self.blocks = saved_blocks;
                    self.para = saved_para;
                    self.cur = saved;
                    b
                };
                let fb = if blocks.len() == 1 {
                    blocks.into_iter().next().unwrap()
                } else {
                    Block::AlignBlock(Align::Justify, blocks)
                };
                self.footnotes.push(fb);
            }
            "label" => {
                flush!();
                let g = self.cur.read_group();
                let key = detokenize(&g).trim().to_string();
                self.register_label(&key);
                out.push(Inline::Meta(format!(
                    "anchor:{}",
                    bib::sanitize_anchor(&key)
                )));
            }
            "ref" | "eqref" | "pageref" | "autoref" | "cref" | "Cref" | "nameref" => {
                flush!();
                let g = self.cur.read_group();
                let key = detokenize(&g).trim().to_string();
                out.push(self.make_ref(&key, name));
            }
            "crefname" => {
                flush!();
                let ctr = detokenize(&self.cur.read_group());
                let sing = detokenize(&self.cur.read_group());
                let plur = detokenize(&self.cur.read_group());
                self.st.crefnames.insert(ctr, (sing, plur));
            }
            "Crefname" => {
                flush!();
                let ctr = detokenize(&self.cur.read_group());
                let sing = detokenize(&self.cur.read_group());
                let plur = detokenize(&self.cur.read_group());
                self.st.crefnames.insert(ctr, (sing, plur));
            }
            "cite" | "citet" | "citep" | "citealp" | "citealt" | "citeauthor" | "citeyear"
            | "citeyearpar" | "citeonline" | "nocite" => {
                flush!();
                let _opt = self.cur.read_optional();
                let g = self.cur.read_group();
                let keys: Vec<String> = detokenize(&g)
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if name == "nocite" {
                    self.bib.cite(&keys);
                } else {
                    self.bib.cite(&keys);
                    let textual = matches!(
                        name,
                        "citet"
                            | "citealt"
                            | "citeonline"
                            | "citeauthor"
                            | "citeyear"
                            | "citeyearpar"
                    );
                    out.push(self.make_cite(&keys, textual, name));
                }
            }
            "url" | "path" | "nolinkurl" => {
                flush!();
                let g = self.cur.read_group();
                let url = detokenize(&g);
                out.push(Inline::Link {
                    url: url.clone(),
                    content: vec![Inline::Family(FontFamily::Tt, vec![Inline::Text(url)])],
                    external: true,
                });
            }
            "href" => {
                flush!();
                let url = detokenize(&self.cur.read_group());
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                out.push(Inline::Link {
                    url,
                    content: v,
                    external: true,
                });
            }
            "hyperref" => {
                flush!();
                let opt = self.cur.read_optional();
                let g = self.cur.read_group();
                let v = self.parse_inlines_toks(g, &mut Style::default());
                if let Some(o) = opt {
                    let key = detokenize(&o).trim().to_string();
                    let r = self.make_ref(&key, "ref");
                    out.push(Inline::Link {
                        url: format!("#{}", bib::sanitize_anchor(&key)),
                        content: v,
                        external: false,
                    });
                    let _ = r;
                } else {
                    out.push(Inline::Group(v));
                }
            }
            "includegraphics" => {
                flush!();
                if let Some(img) = self.read_graphics() {
                    out.push(Inline::Image(img));
                }
            }
            "item" => {
                // \item outside a list: ignore
            }
            "index" | "glossary" => {
                let _ = self.cur.read_group();
            }
            "maketitle" | "tableofcontents" | "listoffigures" | "listoftables" | "bibliography"
            | "printglossaries" | "printglossary" | "printacronyms" | "newpage" | "clearpage"
            | "pagebreak" | "section" | "subsection" | "begin" | "end" | "par" => {
                // block-level: push back and let caller handle
                self.cur.pos -= 1;
                return;
            }
            "protect" => {}
            "hyphenation" => {
                let _ = self.cur.read_group();
            }
            "_" => buf.push('_'),
            "^" => buf.push('^'),
            "%" => buf.push('%'),
            "&" => buf.push('&'),
            "#" => buf.push('#'),
            "$" => buf.push('$'),
            "{" => buf.push('{'),
            "}" => buf.push('}'),
            "selectlanguage" | "otherlanguage" => {
                let _ = self.cur.read_group();
            }
            "today" => {
                buf.push_str("today");
            }
            "caption" | "captionof" => {
                self.cur.pos -= 1;
                return;
            }
            "@verbstart" => {
                flush!();
                let lines = self.collect_verb_lines();
                out.push(Inline::Code(lines.join("\n")));
            }
            "mboxempty" => {}
            _ => {
                // fontAwesome? plain symbol fallback: render the name literally
                if name.chars().all(|c| !c.is_ascii_alphabetic()) && name.len() == 1 {
                    buf.push_str(name);
                } else if name.chars().all(|c| !c.is_ascii_alphabetic()) {
                    buf.push_str(name);
                } else {
                    self.st.warn(format!(
                        "unsupported control sequence \\{}; rendered literally",
                        name
                    ));
                    buf.push('\\');
                    buf.push_str(name);
                }
            }
        }
    }

    fn collect_verb_lines(&mut self) -> Vec<String> {
        let mut lines = vec![];
        while let Some(t) = self.cur.next() {
            match t {
                Token::CtrlSeq(n) if n == "@verbline" => {
                    let mut line = String::new();
                    while let Some(t2) = self.cur.peek().cloned() {
                        match t2 {
                            Token::Char(c, _) => {
                                line.push(c);
                                self.cur.pos += 1;
                            }
                            _ => break,
                        }
                    }
                    lines.push(line);
                }
                Token::CtrlSeq(n) if n == "@verbend" => break,
                _ => {}
            }
        }
        lines
    }

    // -----------------------------------------------------------------------
    // References
    // -----------------------------------------------------------------------

    fn register_label(&mut self, key: &str) {
        let (number, counter, title) = match &self.st.current_ref {
            Some(r) => (r.number.clone(), r.counter.clone(), r.title.clone()),
            None => {
                self.st
                    .warn(format!("\\label{{{}}} with no current number", key));
                (String::new(), String::new(), None)
            }
        };
        self.labels.push((
            key.to_string(),
            number.clone(),
            title.clone(),
            counter.clone(),
        ));
        // store in aux-like map for later refs this pass
        self.st.current_ref = Some(RefTarget {
            counter,
            number,
            title,
        });
    }

    fn lookup_label(&self, key: &str) -> Option<(String, String, Option<String>)> {
        // labels already written this pass take precedence
        for (k, n, t, c) in self.labels.iter().rev() {
            if k == key {
                return Some((n.clone(), c.clone(), t.clone()));
            }
        }
        self.aux
            .labels
            .get(key)
            .map(|l| (l.number.clone(), l.counter.clone(), l.title.clone()))
    }

    fn make_ref(&mut self, key: &str, kind: &str) -> Inline {
        let found = self.lookup_label(key);
        let anchor = bib::sanitize_anchor(key);
        let reference =
            |text: String, ref_kind: RefKind, counter: String, anchor: String| Inline::Ref {
                key: key.into(),
                text,
                anchor: Some(anchor),
                kind: ref_kind,
                counter,
            };
        match found {
            Some((number, counter, title)) => match kind {
                "eqref" => Inline::Group(vec![
                    Inline::Text("(".into()),
                    reference(number, RefKind::Number, counter, anchor),
                    Inline::Text(")".into()),
                ]),
                "pageref" => reference("1".into(), RefKind::Page, counter, anchor),
                "nameref" => {
                    if let Some(title) = title {
                        let title_anchor = if matches!(
                            counter.as_str(),
                            "part"
                                | "chapter"
                                | "section"
                                | "subsection"
                                | "subsubsection"
                                | "paragraph"
                                | "subparagraph"
                        ) {
                            anchor
                        } else {
                            format!("{}_title", anchor)
                        };
                        reference(title, RefKind::Name, counter, title_anchor)
                    } else {
                        reference(number, RefKind::Number, counter, anchor)
                    }
                }
                "autoref" | "cref" | "Cref" => {
                    let (mut name, _) = self.refname_for(&counter);
                    if kind == "Cref" {
                        let mut chars = name.chars();
                        if let Some(first) = chars.next() {
                            name = first.to_uppercase().collect::<String>() + chars.as_str();
                        }
                    }
                    let sep = if self.st.chinese { "" } else { " " };
                    Inline::Group(vec![
                        Inline::Text(format!("{}{}", name, sep)),
                        reference(number, RefKind::Number, counter, anchor),
                    ])
                }
                _ => reference(number, RefKind::Number, counter, anchor),
            },
            None => {
                self.st.warn(format!("reference `{}` undefined", key));
                Inline::Ref {
                    key: key.into(),
                    text: "??".into(),
                    anchor: None,
                    kind: RefKind::Number,
                    counter: String::new(),
                }
            }
        }
    }

    fn refname_for(&self, counter: &str) -> (String, String) {
        if let Some((s, p)) = self.st.crefnames.get(counter) {
            return (s.clone(), p.clone());
        }
        let cn = self.st.chinese;
        let (s, p): (&str, &str) = match counter {
            "figure" => {
                if cn {
                    ("图", "图")
                } else {
                    ("figure", "figures")
                }
            }
            "table" => {
                if cn {
                    ("表", "表")
                } else {
                    ("table", "tables")
                }
            }
            "equation" => {
                if cn {
                    ("式", "式")
                } else {
                    ("equation", "equations")
                }
            }
            "chapter" => {
                if cn {
                    ("章", "章")
                } else {
                    ("chapter", "chapters")
                }
            }
            "section" => {
                if cn {
                    ("节", "节")
                } else {
                    ("section", "sections")
                }
            }
            "subsection" | "subsubsection" => {
                if cn {
                    ("小节", "小节")
                } else {
                    ("section", "sections")
                }
            }
            "part" => {
                if cn {
                    ("部分", "部分")
                } else {
                    ("part", "parts")
                }
            }
            "footnote" => {
                if cn {
                    ("脚注", "脚注")
                } else {
                    ("footnote", "footnotes")
                }
            }
            "page" => {
                if cn {
                    ("页", "页")
                } else {
                    ("page", "pages")
                }
            }
            "enumi" | "enumii" | "enumiii" | "enumiv" => {
                if cn {
                    ("项", "项")
                } else {
                    ("item", "items")
                }
            }
            _ => {
                if let Some((name, _)) = self.st.theorems.get(counter) {
                    return (name.clone(), format!("{}s", name));
                }
                if cn {
                    ("项", "项")
                } else {
                    ("item", "items")
                }
            }
        };
        (s.to_string(), p.to_string())
    }

    fn make_cite(&mut self, keys: &[String], textual: bool, cmd: &str) -> Inline {
        let (text, links) = self.bib.render_cite(keys, textual);
        let final_text = match cmd {
            "citeauthor" => {
                let mut v = vec![];
                for k in keys {
                    if let Some(e) = self.bib.entries.get(k) {
                        v.push(crate::bib::short_authors_pub(e));
                    }
                }
                if v.is_empty() {
                    text
                } else {
                    v.join("; ")
                }
            }
            "citeyear" | "citeyearpar" => {
                let mut v = vec![];
                for k in keys {
                    if let Some(e) = self.bib.entries.get(k) {
                        v.push(crate::bib::year_of_pub(e));
                    }
                }
                if v.is_empty() {
                    text
                } else {
                    v.join(", ")
                }
            }
            _ => text,
        };
        Inline::Cite {
            text: final_text,
            links,
        }
    }

    // -----------------------------------------------------------------------
    // Glossaries / acronyms
    // -----------------------------------------------------------------------

    fn gloss_command(&mut self, name: &str, out: &mut Vec<Inline>, buf: &mut String) {
        if !buf.is_empty() {
            out.push(Inline::Text(std::mem::take(buf)));
        }
        match name {
            "newacronym" => {
                let opts = self.cur.read_optional();
                let lab = detokenize(&self.cur.read_group()).trim().to_string();
                let short = detokenize(&self.cur.read_group());
                let long = detokenize(&self.cur.read_group());
                let mut e = GlossEntry {
                    short,
                    long,
                    first: None,
                    short_plural: None,
                    long_plural: None,
                    description: None,
                    used: false,
                    is_acronym: true,
                };
                if let Some(o) = opts {
                    for kv in split_keyvals(&detokenize(&o)) {
                        if let Some((k, v)) = kv.split_once('=') {
                            match k.trim() {
                                "shortplural" => e.short_plural = Some(strip_outer_group(v)),
                                "longplural" => e.long_plural = Some(strip_outer_group(v)),
                                "first" => e.first = Some(strip_outer_group(v)),
                                "description" => e.description = Some(strip_outer_group(v)),
                                _ => {}
                            }
                        }
                    }
                }
                self.st.gloss.entries.insert(lab, e);
            }
            "newglossaryentry" => {
                let lab = detokenize(&self.cur.read_group()).trim().to_string();
                let body = detokenize(&self.cur.read_group());
                let mut e = GlossEntry {
                    short: String::new(),
                    long: String::new(),
                    first: None,
                    short_plural: None,
                    long_plural: None,
                    description: None,
                    used: false,
                    is_acronym: false,
                };
                let mut name_v = None;
                for kv in split_keyvals(&body) {
                    if let Some((k, v)) = kv.split_once('=') {
                        match k.trim() {
                            "name" => name_v = Some(strip_outer_group(v)),
                            "description" => e.description = Some(strip_outer_group(v)),
                            "first" => e.first = Some(strip_outer_group(v)),
                            "plural" => e.long_plural = Some(strip_outer_group(v)),
                            "long" => e.long = strip_outer_group(v),
                            "short" => e.short = strip_outer_group(v),
                            _ => {}
                        }
                    }
                }
                if e.long.is_empty() {
                    e.long = name_v.unwrap_or_default();
                }
                if e.short.is_empty() {
                    e.short = e.long.clone();
                }
                self.st.gloss.entries.insert(lab, e);
            }
            "gls" | "Gls" | "glspl" | "Glspl" | "acrshort" | "acrlong" | "acrfull" | "Acrshort"
            | "Acrlong" | "Acrfull" | "ac" | "acp" | "Ac" | "Acp" | "acs" | "acl" | "acf"
            | "acrshortpl" | "acrlongpl" | "acrfullpl" => {
                let lab = detokenize(&self.cur.read_group()).trim().to_string();
                let plural = name.ends_with("pl")
                    || name == "glspl"
                    || name == "Glspl"
                    || name == "acp"
                    || name == "Acp";
                let text = self.gloss_use(&lab, name, plural);
                out.push(Inline::Text(text));
            }
            "glslink" => {
                let lab = detokenize(&self.cur.read_group()).trim().to_string();
                let g = detokenize(&self.cur.read_group());
                let _ = lab;
                out.push(Inline::Text(g));
            }
            "glsadd" | "glsreset" | "glsunset" => {
                let _ = self.cur.read_group();
            }
            "makeglossaries" => {}
            "printglossaries" | "printglossary" | "printacronyms" => {
                self.cur.pos -= 1; // let block level handle
            }
            _ => {}
        }
    }

    fn gloss_use(&mut self, lab: &str, cmd: &str, plural: bool) -> String {
        let entry = match self.st.gloss.entries.get_mut(lab) {
            Some(e) => e,
            None => {
                self.st.warn(format!("glossary entry `{}` undefined", lab));
                return format!("[{}?]", lab);
            }
        };
        let capitalize = cmd.starts_with('A')
            && cmd != "acrshort"
            && cmd != "acrlong"
            && cmd != "acrfull"
            && cmd != "acrshortpl"
            && cmd != "acrlongpl"
            && cmd != "acrfullpl";
        let first_use = !entry.used;
        entry.used = true;
        let text = match cmd {
            "acrshort" | "acrshortpl" | "acs" => {
                if plural {
                    entry.plural_short()
                } else {
                    entry.short.clone()
                }
            }
            "acrlong" | "acrlongpl" | "acl" => {
                if plural {
                    entry.plural_long()
                } else {
                    entry.long.clone()
                }
            }
            "acrfull" | "acrfullpl" | "acf" => {
                let l = if plural {
                    entry.plural_long()
                } else {
                    entry.long.clone()
                };
                let s = if plural {
                    entry.plural_short()
                } else {
                    entry.short.clone()
                };
                format!("{} ({})", l, s)
            }
            "glsreset" | "glsunset" => entry.short.clone(),
            _ => {
                // \gls / \ac: full on first use, short afterwards
                if first_use {
                    match &entry.first {
                        Some(f) => f.clone(),
                        None => {
                            let l = if plural {
                                entry.plural_long()
                            } else {
                                entry.long.clone()
                            };
                            let s = if plural {
                                entry.plural_short()
                            } else {
                                entry.short.clone()
                            };
                            if entry.is_acronym || entry.short != entry.long {
                                format!("{} ({})", l, s)
                            } else {
                                l
                            }
                        }
                    }
                } else if plural {
                    entry.plural_short()
                } else {
                    entry.short.clone()
                }
            }
        };
        if capitalize {
            let mut cs = text.chars();
            match cs.next() {
                Some(f) => f.to_uppercase().collect::<String>() + cs.as_str(),
                None => text,
            }
        } else {
            text
        }
    }

    fn render_glossary(&mut self, which: &str) {
        let mut items: Vec<GlossItem> = self
            .st
            .gloss
            .entries
            .iter()
            .map(|(k, e)| GlossItem {
                abbr: e.short.clone(),
                long: e.long.clone(),
                desc: e.description.clone(),
            })
            .collect();
        items.sort_by(|a, b| a.abbr.cmp(&b.abbr));
        let title = match which {
            "printacronyms" => {
                if self.st.chinese {
                    "缩略语表".into()
                } else {
                    "Acronyms".into()
                }
            }
            _ => {
                if self.st.chinese {
                    "术语表".into()
                } else {
                    "Glossary".into()
                }
            }
        };
        self.blocks.push(Block::GlossSection {
            title,
            entries: items,
        });
    }

    // -----------------------------------------------------------------------
    // Bibliography
    // -----------------------------------------------------------------------

    fn render_bibliography(&mut self) {
        let title = if self.st.chinese {
            "参考文献".into()
        } else {
            "References".into()
        };
        let items = self.bib.to_model_items();
        self.blocks.push(Block::BibSection { title, items });
    }

    // -----------------------------------------------------------------------
    // Headings
    // -----------------------------------------------------------------------

    fn heading(&mut self, name: &str) {
        self.cur.pos += 1;
        self.flush_para();
        // star?
        let starred = {
            let save = self.cur.pos;
            self.cur.skip_spaces();
            matches!(self.cur.peek(), Some(Token::Char('*', _)))
        };
        if starred {
            self.cur.pos += 1;
        }
        let _short = self.cur.read_optional();
        let title_toks = self.cur.read_group();

        let (counter, kind): (&str, HeadingKind) = match name {
            "part" => ("part", HeadingKind::Part),
            "chapter" => ("chapter", HeadingKind::Chapter),
            "section" => ("section", HeadingKind::Section),
            _ => ("section", HeadingKind::Other),
        };
        let _ = counter;

        let number = if !starred {
            self.st.refstep(name);
            let n = self.st.the_counter(name);
            // appendix: sections lettered
            if self.st.appendix && name == "section" {
                let v = self.st.counter_value("section");
                crate::engine::format_number(v, "Alph")
            } else {
                n
            }
        } else {
            String::new()
        };

        // set current ref BEFORE parsing title so \label in title works
        self.st.current_ref = Some(RefTarget {
            counter: name.to_string(),
            number: number.clone(),
            title: None,
        });

        let mut style = Style::default();
        let title = self.parse_inlines_toks(title_toks, &mut style);
        let plain_title = inlines_to_text(&title);
        self.st.current_ref = Some(RefTarget {
            counter: name.to_string(),
            number: number.clone(),
            title: Some(plain_title.clone()),
        });

        // absorb an immediately following \label{...}
        let mut anchor = None;
        {
            let save = self.cur.pos;
            self.cur.skip_spaces();
            if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "label") {
                self.cur.pos += 1;
                let g = self.cur.read_group();
                let key = detokenize(&g).trim().to_string();
                anchor = Some(bib::sanitize_anchor(&key));
                self.register_label(&key);
            } else {
                self.cur.pos = save;
            }
        }

        let level = self.heading_level(name);
        self.blocks.push(Block::Heading {
            level,
            number: if number.is_empty() {
                None
            } else {
                Some(number)
            },
            title,
            anchor,
            kind,
        });
    }

    fn heading_level(&self, name: &str) -> u8 {
        let rank = match name {
            "part" => 1,
            "chapter" => 1,
            "section" => 2,
            "subsection" => 3,
            "subsubsection" => 4,
            "paragraph" => 5,
            "subparagraph" => 6,
            _ => 2,
        };
        let base = if self.st.class == ClassKind::Article {
            -1
        } else {
            0
        };
        (rank + base).clamp(1, 9) as u8
    }

    fn make_title(&mut self) {
        let title = self
            .title
            .take()
            .map(|t| self.parse_inlines_toks(t, &mut Style::default()))
            .unwrap_or_default();
        let author = self
            .author
            .take()
            .map(|t| self.parse_inlines_toks(t, &mut Style::default()))
            .unwrap_or_default();
        let date = self
            .date
            .take()
            .map(|t| self.parse_inlines_toks(t, &mut Style::default()))
            .unwrap_or_default();
        let thanks: Vec<Vec<Inline>> = std::mem::take(&mut self.thanks)
            .into_iter()
            .map(|t| self.parse_inlines_toks(t, &mut Style::default()))
            .collect();
        self.blocks.push(Block::TitlePage {
            title,
            author,
            date,
            thanks,
        });
    }

    // -----------------------------------------------------------------------
    // Math displays
    // -----------------------------------------------------------------------

    fn eq_step(&mut self) -> String {
        if let Some((parent, letter)) = &mut self.st.subeq_active {
            *letter += 1;
            let l = (*letter as char).to_string();
            let n = format!("{}{}", parent, l);
            self.st.current_ref = Some(RefTarget {
                counter: "equation".into(),
                number: n.clone(),
                title: None,
            });
            n
        } else {
            self.st.refstep("equation");
            self.st.the_counter("equation")
        }
    }

    /// Collect math tokens until `\end{env}`; returns content tokens.
    fn collect_until_end(&mut self, env: &str) -> Vec<Token> {
        let mut out = vec![];
        let mut depth = 0;
        loop {
            let t = match self.cur.next() {
                Some(t) => t,
                None => break,
            };
            match &t {
                Token::CtrlSeq(n) if n == "begin" => {
                    depth += 1;
                    out.push(t);
                }
                Token::CtrlSeq(n) if n == "end" && depth == 0 => {
                    let g = self.cur.read_group();
                    let e = detokenize(&g);
                    if e.trim() == env {
                        break;
                    }
                    out.push(Token::cs("end"));
                    out.extend(g);
                }
                Token::CtrlSeq(n) if n == "end" => {
                    depth -= 1;
                    out.push(t);
                }
                _ => out.push(t),
            }
        }
        out
    }

    /// Strip \label/\nonumber/\tag from a math row; returns (clean, label, tag, nonumber).
    fn strip_math_markers(
        &mut self,
        toks: Vec<Token>,
    ) -> (Vec<Token>, Option<String>, Option<String>, bool) {
        let mut clean = vec![];
        let mut label = None;
        let mut tag = None;
        let mut nonumber = false;
        let mut cur = Cursor::new(toks);
        while let Some(t) = cur.next() {
            match t {
                Token::CtrlSeq(n) if n == "label" => {
                    let g = cur.read_group();
                    label = Some(detokenize(&g).trim().to_string());
                }
                Token::CtrlSeq(n) if n == "nonumber" || n == "notag" => {
                    nonumber = true;
                }
                Token::CtrlSeq(n) if n == "tag" || n == "tag*" => {
                    let g = cur.read_group();
                    tag = Some(detokenize(&g));
                }
                other => clean.push(other),
            }
        }
        (clean, label, tag, nonumber)
    }

    fn register_math_label(&mut self, key: &str, number: &str) {
        self.labels.push((
            key.to_string(),
            number.to_string(),
            None,
            "equation".to_string(),
        ));
    }

    fn display_math_tokens(&mut self, toks: Vec<Token>, numbered: bool, env: &str) {
        let _ = env;
        let (clean, label, tag, nonumber) = self.strip_math_markers(toks);
        let number = if numbered && !nonumber {
            Some(self.eq_step())
        } else {
            None
        };
        if let (Some(k), Some(n)) = (&label, &number) {
            self.register_math_label(k, n);
        }
        match math::parse_math(self.st, clean, true) {
            Ok(omml) => {
                let anchor = label.as_ref().map(|k| bib::sanitize_anchor(k));
                self.blocks.push(Block::DisplayMath {
                    omml,
                    number: number.clone(),
                    tag: tag.clone(),
                    anchor,
                });
            }
            Err(e) => self.st.warn(e),
        }
    }

    fn display_math_until_ctrl(&mut self, end: &str) {
        self.flush_para();
        let mut toks = vec![];
        while let Some(t) = self.cur.next() {
            if t.is_cs(end) {
                break;
            }
            toks.push(t);
        }
        self.display_math_tokens(toks, false, "displaymath");
    }

    fn display_math_until_dollar_dollar(&mut self) {
        self.flush_para();
        let mut toks = vec![];
        while let Some(t) = self.cur.next() {
            if matches!(t, Token::Char('$', CatCode::MathShift)) {
                if matches!(self.cur.peek(), Some(Token::Char('$', CatCode::MathShift))) {
                    self.cur.pos += 1;
                    break;
                }
            }
            toks.push(t);
        }
        self.display_math_tokens(toks, false, "displaymath");
    }

    /// Split math content into rows at top-level \\.
    fn split_rows(&mut self, toks: Vec<Token>) -> Vec<Vec<Token>> {
        let mut rows = vec![];
        let mut cur = vec![];
        let mut depth = 0;
        let mut nested_env = 0;
        let mut c = Cursor::new(toks);
        while let Some(t) = c.next() {
            match &t {
                Token::Char('{', CatCode::BGroup) => depth += 1,
                Token::Char('}', CatCode::EGroup) => depth -= 1,
                Token::CtrlSeq(n) if n == "begin" => {
                    nested_env += 1;
                }
                Token::CtrlSeq(n) if n == "end" && nested_env > 0 => {
                    nested_env -= 1;
                }
                Token::CtrlSeq(n)
                    if (n == "\\" || n == "newline" || n == "tabularnewline")
                        && depth == 0
                        && nested_env == 0 =>
                {
                    rows.push(std::mem::take(&mut cur));
                    continue;
                }
                _ => {}
            }
            cur.push(t);
        }
        rows.push(cur);
        rows
    }

    fn display_math_env(&mut self, env: &str, starred: bool) {
        let toks = self.collect_until_end(env);
        let rows = self.split_rows(toks);
        // decide numbering
        let mut row_info = vec![];
        let mut any_numbered = false;
        for r in rows {
            let (clean, label, tag, nonumber) = self.strip_math_markers(r);
            let wants = !starred && !nonumber;
            if wants {
                any_numbered = true;
            }
            row_info.push((clean, label, tag, wants));
        }
        if !any_numbered {
            // single eqArr block
            let mut omml = String::new();
            omml.push_str("<m:oMathPara><m:oMath>");
            omml.push_str("<m:eqArr><m:eqArrPr><m:maxDist m:val=\"0\"/></m:eqArrPr>");
            for (clean, _, _, _) in &row_info {
                omml.push_str("<m:e>");
                match math::parse_math_row(self.st, clean.clone(), true) {
                    Ok(s) => omml.push_str(&s),
                    Err(e) => self.st.warn(e),
                }
                omml.push_str("</m:e>");
            }
            omml.push_str("</m:eqArr></m:oMath></m:oMathPara>");
            self.blocks.push(Block::DisplayMath {
                omml,
                number: None,
                tag: None,
                anchor: None,
            });
        } else {
            for (clean, label, tag, wants) in row_info {
                let number = if wants { Some(self.eq_step()) } else { None };
                if let (Some(k), Some(n)) = (&label, &number) {
                    self.register_math_label(k, n);
                }
                match math::parse_math(self.st, clean, true) {
                    Ok(omml) => {
                        let anchor = label.as_ref().map(|k| bib::sanitize_anchor(k));
                        self.blocks.push(Block::DisplayMath {
                            omml,
                            number: number.clone(),
                            tag: tag.clone(),
                            anchor,
                        });
                    }
                    Err(e) => self.st.warn(e),
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Graphics
    // -----------------------------------------------------------------------

    fn read_graphics(&mut self) -> Option<Image> {
        let mut scale = 1.0f64;
        let mut width: Option<f64> = None;
        let mut height: Option<f64> = None;
        if let Some(opts) = self.cur.read_optional() {
            let text_width_pt = 469.0; // 6.5in text block
            for kv in split_keyvals(&detokenize(&opts)) {
                if let Some((k, v)) = kv.split_once('=') {
                    let k = k.trim();
                    let v = v.trim();
                    match k {
                        "scale" => scale = v.parse().unwrap_or(1.0),
                        "width" => width = Some(parse_len_frac(v, text_width_pt)),
                        "height" => height = Some(parse_len_frac(v, text_width_pt)),
                        _ => {}
                    }
                }
            }
        }
        let name = detokenize(&self.cur.read_group()).trim().to_string();
        // find file
        let mut path = std::path::PathBuf::from(&name);
        let mut tried = vec![path.clone()];
        if path.extension().is_none() {
            for ext in [
                "png", "jpg", "jpeg", "pdf", "svg", "PNG", "JPG", "JPEG", "PDF", "SVG",
            ] {
                let p = std::path::PathBuf::from(format!("{}.{}", name, ext));
                tried.push(p.clone());
                if p.exists() {
                    path = p;
                    break;
                }
            }
        }
        if !path.exists() {
            self.st.warn(format!("image not found: {}", name));
            let _ = tried;
            return None;
        }
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let data = if extension == "pdf" {
            match pdf_to_svg(&path) {
                Ok(data) => data,
                Err(error) => {
                    self.st
                        .warn(format!("cannot convert PDF image `{}`: {}", name, error));
                    return None;
                }
            }
        } else {
            std::fs::read(&path).ok()?
        };
        let natural_size = if extension == "pdf" || extension == "svg" {
            svg_dimensions(&data)
        } else {
            None
        };
        let (w_px, h_px) = image_size(&data)
            .or_else(|| {
                natural_size.map(|(w, h)| {
                    (
                        (w * 96.0 / 72.0).round() as u32,
                        (h * 96.0 / 72.0).round() as u32,
                    )
                })
            })
            .unwrap_or((600, 400));
        Some(Image {
            path: path.to_string_lossy().to_string(),
            data,
            width_px: w_px,
            height_px: h_px,
            width_pt: width
                .or_else(|| natural_size.map(|s| s.0))
                .map(|w| w * scale),
            height_pt: height
                .or_else(|| natural_size.map(|s| s.1))
                .map(|h| h * scale),
            alt: name,
        })
    }

    // -----------------------------------------------------------------------
    // Floats, tables, environments
    // -----------------------------------------------------------------------

    fn environment(&mut self, env: String) {
        let starred = env.ends_with('*');
        let base = env.trim_end_matches('*').to_string();
        match base.as_str() {
            "document" => {
                self.flush_para();
                self.in_preamble = false;
                self.st.in_preamble = false;
            }
            "abstract" => {
                self.flush_para();
                let title = if self.st.chinese {
                    "摘要".to_string()
                } else {
                    "Abstract".to_string()
                };
                self.blocks.push(Block::Heading {
                    level: 1,
                    number: None,
                    title: vec![Inline::Text(title)],
                    anchor: None,
                    kind: HeadingKind::Other,
                });
                self.parse_env_body_blocks(&env);
            }
            "itemize" => self.parse_list(ListKind::Bullet, &env),
            "enumerate" => self.parse_list(ListKind::Number, &env),
            "description" => self.parse_deflist(&env),
            "quote" | "quotation" | "verse" => {
                self.flush_para();
                let inner = self.parse_env_body_blocks(&env);
                self.blocks.push(Block::Quote(inner));
            }
            "center" => {
                self.flush_para();
                let inner = self.parse_env_body_blocks(&env);
                self.blocks.push(Block::AlignBlock(Align::Center, inner));
            }
            "flushleft" => {
                self.flush_para();
                let inner = self.parse_env_body_blocks(&env);
                self.blocks.push(Block::AlignBlock(Align::Left, inner));
            }
            "flushright" => {
                self.flush_para();
                let inner = self.parse_env_body_blocks(&env);
                self.blocks.push(Block::AlignBlock(Align::Right, inner));
            }
            "verbatim" | "lstlisting" => {
                self.flush_para();
                // tokens come from engine markers
                let lines = self.collect_verb_lines();
                self.blocks.push(Block::Verbatim {
                    lines,
                    style: base.clone(),
                });
            }
            "equation" | "displaymath" => {
                self.flush_para();
                let toks = self.collect_until_end(&env);
                self.display_math_tokens(toks, !starred, &env);
            }
            "align" | "gather" | "multline" | "alignat" => {
                self.flush_para();
                self.display_math_env(&env, starred);
            }
            "split" | "aligned" | "gathered" | "math" | "eqnarray" | "cases" => {
                self.flush_para();
                let toks = self.collect_until_end(&env);
                self.display_math_tokens(toks, false, &env);
            }
            "subequations" => {
                self.flush_para();
                let parent = self.st.the_counter("equation");
                self.st.subeq_active = Some((parent, 0));
                self.parse_env_body_blocks(&env);
                self.st.subeq_active = None;
            }
            "array" | "tabular" | "tabularx" | "tabular*" => {
                self.flush_para();
                self.parse_table(&env);
            }
            "figure" => self.parse_float(&env, "figure"),
            "table" => self.parse_float(&env, "table"),
            "thebibliography" => {
                self.flush_para();
                self.parse_thebibliography(&env);
            }
            "minipage" => {
                self.flush_para();
                if let Some(w) = self.cur.read_optional() {
                    let _ = w;
                }
                let _ = self.cur.read_group();
                let inner = self.parse_env_body_blocks(&env);
                self.blocks.extend(inner);
            }
            "titlepage" => {
                let inner = self.parse_env_body_blocks(&env);
                self.blocks.extend(inner);
            }
            _ => {
                if self.st.theorems.contains_key(&base) {
                    self.flush_para();
                    self.parse_theorem(&base, &env);
                } else if self.st.user_envs.contains_key(&base) {
                    self.parse_user_env(&base, &env);
                } else {
                    self.st
                        .warn(format!("unknown environment `{}`; contents kept", env));
                    self.flush_para();
                    let inner = self.parse_env_body_blocks(&env);
                    self.blocks.extend(inner);
                }
            }
        }
    }

    /// Parse blocks until `\end{env}`.
    fn parse_env_body_blocks(&mut self, env: &str) -> Vec<Block> {
        let saved_blocks = std::mem::take(&mut self.blocks);
        let saved_para = std::mem::take(&mut self.para);
        loop {
            self.skip_junk();
            if self.cur.at_end() || self.peek_is_end(env) {
                break;
            }
            if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "end") {
                break;
            }
            self.parse_block();
            // paragraph flush on par boundaries happens at block starts
            if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "par") {
                self.flush_para();
            }
        }
        self.flush_para();
        // consume \end{env}
        if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "end") {
            self.cur.pos += 1;
            let _ = self.cur.read_group();
        }
        let inner = std::mem::replace(&mut self.blocks, saved_blocks);
        self.para = saved_para;
        inner
    }

    fn parse_list(&mut self, kind: ListKind, env: &str) {
        self.flush_para();
        self.list_depth += 1;
        let mut items = vec![];
        loop {
            self.skip_junk();
            if self.cur.at_end() || self.peek_is_end(env) {
                break;
            }
            // expect \item
            if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "item") {
                self.cur.pos += 1;
                let label = self.cur.read_optional();
                let label_inls = label.map(|l| self.parse_inlines_toks(l, &mut Style::default()));
                let blocks = self.parse_env_body_list_item(env);
                items.push(ListItem {
                    label: label_inls,
                    blocks,
                });
            } else if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "end") {
                break;
            } else {
                // stray content: treat as loose paragraph in list
                self.parse_block();
            }
        }
        if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "end") {
            self.cur.pos += 1;
            let _ = self.cur.read_group();
        }
        self.list_depth -= 1;
        self.blocks.push(Block::List { kind, items });
    }

    /// Parse blocks for a list item: until \item or \end (without consuming).
    fn parse_env_body_list_item(&mut self, env: &str) -> Vec<Block> {
        let saved_blocks = std::mem::take(&mut self.blocks);
        let saved_para = std::mem::take(&mut self.para);
        loop {
            self.skip_junk();
            if self.cur.at_end() {
                break;
            }
            if self.peek_is_end(env) {
                break;
            }
            if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "item" || n == "end") {
                break;
            }
            self.parse_block();
            if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "par") {
                self.flush_para();
            }
        }
        self.flush_para();
        let inner = std::mem::replace(&mut self.blocks, saved_blocks);
        self.para = saved_para;
        inner
    }

    fn parse_deflist(&mut self, env: &str) {
        self.flush_para();
        let mut items = vec![];
        loop {
            self.skip_junk();
            if self.cur.at_end() || self.peek_is_end(env) {
                break;
            }
            if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "item") {
                self.cur.pos += 1;
                let label = self.cur.read_optional().unwrap_or_default();
                let label_inls = self.parse_inlines_toks(label, &mut Style::default());
                let blocks = self.parse_env_body_list_item(env);
                items.push((label_inls, blocks));
            } else if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "end") {
                break;
            } else {
                self.parse_block();
            }
        }
        if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "end") {
            self.cur.pos += 1;
            let _ = self.cur.read_group();
        }
        self.blocks.push(Block::DefinitionList(items));
    }

    fn parse_float(&mut self, env: &str, kind: &str) {
        self.flush_para();
        let _ = self.cur.read_optional();
        let saved_blocks = std::mem::take(&mut self.blocks);
        let saved_para = std::mem::take(&mut self.para);
        let mut caption: Option<Caption> = None;
        let mut anchor: Option<String> = None;
        loop {
            self.skip_junk();
            if self.cur.at_end() || self.peek_is_end(env) {
                break;
            }
            match self.cur.peek().cloned() {
                Some(Token::CtrlSeq(n)) if n == "caption" || n == "captionof" => {
                    self.cur.pos += 1;
                    let mut of_kind = kind.to_string();
                    if n == "captionof" {
                        of_kind = detokenize(&self.cur.read_group());
                    }
                    let _short = self.cur.read_optional();
                    let g = self.cur.read_group();
                    self.st.refstep(&if of_kind == "table" {
                        "table"
                    } else {
                        "figure"
                    });
                    let num = self.st.the_counter(&if of_kind == "table" {
                        "table"
                    } else {
                        "figure"
                    });
                    let content = self.parse_inlines_toks(g, &mut Style::default());
                    let plain = inlines_to_text(&content);
                    self.st.current_ref = Some(RefTarget {
                        counter: if of_kind == "table" {
                            "table".into()
                        } else {
                            "figure".into()
                        },
                        number: num.clone(),
                        title: Some(plain),
                    });
                    // absorb following \label
                    let save = self.cur.pos;
                    self.cur.skip_spaces();
                    if matches!(self.cur.peek(), Some(Token::CtrlSeq(l)) if l == "label") {
                        self.cur.pos += 1;
                        let lg = self.cur.read_group();
                        let key = detokenize(&lg).trim().to_string();
                        anchor = Some(bib::sanitize_anchor(&key));
                        self.register_label(&key);
                    } else {
                        self.cur.pos = save;
                    }
                    caption = Some(Caption {
                        number: Some(num),
                        content,
                        label: anchor.clone(),
                    });
                }
                Some(Token::CtrlSeq(n)) if n == "label" => {
                    self.cur.pos += 1;
                    let g = self.cur.read_group();
                    let key = detokenize(&g).trim().to_string();
                    anchor = Some(bib::sanitize_anchor(&key));
                    self.register_label(&key);
                }
                Some(Token::CtrlSeq(n)) if n == "end" => break,
                _ => {
                    self.parse_block();
                    if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "par") {
                        self.flush_para();
                    }
                }
            }
        }
        self.flush_para();
        if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "end") {
            self.cur.pos += 1;
            let _ = self.cur.read_group();
        }
        let inner = std::mem::replace(&mut self.blocks, saved_blocks);
        self.para = saved_para;
        // caption block embedded: if no caption but anchor exists, attach anyway
        self.blocks.push(Block::Figure {
            content: inner,
            caption,
            anchor,
            kind: kind.to_string(),
        });
    }

    fn parse_theorem(&mut self, name: &str, env: &str) {
        let (thmname, within) = self.st.theorems.get(name).cloned().unwrap_or_default();
        let _ = within;
        let number = if self.st.counters.contains_key(name) {
            self.st.refstep(name);
            Some(self.st.the_counter(name))
        } else {
            None
        };
        self.st.current_ref = Some(RefTarget {
            counter: name.to_string(),
            number: number.clone().unwrap_or_default(),
            title: None,
        });
        let inner = self.parse_env_body_blocks(env);
        // absorb label right after begin (common: \begin{thm}\label{x})
        let mut anchor = None;
        // (labels already registered via inline \label handler using current_ref)
        let _ = &anchor;
        // find anchor: labels registered this pass for this counter most recent
        if let Some((k, _, _, _)) = self.labels.last() {
            anchor = Some(bib::sanitize_anchor(k));
        }
        self.blocks.push(Block::Theorem {
            name: thmname,
            number,
            content: inner,
            anchor,
        });
    }

    fn parse_user_env(&mut self, name: &str, env: &str) {
        let (nargs, optdef, begin_toks, end_toks) = match self.st.user_envs.get(name) {
            Some(v) => v.clone(),
            None => return,
        };
        // read args
        let mut args: Vec<Vec<Token>> = vec![];
        for i in 0..nargs {
            if i == 0 {
                if let Some(d) = &optdef {
                    if let Some(o) = self.cur.read_optional() {
                        args.push(o);
                        continue;
                    }
                    args.push(d.clone());
                    continue;
                }
            }
            args.push(self.cur.read_group());
        }
        // expand begin tokens with args substituted
        let mut toks = begin_toks.clone();
        substitute_args(&mut toks, &args);
        let expanded = {
            let mut ex = Expander::new(self.st);
            ex.expand_all(toks)
        };
        // splice into stream
        let mut new_toks: Vec<Token> = vec![];
        new_toks.extend(self.cur.toks[..self.cur.pos].iter().cloned());
        new_toks.extend(expanded);
        new_toks.extend(self.cur.toks[self.cur.pos..].iter().cloned());
        self.cur.toks = new_toks;
        // parse body
        let inner = self.parse_env_body_blocks(env);
        self.blocks.extend(inner);
        // end tokens
        let mut etoks = end_toks.clone();
        substitute_args(&mut etoks, &args);
        let expanded = {
            let mut ex = Expander::new(self.st);
            ex.expand_all(etoks)
        };
        let mut new_toks: Vec<Token> = vec![];
        new_toks.extend(self.cur.toks[..self.cur.pos].iter().cloned());
        new_toks.extend(expanded);
        new_toks.extend(self.cur.toks[self.cur.pos..].iter().cloned());
        self.cur.toks = new_toks;
    }

    fn parse_thebibliography(&mut self, env: &str) {
        // width arg
        let _ = self.cur.read_group();
        let mut items: Vec<BibItem> = vec![];
        let mut n = 0;
        loop {
            self.skip_junk();
            if self.cur.at_end() || self.peek_is_end(env) {
                break;
            }
            match self.cur.peek().cloned() {
                Some(Token::CtrlSeq(c)) if c == "bibitem" => {
                    self.cur.pos += 1;
                    n += 1;
                    let label = self.cur.read_optional();
                    let key = detokenize(&self.cur.read_group()).trim().to_string();
                    let blocks = self.parse_env_body_list_item(env);
                    let content = blocks
                        .iter()
                        .map(|b| match b {
                            Block::Para { inlines, .. } => inlines.clone(),
                            other => vec![Inline::Text(format!("{:?}", other))],
                        })
                        .flatten()
                        .collect::<Vec<_>>();
                    let lab = label
                        .map(|l| detokenize(&l))
                        .unwrap_or_else(|| format!("[{}]", n));
                    // register so \cite works
                    self.bib.items.push(bib::BibItemFull {
                        key: key.clone(),
                        label: lab.clone(),
                        number: n,
                        content: content.clone(),
                        anchor: format!("bibitem-{}", bib::sanitize_anchor(&key)),
                    });
                    items.push(BibItem {
                        key,
                        label: lab,
                        content,
                    });
                }
                Some(Token::CtrlSeq(c)) if c == "end" => break,
                _ => {
                    // stray text between items: append to previous item
                    self.parse_block();
                }
            }
        }
        if matches!(self.cur.peek(), Some(Token::CtrlSeq(n)) if n == "end") {
            self.cur.pos += 1;
            let _ = self.cur.read_group();
        }
        let title = if self.st.chinese {
            "参考文献".into()
        } else {
            "References".into()
        };
        self.blocks.push(Block::BibSection { title, items });
    }

    // -----------------------------------------------------------------------
    // tabular
    // -----------------------------------------------------------------------

    fn parse_table(&mut self, env: &str) {
        // optional width for tabular*
        let _ = self.cur.read_optional();
        let spec_toks = self.cur.read_group();
        let spec = detokenize(&spec_toks);
        let mut aligns: Vec<Align> = vec![];
        let mut widths: Vec<Option<f64>> = vec![];
        let mut borders = false;
        {
            let mut chars = spec.chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    'l' => {
                        aligns.push(Align::Left);
                        widths.push(None);
                    }
                    'c' => {
                        aligns.push(Align::Center);
                        widths.push(None);
                    }
                    'r' => {
                        aligns.push(Align::Right);
                        widths.push(None);
                    }
                    'p' | 'm' | 'b' => {
                        // p{width}
                        while matches!(chars.peek(), Some(' ')) {
                            chars.next();
                        }
                        if chars.peek() == Some(&'{') {
                            chars.next();
                            let mut w = String::new();
                            for d in chars.by_ref() {
                                if d == '}' {
                                    break;
                                }
                                w.push(d);
                            }
                            aligns.push(Align::Left);
                            let mut cc = Cursor::new(lex(&w, &self.st.catcodes));
                            widths.push(Some(scan_dimen(self.st, &mut cc)));
                        }
                    }
                    '|' => borders = true,
                    '>' | '<' | '@' => {
                        // skip {...}
                        while matches!(chars.peek(), Some(' ')) {
                            chars.next();
                        }
                        if chars.peek() == Some(&'{') {
                            chars.next();
                            let mut depth = 1;
                            for d in chars.by_ref() {
                                if d == '{' {
                                    depth += 1;
                                } else if d == '}' {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    '*' => {
                        // *{n}{spec} — naive: skip
                        while matches!(chars.peek(), Some(' ')) {
                            chars.next();
                        }
                        if chars.peek() == Some(&'{') {
                            chars.next();
                            for d in chars.by_ref() {
                                if d == '}' {
                                    break;
                                }
                            }
                        }
                        while matches!(chars.peek(), Some(' ')) {
                            chars.next();
                        }
                        if chars.peek() == Some(&'{') {
                            chars.next();
                            let mut depth = 1;
                            for d in chars.by_ref() {
                                if d == '{' {
                                    depth += 1;
                                } else if d == '}' {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let ncol = aligns.len().max(1);

        let mut rows: Vec<TableRow> = vec![];
        let mut pending_top: RuleKind = RuleKind::None;
        let mut cur_cells: Vec<TableCell> = vec![];
        let mut cur_align_idx = 0usize;
        let mut row_rule_bottom: RuleKind = RuleKind::None;

        let mut cell_toks: Vec<Token> = vec![];
        let mut cell_depth = 0i32;
        let mut cell_colspan = 1usize;
        let mut cell_rowspan = 1usize;
        let mut cell_align_override: Option<Align> = None;

        macro_rules! finish_cell {
            () => {{
                let align = cell_align_override
                    .take()
                    .unwrap_or_else(|| aligns.get(cur_align_idx).copied().unwrap_or(Align::Left));
                let inlines = {
                    let saved = std::mem::replace(
                        &mut self.cur,
                        Cursor::new(std::mem::take(&mut cell_toks)),
                    );
                    let v = self.parse_inlines(&mut Style::default());
                    self.cur = saved;
                    v
                };
                cur_cells.push(TableCell {
                    blocks: vec![Block::Para {
                        inlines,
                        align: None,
                        indent: false,
                    }],
                    align,
                    colspan: cell_colspan,
                    rowspan: cell_rowspan,
                    header: false,
                });
                cell_colspan = 1;
                cell_rowspan = 1;
                cur_align_idx += 1;
            }};
        }
        macro_rules! finish_row {
            () => {{
                if !cur_cells.is_empty() {
                    // pad with empty cells
                    while cur_cells.len() < ncol {
                        cur_cells.push(TableCell {
                            blocks: vec![],
                            align: Align::Left,
                            colspan: 1,
                            rowspan: 1,
                            header: false,
                        });
                        cur_align_idx += 1;
                    }
                    rows.push(TableRow {
                        cells: std::mem::take(&mut cur_cells),
                        rule_top: std::mem::replace(&mut pending_top, RuleKind::None),
                        rule_bottom: std::mem::replace(&mut row_rule_bottom, RuleKind::None),
                    });
                    cur_align_idx = 0;
                } else if pending_top != RuleKind::None {
                    // rule with no row yet: attach to next row (keep pending)
                }
            }};
        }

        loop {
            let t = match self.cur.next() {
                Some(t) => t,
                None => break,
            };
            match &t {
                Token::CtrlSeq(n) if n == "end" && cell_depth == 0 => {
                    let g = self.cur.read_group();
                    if detokenize(&g).trim_end_matches('*') == env.trim_end_matches('*') {
                        break;
                    }
                    cell_toks.push(Token::cs("end"));
                    cell_toks.extend(g);
                }
                Token::CtrlSeq(n)
                    if (n == "\\" || n == "newline" || n == "tabularnewline")
                        && cell_depth == 0 =>
                {
                    finish_cell!();
                    finish_row!();
                }
                Token::Char('&', CatCode::Alignment) if cell_depth == 0 => {
                    finish_cell!();
                }
                Token::Char('{', CatCode::BGroup) => {
                    cell_depth += 1;
                    cell_toks.push(t);
                }
                Token::Char('}', CatCode::EGroup) => {
                    cell_depth -= 1;
                    cell_toks.push(t);
                }
                Token::CtrlSeq(n) if n == "hline" && cell_depth == 0 => {
                    if cur_cells.is_empty() && rows.is_empty() {
                        pending_top = RuleKind::Thin;
                    } else if cur_cells.is_empty() {
                        pending_top = RuleKind::Thin;
                    } else {
                        row_rule_bottom = RuleKind::Thin;
                    }
                }
                Token::CtrlSeq(n)
                    if (n == "toprule" || n == "midrule" || n == "bottomrule")
                        && cell_depth == 0 =>
                {
                    if cur_cells.is_empty() {
                        pending_top = RuleKind::Thick;
                    } else {
                        row_rule_bottom = RuleKind::Thick;
                    }
                }
                Token::CtrlSeq(n) if n == "cmidrule" && cell_depth == 0 => {
                    let _ = self.cur.read_optional();
                    let _ = self.cur.read_group();
                }
                Token::CtrlSeq(n) if n == "cline" && cell_depth == 0 => {
                    let _ = self.cur.read_group();
                }
                Token::CtrlSeq(n) if n == "multicolumn" && cell_depth == 0 => {
                    let num = detokenize(&self.cur.read_group());
                    cell_colspan = num.trim().parse().unwrap_or(1);
                    let aspec = detokenize(&self.cur.read_group());
                    cell_align_override = aspec.chars().find_map(|c| match c {
                        'l' => Some(Align::Left),
                        'c' => Some(Align::Center),
                        'r' => Some(Align::Right),
                        _ => None,
                    });
                    let g = self.cur.read_group();
                    cell_toks.extend(g);
                }
                Token::CtrlSeq(n) if n == "multirow" && cell_depth == 0 => {
                    let num = detokenize(&self.cur.read_group());
                    cell_rowspan = num.trim().parse().unwrap_or(1);
                    let _ = self.cur.read_group(); // width
                    let g = self.cur.read_group();
                    cell_toks.extend(g);
                }
                Token::CtrlSeq(n) if n == "par" => {
                    cell_toks.push(Token::Space);
                }
                other => cell_toks.push(other.clone()),
            }
        }
        if !cur_cells.is_empty() || cell_toks.iter().any(|t| !matches!(t, Token::Space)) {
            finish_cell!();
            finish_row!();
        } else if pending_top != RuleKind::None {
            if let Some(row) = rows.last_mut() {
                row.rule_bottom = pending_top;
            }
        }

        let caption = None;
        let mut table = Table {
            rows,
            aligns: aligns.clone(),
            width_pt: None,
            borders,
            caption,
        };
        // column widths
        if widths.iter().any(|w| w.is_some()) {
            table.width_pt = Some(widths.iter().flatten().sum());
        }
        let _ = ncol;
        self.blocks.push(Block::TableBlock(table));
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn wrap_style(style: &Style, inline: Inline) -> Inline {
    let mut cur = inline;
    if style.bold && style.italic {
        cur = Inline::BoldItalic(vec![cur]);
    } else if style.bold {
        cur = Inline::Bold(vec![cur]);
    } else if style.italic {
        cur = Inline::Italic(vec![cur]);
    }
    if style.underline {
        cur = Inline::Underline(vec![cur]);
    }
    if style.strike {
        cur = Inline::Strike(vec![cur]);
    }
    if style.smallcaps {
        cur = Inline::SmallCaps(vec![cur]);
    }
    if style.slanted {
        cur = Inline::Slanted(vec![cur]);
    }
    if style.mono {
        cur = Inline::Family(FontFamily::Tt, vec![cur]);
    } else if style.sans {
        cur = Inline::Family(FontFamily::Sf, vec![cur]);
    }
    if let Some(sz) = style.size {
        cur = Inline::FontSize(sz, vec![cur]);
    }
    if let Some(c) = &style.color {
        cur = Inline::Color {
            color: c.clone(),
            content: vec![cur],
        };
    }
    cur
}

fn fixup_text(inlines: &mut [Inline]) {
    for i in inlines {
        match i {
            Inline::Text(t) => {
                *t = t
                    .replace("``", "“")
                    .replace("''", "”")
                    .replace("`", "‘")
                    .replace("'", "’")
                    .replace("---", "—")
                    .replace("--", "–");
            }
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
            | Inline::FontSize(_, v) => fixup_text(v),
            Inline::Color { content, .. } | Inline::Highlight { content, .. } => {
                fixup_text(content)
            }
            Inline::Link { content, .. } => fixup_text(content),
            Inline::Footnote(b) => fixup_blocks(std::slice::from_mut(&mut (**b))),
            _ => {}
        }
    }
}

fn fixup_blocks(blocks: &mut [Block]) {
    for b in blocks {
        match b {
            Block::Para { inlines, .. } => fixup_text(inlines),
            Block::Heading { title, .. } => fixup_text(title),
            Block::List { items, .. } => {
                for it in items {
                    if let Some(l) = &mut it.label {
                        fixup_text(l);
                    }
                    fixup_blocks(&mut it.blocks);
                }
            }
            Block::DefinitionList(items) => {
                for (t, bs) in items {
                    fixup_text(t);
                    fixup_blocks(bs);
                }
            }
            Block::Quote(bs) | Block::AlignBlock(_, bs) => fixup_blocks(bs),
            Block::Figure {
                content, caption, ..
            } => {
                fixup_blocks(content);
                if let Some(c) = caption {
                    fixup_text(&mut c.content);
                }
            }
            Block::Theorem { content, .. } => fixup_blocks(content),
            Block::TableBlock(t) => {
                for r in &mut t.rows {
                    for c in &mut r.cells {
                        fixup_blocks(&mut c.blocks);
                    }
                }
            }
            Block::BibSection { items, .. } => {
                for it in items {
                    fixup_text(&mut it.content);
                }
            }
            _ => {}
        }
    }
}

/// color name or mix to hex
pub fn color_to_rgb(c: &str) -> String {
    let c = c.trim();
    if c.starts_with('#') {
        return c.trim_start_matches('#').to_string();
    }
    let named = |n: &str| -> Option<&'static str> {
        Some(match n {
            "black" => "000000",
            "white" => "FFFFFF",
            "red" => "FF0000",
            "green" => "008000",
            "blue" => "0000FF",
            "cyan" => "00FFFF",
            "magenta" => "FF00FF",
            "yellow" => "FFFF00",
            "gray" => "808080",
            "grey" => "808080",
            "darkgray" | "darkgrey" => "404040",
            "lightgray" | "lightgrey" => "C0C0C0",
            "brown" => "A52A2A",
            "orange" => "FFA500",
            "purple" => "800080",
            "violet" => "EE82EE",
            "pink" => "FFC0CB",
            "teal" => "008080",
            "olive" => "808000",
            "navy" => "000080",
            "maroon" => "800000",
            "lime" => "00FF00",
            _ => return None,
        })
    };
    // xcolor mix: "red!30!blue" or "red!30"
    if c.contains('!') {
        let parts: Vec<&str> = c.split('!').collect();
        if let Some(base) = named(parts[0]) {
            return base.to_string();
        }
        return "000000".to_string();
    }
    named(c).unwrap_or("000000").to_string()
}

fn strip_outer_group(value: &str) -> String {
    let value = value.trim();
    if value.starts_with('{') && value.ends_with('}') {
        value[1..value.len() - 1].trim().to_string()
    } else {
        value.to_string()
    }
}

fn split_keyvals(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut depth = 0;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '{' => {
                depth += 1;
                cur.push(c);
            }
            '}' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            c => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn parse_len_frac(v: &str, textwidth: f64) -> f64 {
    let v = v.trim();
    for suffix in ["\\textwidth", "\\linewidth", "\\columnwidth"] {
        if let Some(num) = v.strip_suffix(suffix) {
            let f: f64 = num.trim().parse().unwrap_or(1.0);
            return f * textwidth;
        }
    }
    let mut cur = Cursor::new(lex(v, &default_catcodes()));
    scan_dimen(&State::new(), &mut cur)
}

fn pdf_to_svg(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let output =
        std::env::temp_dir().join(format!("doclatex-{}-{}.svg", std::process::id(), unique));
    let cairo = std::process::Command::new("pdftocairo")
        .arg("-svg")
        .arg(path)
        .arg(&output)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let success = match cairo {
        Ok(status) if status.success() => true,
        _ => std::process::Command::new("mutool")
            .args(["draw", "-q", "-F", "svg", "-o"])
            .arg(&output)
            .arg(path)
            .arg("1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
    };
    if !success {
        let _ = std::fs::remove_file(&output);
        return Err("neither pdftocairo nor mutool could convert the first page".into());
    }
    let data = std::fs::read(&output).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&output);
    data
}

fn svg_dimensions(data: &[u8]) -> Option<(f64, f64)> {
    let text = std::str::from_utf8(data).ok()?;
    let start = text.find("<svg")?;
    let end = text[start..].find('>')? + start;
    let tag = &text[start..end];
    let width = svg_attribute(tag, "width").and_then(parse_svg_length);
    let height = svg_attribute(tag, "height").and_then(parse_svg_length);
    if let (Some(width), Some(height)) = (width, height) {
        return Some((width, height));
    }
    let view_box = svg_attribute(tag, "viewBox")?;
    let values: Vec<f64> = view_box
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter_map(|v| v.parse().ok())
        .collect();
    if values.len() == 4 && values[2] > 0.0 && values[3] > 0.0 {
        Some((values[2] * 72.0 / 96.0, values[3] * 72.0 / 96.0))
    } else {
        None
    }
}

fn svg_attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut search = tag;
    while let Some(index) = search.find(name) {
        let after = &search[index + name.len()..];
        let before_ok = index == 0
            || !search[..index]
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '-');
        if before_ok {
            let after = after.trim_start();
            if let Some(after) = after.strip_prefix('=') {
                let after = after.trim_start();
                let quote = after.chars().next()?;
                if quote == '\'' || quote == '"' {
                    let value = &after[1..];
                    let end = value.find(quote)?;
                    return Some(&value[..end]);
                }
            }
        }
        search = after;
    }
    None
}

fn parse_svg_length(value: &str) -> Option<f64> {
    let value = value.trim();
    let split = value
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E')))
        .unwrap_or(value.len());
    let number: f64 = value[..split].parse().ok()?;
    let points = match value[split..].trim() {
        "pt" => number,
        "in" => number * 72.0,
        "cm" => number * 72.0 / 2.54,
        "mm" => number * 72.0 / 25.4,
        _ => number * 72.0 / 96.0,
    };
    Some(points)
}

fn image_size(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() >= 24 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
        let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        return Some((w, h));
    }
    if data.len() > 4 && &data[0..3] == b"\xff\xd8\xff" {
        // JPEG: scan markers
        let mut i = 2;
        while i + 9 < data.len() {
            if data[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = data[i + 1];
            if marker == 0xC0 || marker == 0xC1 || marker == 0xC2 {
                let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
                let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
                return Some((w, h));
            }
            let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            i += 2 + len;
        }
    }
    None
}

fn substitute_args(toks: &mut Vec<Token>, args: &[Vec<Token>]) {
    let mut out = vec![];
    let mut i = 0;
    while i < toks.len() {
        if matches!(toks[i], Token::Char('#', CatCode::Parameter)) {
            if let Some(Token::Char(d, _)) = toks.get(i + 1) {
                if d.is_ascii_digit() {
                    let n = (*d as u8 - b'0') as usize;
                    if n >= 1 && n <= args.len() {
                        out.extend(args[n - 1].iter().cloned());
                    }
                    i += 2;
                    continue;
                }
            }
        }
        out.push(toks[i].clone());
        i += 1;
    }
    *toks = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_hline_does_not_create_empty_table_row() {
        let mut st = State::new();
        let tokens = lex(
            r"\begin{tabular}{lcr}\hline Left & Center & Right\\\hline A & B & C\\1 & 2 & 3\\\hline\end{tabular}",
            &default_catcodes(),
        );
        let result = parse_document(&mut st, tokens, &AuxData::default(), &mut BibState::new());
        let table = result
            .doc
            .blocks
            .iter()
            .find_map(|block| match block {
                Block::TableBlock(table) => Some(table),
                _ => None,
            })
            .unwrap();
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[0].rule_top, RuleKind::Thin);
        assert_eq!(table.rows[1].rule_top, RuleKind::Thin);
        assert_eq!(table.rows[2].rule_bottom, RuleKind::Thin);
    }

    #[test]
    fn preamble_and_body_whitespace_do_not_create_paragraphs() {
        let mut st = State::new();
        let tokens = lex(
            "\\title{Demo}\n\n\\author{Author}\n\n\\begin{document}\n\n\\maketitle\n\nBody text.\n\n   \n\\end{document}",
            &default_catcodes(),
        );
        let result = parse_document(&mut st, tokens, &AuxData::default(), &mut BibState::new());
        assert_eq!(result.doc.blocks.len(), 2);
        assert!(matches!(
            result.doc.blocks.first(),
            Some(Block::TitlePage { .. })
        ));
        assert!(
            matches!(result.doc.blocks.last(), Some(Block::Para { inlines, .. }) if has_visible_content(inlines))
        );
    }
}
