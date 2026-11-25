use crate::model::*;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const XML_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";
const REL_OFFICE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const REL_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const REL_HYPERLINK: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
const MATH_TABS: &str = "<w:tabs><w:tab w:val=\"center\" w:pos=\"4513\"/><w:tab w:val=\"right\" w:pos=\"9026\"/></w:tabs>";

#[derive(Clone, Default)]
struct RunStyle {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    caps: bool,
    vertical: Option<&'static str>,
    family: Option<FontFamily>,
    size: Option<FontSize>,
    color: Option<String>,
    highlight: Option<String>,
    hyperlink: bool,
}

struct Relationship {
    id: String,
    kind: String,
    target: String,
    external: bool,
}

struct ImagePart {
    name: String,
    content_type: &'static str,
    data: Vec<u8>,
}

struct Renderer {
    xml: String,
    relationships: Vec<Relationship>,
    images: Vec<ImagePart>,
    footnote_sources: Vec<Block>,
    next_source_footnote: usize,
    footnotes: Vec<Block>,
    bookmark_id: u32,
    drawing_id: u32,
    list_depth: usize,
    last_was_paragraph: bool,
}

pub fn write_docx(path: &Path, doc: &Document) -> Result<(), Box<dyn std::error::Error>> {
    let mut renderer = Renderer {
        xml: String::new(),
        relationships: Vec::new(),
        images: Vec::new(),
        footnote_sources: doc.footnotes.clone(),
        next_source_footnote: 0,
        footnotes: Vec::new(),
        bookmark_id: 1,
        drawing_id: 1,
        list_depth: 0,
        last_was_paragraph: false,
    };
    renderer.blocks(&doc.blocks, None);
    while renderer.next_source_footnote < renderer.footnote_sources.len() {
        let block = renderer.footnote_sources[renderer.next_source_footnote].clone();
        renderer.next_source_footnote += 1;
        let id = renderer.footnotes.len() + 1;
        renderer.footnotes.push(block);
        renderer.xml.push_str(&format!(
            "<w:p><w:r><w:rPr><w:rStyle w:val=\"FootnoteReference\"/></w:rPr><w:footnoteReference w:id=\"{}\"/></w:r></w:p>",
            id
        ));
    }

    let document = format!(
        "{}<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\" xmlns:asvg=\"http://schemas.microsoft.com/office/drawing/2016/SVG/main\"><w:body>{}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>",
        XML_DECL, renderer.xml
    );
    let document_rels = renderer.document_relationships();
    let footnotes = renderer.footnotes_xml();
    let images = renderer.images;

    let file = File::create(path)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut part = |name: &str, data: &[u8]| -> Result<(), Box<dyn std::error::Error>> {
        zip.start_file(name, options)?;
        zip.write_all(data)?;
        Ok(())
    };
    part("[Content_Types].xml", content_types(&images).as_bytes())?;
    part("_rels/.rels", root_relationships().as_bytes())?;
    part("word/document.xml", document.as_bytes())?;
    part("word/_rels/document.xml.rels", document_rels.as_bytes())?;
    part("word/styles.xml", styles().as_bytes())?;
    part("word/numbering.xml", numbering().as_bytes())?;
    part("word/footnotes.xml", footnotes.as_bytes())?;
    part("word/settings.xml", settings().as_bytes())?;
    part("docProps/core.xml", core_properties().as_bytes())?;
    part("docProps/app.xml", app_properties().as_bytes())?;
    for image in &images {
        part(&format!("word/media/{}", image.name), &image.data)?;
    }
    drop(part);
    zip.finish()?;
    Ok(())
}

impl Renderer {
    fn blocks(&mut self, blocks: &[Block], forced_align: Option<Align>) {
        for block in blocks {
            self.block(block, forced_align);
        }
    }

    fn block(&mut self, block: &Block, forced_align: Option<Align>) {
        if !matches!(block, Block::Para { .. } | Block::DisplayMath { .. }) {
            self.last_was_paragraph = false;
        }
        match block {
            Block::Heading { level, number, title, anchor, .. } => {
                self.xml.push_str(&format!("<w:p><w:pPr><w:pStyle w:val=\"Heading{}\"/>", level));
                if number.is_some() {
                    self.xml.push_str(&format!("<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"19\"/></w:numPr>", level.saturating_sub(1)));
                }
                self.xml.push_str("</w:pPr>");
                if let Some(anchor) = anchor {
                    self.bookmark_start(anchor);
                }
                self.inlines(title, &RunStyle::default());
                if anchor.is_some() {
                    self.bookmark_end();
                }
                self.xml.push_str("</w:p>");
            }
            Block::Para { inlines, align, indent } => {
                self.paragraph(inlines, None, forced_align.or(*align), *indent);
            }
            Block::List { kind, items } => {
                let depth = self.list_depth;
                for item in items {
                    if let Some(label) = &item.label {
                        self.xml.push_str("<w:p><w:pPr><w:pStyle w:val=\"ListParagraph\"/></w:pPr>");
                        self.inlines(label, &RunStyle { bold: true, ..RunStyle::default() });
                        self.text_run(" ", &RunStyle::default());
                        if let Some(first) = item.blocks.first() {
                            if let Block::Para { inlines, .. } = first {
                                self.inlines(inlines, &RunStyle::default());
                            }
                        }
                        self.xml.push_str("</w:p>");
                        self.list_depth += 1;
                        self.blocks(item.blocks.get(1..).unwrap_or(&[]), None);
                        self.list_depth -= 1;
                    } else {
                        let num_id = depth.min(8) * 2 + if *kind == ListKind::Bullet { 1 } else { 2 };
                        let mut first_para = true;
                        for child in &item.blocks {
                            if let Block::Para { inlines, align, .. } = child {
                                self.xml.push_str(&format!(
                                    "<w:p><w:pPr><w:pStyle w:val=\"ListParagraph\"/><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"{}\"/></w:numPr></w:pPr>",
                                    num_id
                                ));
                                self.inlines(inlines, &RunStyle::default());
                                let _ = align;
                                self.xml.push_str("</w:p>");
                                first_para = false;
                            } else {
                                self.list_depth += 1;
                                self.block(child, None);
                                self.list_depth -= 1;
                            }
                        }
                        if first_para {
                            self.xml.push_str(&format!("<w:p><w:pPr><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"{}\"/></w:numPr></w:pPr></w:p>", num_id));
                        }
                    }
                }
            }
            Block::DefinitionList(items) => {
                for (term, content) in items {
                    self.xml.push_str("<w:p><w:pPr><w:pStyle w:val=\"ListParagraph\"/></w:pPr>");
                    self.inlines(term, &RunStyle { bold: true, ..RunStyle::default() });
                    self.xml.push_str("</w:p>");
                    self.blocks(content, None);
                }
            }
            Block::Quote(content) => {
                self.xml.push_str("<w:p><w:pPr><w:pStyle w:val=\"Quote\"/></w:pPr></w:p>");
                self.blocks(content, forced_align);
            }
            Block::AlignBlock(align, content) => self.blocks(content, Some(*align)),
            Block::Verbatim { lines, .. } => {
                for line in lines {
                    self.xml.push_str("<w:p><w:pPr><w:pStyle w:val=\"Verbatim\"/></w:pPr>");
                    self.code_run(line);
                    self.xml.push_str("</w:p>");
                }
            }
            Block::DisplayMath { omml, number, tag, anchor } => self.display_math(omml, number.as_deref(), tag.as_deref(), anchor.as_deref()),
            Block::TableBlock(table) => self.table(table),
            Block::Figure { content, caption, anchor, kind } => {
                self.blocks(content, None);
                if let Some(caption) = caption {
                    self.xml.push_str("<w:p><w:pPr><w:pStyle w:val=\"Caption\"/></w:pPr>");
                    let caption_anchor = caption.label.as_ref().or(anchor.as_ref()).cloned();
                    let name = if kind == "table" { "Table" } else { "Figure" };
                    self.text_run(&format!("{} ", name), &RunStyle::default());
                    if let Some(anchor) = &caption_anchor {
                        self.bookmark_start(anchor);
                    }
                    if let Some(number) = &caption.number {
                        self.text_run(number, &RunStyle::default());
                    }
                    if caption_anchor.is_some() {
                        self.bookmark_end();
                    }
                    self.text_run(": ", &RunStyle::default());
                    if let Some(anchor) = &caption_anchor {
                        self.bookmark_start(&format!("{}_title", anchor));
                    }
                    self.inlines(&caption.content, &RunStyle::default());
                    if caption_anchor.is_some() {
                        self.bookmark_end();
                    }
                    self.xml.push_str("</w:p>");
                } else if let Some(anchor) = anchor {
                    self.xml.push_str("<w:p>");
                    self.bookmark_start(anchor);
                    self.bookmark_end();
                    self.xml.push_str("</w:p>");
                }
            }
            Block::BibSection { title, items } => {
                self.xml.push_str("<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr>");
                self.text_run(title, &RunStyle::default());
                self.xml.push_str("</w:p>");
                for item in items {
                    self.xml.push_str("<w:p><w:pPr><w:pStyle w:val=\"Bibliography\"/></w:pPr>");
                    self.bookmark_start(&format!("bibitem-{}", item.key));
                    self.text_run(&format!("{} ", item.label), &RunStyle::default());
                    self.inlines(&item.content, &RunStyle::default());
                    self.bookmark_end();
                    self.xml.push_str("</w:p>");
                }
            }
            Block::GlossSection { title, entries } => {
                self.xml.push_str("<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr>");
                self.text_run(title, &RunStyle::default());
                self.xml.push_str("</w:p>");
                for entry in entries {
                    self.xml.push_str("<w:p><w:pPr><w:pStyle w:val=\"Glossary\"/></w:pPr>");
                    self.text_run(&entry.abbr, &RunStyle { bold: true, ..RunStyle::default() });
                    let mut text = format!(" — {}", entry.long);
                    if let Some(desc) = &entry.desc {
                        if !desc.is_empty() && desc != &entry.long {
                            text.push_str(&format!(" — {}", desc));
                        }
                    }
                    self.text_run(&text, &RunStyle::default());
                    self.xml.push_str("</w:p>");
                }
            }
            Block::TitlePage { title, author, date, thanks } => {
                self.xml.push_str("<w:p><w:pPr><w:pStyle w:val=\"Title\"/></w:pPr>");
                self.inlines(title, &RunStyle::default());
                self.xml.push_str("</w:p><w:p><w:pPr><w:pStyle w:val=\"Author\"/></w:pPr>");
                self.inlines(author, &RunStyle::default());
                self.xml.push_str("</w:p><w:p><w:pPr><w:pStyle w:val=\"Date\"/></w:pPr>");
                self.inlines(date, &RunStyle::default());
                self.xml.push_str("</w:p>");
                for note in thanks {
                    self.xml.push_str("<w:p><w:pPr><w:pStyle w:val=\"FootnoteText\"/></w:pPr>");
                    self.inlines(note, &RunStyle::default());
                    self.xml.push_str("</w:p>");
                }
            }
            Block::Toc => {
                self.xml.push_str("<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>Contents</w:t></w:r></w:p><w:p><w:r><w:fldChar w:fldCharType=\"begin\" w:dirty=\"true\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> TOC \\o \"1-3\" \\h \\z \\u </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>Update this field in Word to generate the table of contents.</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p>");
            }
            Block::PageBreak => self.xml.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>"),
            Block::HRule => self.xml.push_str("<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"000000\"/></w:pBdr></w:pPr></w:p>"),
            Block::Theorem { name, number, content, anchor } => {
                self.xml.push_str("<w:p><w:pPr><w:pStyle w:val=\"Theorem\"/></w:pPr>");
                self.text_run(name, &RunStyle { bold: true, ..RunStyle::default() });
                if let Some(number) = number {
                    self.text_run(" ", &RunStyle { bold: true, ..RunStyle::default() });
                    if let Some(anchor) = anchor {
                        self.bookmark_start(anchor);
                    }
                    self.text_run(number, &RunStyle { bold: true, ..RunStyle::default() });
                    if anchor.is_some() {
                        self.bookmark_end();
                    }
                }
                self.xml.push_str("</w:p>");
                self.blocks(content, None);
            }
            Block::Spacer(points) => {
                let twips = (points.max(0.0) * 20.0).round() as u32;
                self.xml.push_str(&format!("<w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"{}\" w:lineRule=\"exact\"/></w:pPr></w:p>", twips.max(1)));
            }
        }
    }

    fn paragraph(
        &mut self,
        inlines: &[Inline],
        style: Option<&str>,
        align: Option<Align>,
        indent: bool,
    ) {
        self.xml.push_str("<w:p><w:pPr>");
        if let Some(style) = style {
            self.xml
                .push_str(&format!("<w:pStyle w:val=\"{}\"/>", style));
        }
        if let Some(align) = align {
            self.xml
                .push_str(&format!("<w:jc w:val=\"{}\"/>", alignment(align)));
        }
        if indent {
            self.xml.push_str("<w:ind w:firstLine=\"480\"/>");
        }
        self.xml.push_str(MATH_TABS);
        self.xml.push_str("</w:pPr>");
        self.inlines(inlines, &RunStyle::default());
        self.xml.push_str("</w:p>");
        self.last_was_paragraph = true;
    }

    fn inlines(&mut self, inlines: &[Inline], style: &RunStyle) {
        for inline in inlines {
            match inline {
                Inline::Text(text) => self.text_run(text, style),
                Inline::Space => self.text_run(" ", style),
                Inline::Nbsp => self.text_run("\u{00a0}", style),
                Inline::LineBreak => self.xml.push_str("<w:r><w:br/></w:r>"),
                Inline::SoftHyphen => self.xml.push_str("<w:r><w:softHyphen/></w:r>"),
                Inline::Bold(content) => self.inlines(
                    content,
                    &RunStyle {
                        bold: true,
                        ..style.clone()
                    },
                ),
                Inline::Italic(content) | Inline::Slanted(content) => self.inlines(
                    content,
                    &RunStyle {
                        italic: true,
                        ..style.clone()
                    },
                ),
                Inline::BoldItalic(content) => self.inlines(
                    content,
                    &RunStyle {
                        bold: true,
                        italic: true,
                        ..style.clone()
                    },
                ),
                Inline::Underline(content) => self.inlines(
                    content,
                    &RunStyle {
                        underline: true,
                        ..style.clone()
                    },
                ),
                Inline::Strike(content) => self.inlines(
                    content,
                    &RunStyle {
                        strike: true,
                        ..style.clone()
                    },
                ),
                Inline::SmallCaps(content) => self.inlines(
                    content,
                    &RunStyle {
                        caps: true,
                        ..style.clone()
                    },
                ),
                Inline::Superscript(content) => self.inlines(
                    content,
                    &RunStyle {
                        vertical: Some("superscript"),
                        ..style.clone()
                    },
                ),
                Inline::Subscript(content) => self.inlines(
                    content,
                    &RunStyle {
                        vertical: Some("subscript"),
                        ..style.clone()
                    },
                ),
                Inline::Family(family, content) => self.inlines(
                    content,
                    &RunStyle {
                        family: Some(*family),
                        ..style.clone()
                    },
                ),
                Inline::FontSize(size, content) => self.inlines(
                    content,
                    &RunStyle {
                        size: Some(*size),
                        ..style.clone()
                    },
                ),
                Inline::Color { color, content } => self.inlines(
                    content,
                    &RunStyle {
                        color: Some(color.clone()),
                        ..style.clone()
                    },
                ),
                Inline::Highlight { color, content } => self.inlines(
                    content,
                    &RunStyle {
                        highlight: Some(color.clone()),
                        ..style.clone()
                    },
                ),
                Inline::Math { omml, .. } => {
                    if omml.starts_with("<m:oMath") {
                        self.xml.push_str(omml);
                    } else {
                        self.xml.push_str(&format!("<m:oMath>{}</m:oMath>", omml));
                    }
                }
                Inline::Ref {
                    text,
                    anchor,
                    kind,
                    counter,
                    ..
                } => {
                    if let Some(anchor) = anchor {
                        let anchor = safe_anchor(anchor);
                        let instruction = match kind {
                            RefKind::Page => format!(" PAGEREF {} \\h ", anchor),
                            RefKind::Name => format!(" REF {} \\h ", anchor),
                            RefKind::Number
                                if matches!(
                                    counter.as_str(),
                                    "part"
                                        | "chapter"
                                        | "section"
                                        | "subsection"
                                        | "subsubsection"
                                        | "paragraph"
                                        | "subparagraph"
                                ) =>
                            {
                                format!(" REF {} \\r \\h ", anchor)
                            }
                            RefKind::Number => format!(" REF {} \\h ", anchor),
                        };
                        self.field(
                            &instruction,
                            text,
                            &RunStyle {
                                hyperlink: true,
                                ..style.clone()
                            },
                        );
                    } else {
                        self.text_run(text, style);
                    }
                }
                Inline::Cite { text, links } => {
                    if let Some((target, _)) = links.first() {
                        self.xml.push_str(&format!(
                            "<w:hyperlink w:anchor=\"{}\" w:history=\"1\">",
                            escape(&safe_anchor(target.trim_start_matches('#')))
                        ));
                        self.text_run(
                            text,
                            &RunStyle {
                                hyperlink: true,
                                ..style.clone()
                            },
                        );
                        self.xml.push_str("</w:hyperlink>");
                    } else {
                        self.text_run(text, style);
                    }
                }
                Inline::Link {
                    url,
                    content,
                    external,
                } => {
                    if *external {
                        let id = self.add_relationship(REL_HYPERLINK, url, true);
                        self.xml
                            .push_str(&format!("<w:hyperlink r:id=\"{}\" w:history=\"1\">", id));
                    } else {
                        self.xml.push_str(&format!(
                            "<w:hyperlink w:anchor=\"{}\" w:history=\"1\">",
                            escape(&safe_anchor(url.trim_start_matches('#')))
                        ));
                    }
                    self.inlines(
                        content,
                        &RunStyle {
                            hyperlink: true,
                            ..style.clone()
                        },
                    );
                    self.xml.push_str("</w:hyperlink>");
                }
                Inline::Image(image) => self.image(image),
                Inline::Footnote(block) => {
                    let source = if self.next_source_footnote < self.footnote_sources.len() {
                        let source = self.footnote_sources[self.next_source_footnote].clone();
                        self.next_source_footnote += 1;
                        source
                    } else {
                        (**block).clone()
                    };
                    let id = self.footnotes.len() + 1;
                    self.footnotes.push(source);
                    self.xml.push_str(&format!("<w:r><w:rPr><w:rStyle w:val=\"FootnoteReference\"/></w:rPr><w:footnoteReference w:id=\"{}\"/></w:r>", id));
                }
                Inline::Code(code) => self.code_run(code),
                Inline::Group(content) => self.inlines(content, style),
                Inline::Meta(meta) => {
                    if let Some(anchor) = meta.strip_prefix("anchor:") {
                        self.bookmark_start(anchor);
                        self.bookmark_end();
                    }
                }
            }
        }
    }

    fn field(&mut self, instruction: &str, cached: &str, style: &RunStyle) {
        self.xml
            .push_str("<w:r><w:fldChar w:fldCharType=\"begin\" w:dirty=\"true\"/></w:r>");
        self.xml.push_str(&format!(
            "<w:r><w:instrText xml:space=\"preserve\">{}</w:instrText></w:r>",
            escape(instruction)
        ));
        self.xml
            .push_str("<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>");
        self.text_run(cached, style);
        self.xml
            .push_str("<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>");
    }

    fn text_run(&mut self, text: &str, style: &RunStyle) {
        self.xml.push_str("<w:r>");
        self.run_properties(style);
        self.xml.push_str(&format!(
            "<w:t xml:space=\"preserve\">{}</w:t></w:r>",
            escape(text)
        ));
    }

    fn code_run(&mut self, code: &str) {
        let style = RunStyle {
            family: Some(FontFamily::Tt),
            ..RunStyle::default()
        };
        self.xml.push_str("<w:r>");
        self.run_properties(&style);
        let mut parts = code.split('\n').peekable();
        while let Some(part) = parts.next() {
            self.xml.push_str(&format!(
                "<w:t xml:space=\"preserve\">{}</w:t>",
                escape(part)
            ));
            if parts.peek().is_some() {
                self.xml.push_str("<w:br/>");
            }
        }
        self.xml.push_str("</w:r>");
    }

    fn run_properties(&mut self, style: &RunStyle) {
        self.xml.push_str("<w:rPr>");
        if style.hyperlink {
            self.xml.push_str("<w:rStyle w:val=\"Hyperlink\"/>");
        }
        if style.bold {
            self.xml.push_str("<w:b/>");
        }
        if style.italic {
            self.xml.push_str("<w:i/>");
        }
        if style.underline {
            self.xml.push_str("<w:u w:val=\"single\"/>");
        }
        if style.strike {
            self.xml.push_str("<w:strike/>");
        }
        if style.caps {
            self.xml.push_str("<w:caps/>");
        }
        if let Some(family) = style.family {
            let font = match family {
                FontFamily::Rm => "Cambria",
                FontFamily::Sf => "Calibri",
                FontFamily::Tt => "Consolas",
            };
            self.xml.push_str(&format!(
                "<w:rFonts w:ascii=\"{}\" w:hAnsi=\"{}\"/>",
                font, font
            ));
        }
        if let Some(size) = style.size {
            let points = match size {
                FontSize::Tiny => 5,
                FontSize::Scriptsize => 7,
                FontSize::Footnotesize => 8,
                FontSize::Small => 9,
                FontSize::Normalsize => 12,
                FontSize::Large => 14,
                FontSize::LargeX => 17,
                FontSize::LargeXX => 20,
                FontSize::Huge => 24,
                FontSize::HugeX => 28,
            };
            self.xml.push_str(&format!(
                "<w:sz w:val=\"{}\"/><w:szCs w:val=\"{}\"/>",
                points * 2,
                points * 2
            ));
        }
        if let Some(color) = &style.color {
            self.xml
                .push_str(&format!("<w:color w:val=\"{}\"/>", escape(color)));
        }
        if let Some(color) = &style.highlight {
            self.xml.push_str(&format!(
                "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
                escape(color)
            ));
        }
        if let Some(vertical) = style.vertical {
            self.xml
                .push_str(&format!("<w:vertAlign w:val=\"{}\"/>", vertical));
        }
        self.xml.push_str("</w:rPr>");
    }

    fn display_math(
        &mut self,
        omml: &str,
        number: Option<&str>,
        tag: Option<&str>,
        anchor: Option<&str>,
    ) {
        if self.last_was_paragraph && self.xml.ends_with("</w:p>") {
            self.xml.truncate(self.xml.len() - "</w:p>".len());
        } else {
            self.xml.push_str("<w:p><w:pPr>");
            self.xml.push_str(MATH_TABS);
            self.xml.push_str("</w:pPr>");
        }
        self.xml
            .push_str("<w:r><w:br/></w:r><w:r><w:tab/></w:r><m:oMath>");
        let body = omml
            .strip_prefix("<m:oMathPara><m:oMath>")
            .and_then(|s| s.strip_suffix("</m:oMath></m:oMathPara>"))
            .or_else(|| {
                omml.strip_prefix("<m:oMath>")
                    .and_then(|s| s.strip_suffix("</m:oMath>"))
            })
            .unwrap_or(omml);
        self.xml.push_str(body);
        self.xml.push_str("</m:oMath>");
        if tag.is_some() || number.is_some() {
            self.xml.push_str("<w:r><w:tab/></w:r>");
            self.text_run("(", &RunStyle::default());
            if let Some(anchor) = anchor {
                self.bookmark_start(anchor);
            }
            if let Some(tag) = tag {
                self.text_run(tag, &RunStyle::default());
            } else if let Some(number) = number {
                self.field(" SEQ equation \\* ARABIC ", number, &RunStyle::default());
            }
            if anchor.is_some() {
                self.bookmark_end();
            }
            self.text_run(")", &RunStyle::default());
        } else if let Some(anchor) = anchor {
            self.bookmark_start(anchor);
            self.bookmark_end();
        }
        self.xml.push_str("</w:p>");
        self.last_was_paragraph = true;
    }

    fn table(&mut self, table: &Table) {
        self.xml.push_str(
            "<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/><w:jc w:val=\"center\"/>",
        );
        if table.borders {
            self.xml.push_str("<w:tblBorders><w:top w:val=\"single\" w:sz=\"4\"/><w:left w:val=\"single\" w:sz=\"4\"/><w:bottom w:val=\"single\" w:sz=\"4\"/><w:right w:val=\"single\" w:sz=\"4\"/><w:insideH w:val=\"single\" w:sz=\"4\"/><w:insideV w:val=\"single\" w:sz=\"4\"/></w:tblBorders>");
        }
        self.xml.push_str("</w:tblPr>");
        let columns = table
            .rows
            .iter()
            .map(|r| r.cells.iter().map(|c| c.colspan).sum())
            .max()
            .unwrap_or(table.aligns.len())
            .max(1);
        self.xml.push_str("<w:tblGrid>");
        for _ in 0..columns {
            self.xml.push_str("<w:gridCol w:w=\"2400\"/>");
        }
        self.xml.push_str("</w:tblGrid>");
        for row in &table.rows {
            self.xml.push_str("<w:tr>");
            for cell in &row.cells {
                self.xml.push_str("<w:tc><w:tcPr>");
                if cell.colspan > 1 {
                    self.xml
                        .push_str(&format!("<w:gridSpan w:val=\"{}\"/>", cell.colspan));
                }
                if cell.rowspan > 1 {
                    self.xml.push_str("<w:vMerge w:val=\"restart\"/>");
                }
                self.xml.push_str(&format!(
                    "<w:tcBorders>{}{}</w:tcBorders></w:tcPr>",
                    rule("top", row.rule_top),
                    rule("bottom", row.rule_bottom)
                ));
                if cell.blocks.is_empty() {
                    self.xml.push_str("<w:p/>");
                } else {
                    self.blocks(&cell.blocks, Some(cell.align));
                }
                self.xml.push_str("</w:tc>");
            }
            self.xml.push_str("</w:tr>");
        }
        self.xml.push_str("</w:tbl>");
    }

    fn image(&mut self, image: &Image) {
        let index = self.images.len() + 1;
        let is_svg = std::str::from_utf8(&image.data)
            .map(|s| s.contains("<svg"))
            .unwrap_or(false);
        let extension = if is_svg {
            "svg"
        } else if image.data.starts_with(b"\x89PNG") {
            "png"
        } else {
            "jpg"
        };
        let content_type = if is_svg {
            "image/svg+xml"
        } else if extension == "png" {
            "image/png"
        } else {
            "image/jpeg"
        };
        let name = format!("image{}.{}", index, extension);
        let id = self.add_relationship(REL_IMAGE, &format!("media/{}", name), false);
        self.images.push(ImagePart {
            name,
            content_type,
            data: image.data.clone(),
        });
        let ratio = image.height_px.max(1) as f64 / image.width_px.max(1) as f64;
        let (width_pt, height_pt) = match (image.width_pt, image.height_pt) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * ratio),
            (None, Some(h)) => (h / ratio, h),
            (None, None) => (
                image.width_px as f64 * 72.0 / 96.0,
                image.height_px as f64 * 72.0 / 96.0,
            ),
        };
        let cx = (width_pt.max(1.0) * 12700.0).round() as u64;
        let cy = (height_pt.max(1.0) * 12700.0).round() as u64;
        let drawing_id = self.drawing_id;
        self.drawing_id += 1;
        self.xml.push_str(&format!("<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{}\" cy=\"{}\"/><wp:docPr id=\"{}\" name=\"{}\" descr=\"{}\"/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{}\" name=\"{}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>", cx, cy, drawing_id, escape(&image.path), escape(&image.alt), drawing_id, escape(&image.path), id, cx, cy));
    }

    fn add_relationship(&mut self, kind: &str, target: &str, external: bool) -> String {
        let id = format!("rId{}", self.relationships.len() + 10);
        self.relationships.push(Relationship {
            id: id.clone(),
            kind: kind.into(),
            target: target.into(),
            external,
        });
        id
    }

    fn bookmark_start(&mut self, anchor: &str) {
        let id = self.bookmark_id;
        self.bookmark_id += 1;
        self.xml.push_str(&format!(
            "<w:bookmarkStart w:id=\"{}\" w:name=\"{}\"/>",
            id,
            escape(&safe_anchor(anchor))
        ));
    }

    fn bookmark_end(&mut self) {
        let id = self.bookmark_id - 1;
        self.xml
            .push_str(&format!("<w:bookmarkEnd w:id=\"{}\"/>", id));
    }

    fn document_relationships(&self) -> String {
        let mut xml = format!("{}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">", XML_DECL);
        xml.push_str("<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes\" Target=\"footnotes.xml\"/><Relationship Id=\"rId4\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>");
        for rel in &self.relationships {
            xml.push_str(&format!(
                "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"{}/>",
                rel.id,
                rel.kind,
                escape(&rel.target),
                if rel.external {
                    " TargetMode=\"External\""
                } else {
                    ""
                }
            ));
        }
        xml.push_str("</Relationships>");
        xml
    }

    fn footnotes_xml(&self) -> String {
        let mut xml = format!("{}<w:footnotes xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\">", XML_DECL);
        xml.push_str("<w:footnote w:type=\"separator\" w:id=\"-1\"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>");
        for (index, block) in self.footnotes.iter().enumerate() {
            xml.push_str(&format!("<w:footnote w:id=\"{}\"><w:p><w:pPr><w:pStyle w:val=\"FootnoteText\"/></w:pPr><w:r><w:rPr><w:rStyle w:val=\"FootnoteReference\"/></w:rPr><w:footnoteRef/></w:r>", index + 1));
            simple_block_xml(&mut xml, block);
            xml.push_str("</w:p></w:footnote>");
        }
        xml.push_str("</w:footnotes>");
        xml
    }
}

fn simple_block_xml(xml: &mut String, block: &Block) {
    if let Block::Para { inlines, .. } = block {
        simple_inlines_xml(xml, inlines);
    } else if let Block::AlignBlock(_, blocks) = block {
        for block in blocks {
            simple_block_xml(xml, block);
        }
    }
}

fn simple_inlines_xml(xml: &mut String, inlines: &[Inline]) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => xml.push_str(&format!("<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>", escape(text))),
            Inline::Space => xml.push_str("<w:r><w:t xml:space=\"preserve\"> </w:t></w:r>"),
            Inline::Nbsp => xml.push_str("<w:r><w:t xml:space=\"preserve\">\u{00a0}</w:t></w:r>"),
            Inline::LineBreak => xml.push_str("<w:r><w:br/></w:r>"),
            Inline::Math { omml, .. } => xml.push_str(&format!("<m:oMath>{}</m:oMath>", omml)),
            Inline::Ref { text, .. } | Inline::Cite { text, .. } => xml.push_str(&format!("<w:r><w:t>{}</w:t></w:r>", escape(text))),
            Inline::Code(code) => xml.push_str(&format!("<w:r><w:rPr><w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r>", escape(code))),
            Inline::Bold(content) => {
                xml.push_str("<w:r><w:rPr><w:b/></w:rPr><w:t>");
                xml.push_str(&escape(&inlines_to_text(content)));
                xml.push_str("</w:t></w:r>");
            }
            Inline::Italic(content) | Inline::Slanted(content) => {
                xml.push_str("<w:r><w:rPr><w:i/></w:rPr><w:t>");
                xml.push_str(&escape(&inlines_to_text(content)));
                xml.push_str("</w:t></w:r>");
            }
            Inline::Group(content)
            | Inline::BoldItalic(content)
            | Inline::Underline(content)
            | Inline::Strike(content)
            | Inline::SmallCaps(content)
            | Inline::Superscript(content)
            | Inline::Subscript(content)
            | Inline::Family(_, content)
            | Inline::FontSize(_, content) => simple_inlines_xml(xml, content),
            Inline::Color { content, .. } | Inline::Highlight { content, .. } | Inline::Link { content, .. } => simple_inlines_xml(xml, content),
            _ => {}
        }
    }
}

fn safe_anchor(anchor: &str) -> String {
    let mut value: String = anchor
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if value.is_empty() {
        value.push_str("ref");
    }
    if value.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        value.insert_str(0, "ref_");
    }
    value.truncate(40);
    value
}

fn alignment(align: Align) -> &'static str {
    match align {
        Align::Left => "left",
        Align::Center => "center",
        Align::Right => "right",
        Align::Justify => "both",
    }
}

fn rule(side: &str, kind: RuleKind) -> String {
    match kind {
        RuleKind::None => String::new(),
        RuleKind::Thin => format!(
            "<w:{} w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/>",
            side
        ),
        RuleKind::Thick => format!(
            "<w:{} w:val=\"single\" w:sz=\"12\" w:color=\"000000\"/>",
            side
        ),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn content_types(images: &[ImagePart]) -> String {
    let jpeg = images.iter().any(|i| i.content_type == "image/jpeg");
    let svg = images.iter().any(|i| i.content_type == "image/svg+xml");
    format!("{}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/>{}{}<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/><Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/><Override PartName=\"/word/footnotes.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml\"/><Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/></Types>", XML_DECL, if jpeg { "<Default Extension=\"jpg\" ContentType=\"image/jpeg\"/><Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>" } else { "" }, if svg { "<Default Extension=\"svg\" ContentType=\"image/svg+xml\"/>" } else { "" })
}

fn root_relationships() -> String {
    format!("{}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{}\" Target=\"word/document.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/></Relationships>", XML_DECL, REL_OFFICE)
}

fn styles() -> String {
    let mut xml = format!("{}<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:eastAsia=\"SimSun\"/><w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"120\" w:line=\"276\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>", XML_DECL);
    xml.push_str("<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style><w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:rPr><w:color w:val=\"0563C1\"/><w:u w:val=\"single\"/></w:rPr></w:style><w:style w:type=\"character\" w:styleId=\"FootnoteReference\"><w:name w:val=\"footnote reference\"/><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr></w:style>");
    for level in 1..=9 {
        let size = (34 - level * 2).max(22);
        xml.push_str(&format!("<w:style w:type=\"paragraph\" w:styleId=\"Heading{}\"><w:name w:val=\"heading {}\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before=\"240\" w:after=\"120\"/><w:outlineLvl w:val=\"{}\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"{}\"/><w:szCs w:val=\"{}\"/></w:rPr></w:style>", level, level, level - 1, size, size));
    }
    xml.push_str("<w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/><w:pPr><w:jc w:val=\"center\"/><w:spacing w:before=\"2400\" w:after=\"360\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"44\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Author\"><w:name w:val=\"Author\"/><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:rPr><w:sz w:val=\"28\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Date\"><w:name w:val=\"Date\"/><w:pPr><w:jc w:val=\"center\"/></w:pPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Caption\"><w:name w:val=\"caption\"/><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:rPr><w:i/><w:sz w:val=\"20\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Bibliography\"><w:name w:val=\"Bibliography\"/><w:pPr><w:ind w:left=\"480\" w:hanging=\"480\"/></w:pPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Glossary\"><w:name w:val=\"Glossary\"/><w:pPr><w:ind w:left=\"480\" w:hanging=\"480\"/></w:pPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:pPr><w:ind w:left=\"720\"/></w:pPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"FootnoteText\"><w:name w:val=\"footnote text\"/><w:rPr><w:sz w:val=\"20\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Verbatim\"><w:name w:val=\"Verbatim\"/><w:pPr><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:rPr><w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\"/><w:sz w:val=\"20\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/><w:pPr><w:ind w:left=\"720\" w:right=\"720\"/></w:pPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Theorem\"><w:name w:val=\"Theorem\"/><w:pPr><w:keepNext/></w:pPr></w:style>");
    xml.push_str("</w:styles>");
    xml
}

fn numbering() -> String {
    let mut xml = format!("{}<w:numbering xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:abstractNum w:abstractNumId=\"0\"><w:multiLevelType w:val=\"singleLevel\"/><w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"•\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"720\" w:hanging=\"360\"/></w:pPr><w:rPr><w:rFonts w:ascii=\"Symbol\" w:hAnsi=\"Symbol\" w:hint=\"default\"/></w:rPr></w:lvl></w:abstractNum><w:abstractNum w:abstractNumId=\"1\"><w:multiLevelType w:val=\"singleLevel\"/><w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/><w:lvlText w:val=\"%1.\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"720\" w:hanging=\"360\"/></w:pPr></w:lvl></w:abstractNum>", XML_DECL);
    xml.push_str(
        "<w:abstractNum w:abstractNumId=\"2\"><w:multiLevelType w:val=\"hybridMultilevel\"/>",
    );
    for level in 0..9 {
        let mut text = String::new();
        for part in 0..=level {
            if !text.is_empty() {
                text.push('.');
            }
            text.push_str(&format!("%{}", part + 1));
        }
        xml.push_str(&format!("<w:lvl w:ilvl=\"{}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/><w:pStyle w:val=\"Heading{}\"/><w:lvlText w:val=\"{}\"/><w:lvlJc w:val=\"left\"/><w:suff w:val=\"space\"/><w:pPr><w:ind w:left=\"0\" w:hanging=\"0\"/></w:pPr></w:lvl>", level, level + 1, text));
    }
    xml.push_str("</w:abstractNum><w:num w:numId=\"19\"><w:abstractNumId w:val=\"2\"/></w:num>");
    for depth in 0..9 {
        let base = depth * 2 + 1;
        xml.push_str(&format!("<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"0\"/></w:num><w:num w:numId=\"{}\"><w:abstractNumId w:val=\"1\"/></w:num>", base, base + 1));
    }
    xml.push_str("</w:numbering>");
    xml
}

fn settings() -> String {
    format!("{}<w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:updateFields w:val=\"true\"/></w:settings>", XML_DECL)
}

fn core_properties() -> String {
    format!("{}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:creator>doclatex</dc:creator><cp:lastModifiedBy>doclatex</cp:lastModifiedBy></cp:coreProperties>", XML_DECL)
}

fn app_properties() -> String {
    format!("{}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>doclatex</Application></Properties>", XML_DECL)
}
