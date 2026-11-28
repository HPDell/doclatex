# DocLaTeX

**English** | [简体中文](README.zh-Hans.md)

DocLaTeX is an experimental Rust command-line tool that converts LaTeX source for scholarly documents directly into Microsoft Word `.docx` files. It implements its own lexer, macro expander, document parser, bibliography processor, math-to-OMML converter, and OOXML writer. It does **not** invoke a TeX/LaTeX engine, BibTeX, or Biber.

The main goals are to preserve document semantics rather than approximate a PDF page: editable Word equations, working cross-references, automatic heading and equation numbering, citations, glossaries, and reusable Word styles.

> **Project status:** DocLaTeX supports a substantial but incomplete LaTeX subset. It is not a complete TeX implementation or a drop-in replacement for every package and document class. Check the warnings printed during conversion and visually inspect important documents.

## Implemented features

### Macro expansion and source processing

- TeX-aware, lazy tokenization with category-code changes such as `\makeatletter` and `\makeatother`.
- Macro definitions using `\def`, `\gdef`, `\edef`, `\xdef`, `\let`, `\newcommand`, `\renewcommand`, `\providecommand`, and `\DeclareRobustCommand`.
- A supported subset of `xparse` argument specifications through `\NewDocumentCommand` and related commands, including mandatory and optional arguments.
- User macro expansion in both text and mathematics, including custom math operators.
- Conditionals, counters, number formatting, user-defined environments, and theorem declarations.
- Recursive `\input` and `\include` processing.
- `filecontents` environments for generating local source/assets when required by a document.

### Document classes and packages

- Parsing of `\documentclass`, `\LoadClass`, `\usepackage`, and `\RequirePackage`.
- Loading macro definitions from local `.cls` and `.sty` files, including files loaded from other class or package files.
- Class/package option handling, including option declaration, processing, execution, and forwarding.
- File lookup relative to the current source file and project directory, with additional colon-separated search directories from `TEXINPUTS`.
- Built-in handling for common `article`, `report`, and `book` behavior, selected font-size and `a4paper` options, and basic Chinese class/package behavior.
- Recognition of common scholarly packages such as `amsmath`, `graphicx`, `hyperref`, `glossaries`, `natbib`, `cleveref`, `booktabs`, and `geometry`.

Class and package support is semantic but partial: recognizing a package name does not mean that every command provided by that package is implemented.

### Document structure and text

- Parts, chapters, sections, subsections, and lower heading levels, including starred headings and appendix lettering.
- Word heading styles with multilevel numbering lists.
- `\maketitle` with title, author, date, and `\thanks` content.
- A Word table-of-contents field for `\tableofcontents`.
- Paragraph alignment, indentation, explicit line breaks, page breaks, and basic vertical spacing.
- Bold, italic, slanted, small-caps, underlined, struck-through, monospaced, colored, highlighted, superscript, and subscript text.
- Itemized, enumerated, nested, custom-label, and description lists.
- Footnotes, external hyperlinks, verbatim text, and `lstlisting` content.
- `abstract`, quotation, alignment, `minipage`, theorem-like, and user-defined environments.
- Generic paragraph-based environments receive their own Word paragraph style; `abstract` uses the dedicated `Abstract` style.

### Mathematics

- Inline and display LaTeX mathematics converted directly to editable Office Math Markup Language (OMML), rather than embedded as screenshots.
- Fractions, radicals, superscripts/subscripts, n-ary operators and limits, delimiters, accents, over/under braces, matrices, arrays, cases, boxed expressions, phantom content, and common Greek/symbol/operator commands.
- Math alphabets and text commands such as `\mathrm`, `\mathbf`, `\mathcal`, `\mathbb`, `\mathfrak`, and `\text`.
- `equation`, `align`, `gather`, `multline`, `alignat`, `eqnarray`, and related starred environments, plus `subequations` and per-row `\nonumber`/`\notag` handling.
- Alignment at `&` positions through Word's manual alignment markers.
- Equation numbers generated with Word `SEQ` fields and placed using the professional-math `#(number)` convention.
- Bookmarks at equation numbers and cached reference text for use before fields are updated.

### Cross-references

- `\label` targets on headings, figures, tables, equations, and theorem-like environments.
- `\ref`, `\eqref`, `\pageref`, `\nameref`, `\autoref`, `\cref`, and `\Cref`.
- Word bookmarks and `REF`/`PAGEREF` fields, with hyperlinks to reference targets.
- Custom reference names through `\crefname` and `\Crefname`.
- A two-pass compilation pipeline and a project-specific `.aux` file to carry labels and bibliography information between passes.

### Bibliographies and citations

- Direct parsing of `.bib` files; no external BibTeX installation is required.
- Multiple bibliography files, multiple citation keys, citation-ordering/compression behavior, and `\nocite{*}`.
- `\cite`, `\citet`, `\citep`, `\citealt`, `\citealp`, `\citeauthor`, `\citeyear`, `\citeyearpar`, and `\citeonline`.
- Numeric, alphabetic, IEEE-like, and author-year output for recognized `plain`, `unsrt`, `alpha`, `abbrv`, `ieeetr`/`IEEEtran`, `apalike`, and related styles.
- Manually written `thebibliography` environments and `\bibitem` entries.
- Bibliography entries with internal links from citations.

The bibliography formatter is intentionally smaller than BibTeX's `.bst` language and does not reproduce every journal style.

### Acronyms and glossaries

- `\newacronym` and `\newglossaryentry` definitions.
- First-use expansion and subsequent short forms, capitalization, plural forms, and explicit short/long/full acronym commands.
- Common `\gls`, `\glspl`, `\acrshort`, `\acrlong`, `\acrfull`, and acronym-package aliases.
- `\printglossaries`, `\printglossary`, and `\printacronyms` output without running `makeindex` or `xindy`.

### Figures and tables

- PNG, JPEG, and SVG images embedded directly in the DOCX package.
- PDF figures converted to SVG and embedded; only the first PDF page is used.
- `\includegraphics` sizing through `width`, `height`, and `scale`, including lengths relative to `\textwidth` while preserving aspect ratio when appropriate.
- Centered images with a dedicated `Picture` style that keeps the image with the following paragraph.
- Figure/table captions, labels, and separate built-in `Caption` and custom `TableCaption` styles.
- Centered `tabular`, `tabularx`, and `tabular*` output with column alignment, paragraph-column widths, rules, borders, and supported cell spanning.
- Table text uses the `TableText` style: single spacing and no first-line or hanging indentation.

### Word document generation

- Native OOXML packaging for document content, styles, numbering, relationships, settings, properties, footnotes, and media.
- Built-in Word styles where appropriate, including body text, first-line-indented body text, headings, lists, captions, footnotes, and bibliography entries.
- Dedicated styles for pictures, table text, table captions, abstracts, verbatim content, and other parsed environments.
- Word fields marked for automatic update, with cached values so content remains visible before updating.

## Requirements and building

- A Rust toolchain with Cargo. The repository does not currently pin a specific Rust version.
- Optional, for PDF figures only: either `pdftocairo` or `mutool` available on `PATH`. `pdftocairo -svg` is tried first, then `mutool draw`.

Build the optimized executable:

```sh
cargo build --release
```

The executable is written to `target/release/doclatex`.

## Usage

```text
doclatex <input.tex> [-o <output.docx>] [-v]
doclatex --help
doclatex --version
```

For example, from this repository:

```sh
cargo run -- path/to/main.tex --output path/to/main.docx
```

Or run the release binary directly:

```sh
target/release/doclatex path/to/main.tex -o path/to/main.docx
```

If `-o`/`--output` is omitted, the input extension is replaced with `.docx`. Use `-v` or `--verbose` to print four compilation stages to standard error; by default, only warnings and the final output path are printed. Each compile automatically performs two expansion/parsing passes and writes an `.aux` file beside the input file. Existing output and `.aux` files are overwritten.

Relative input and output paths are resolved against the directory from which DocLaTeX is invoked. Included sources, classes, packages, graphics, and bibliographies should therefore remain available at their original project-relative paths.

## Examples

`examples/demo.tex` is a compact demonstration of macros, references, equations, acronyms, tables, figures, verbatim text, and bibliographies. To try it without overwriting the checked-in example outputs:

```sh
tmpdir=$(mktemp -d)
cp examples/demo.tex examples/refs.bib examples/pixel.png examples/vector.pdf "$tmpdir"/
cargo run -- "$tmpdir/demo.tex" --output "$tmpdir/result.docx"
```

`examples/attention-is-all-you-need/` contains a larger, multi-file example based on the TeX source of *Attention Is All You Need*. It exercises recursive `\input`, a custom `.sty` file, mathematics, tables, and local graphics.

## Opening and updating the DOCX file

The generated document contains cached field results and requests field updates when opened. Depending on the Word-compatible editor and its security settings, you may still need to update fields manually. In Microsoft Word, select the document content and press **F9**; update the entire table of contents if prompted.

This is especially relevant for the table of contents, page references, equation sequence fields, and any numbering affected by later edits. Layout can differ between Microsoft Word, WPS Office, and other OOXML editors, so visual inspection is recommended.

## Not yet supported or only partially supported

- **Complete TeX compatibility:** Delimited macro corner cases, the full TeX programming model, every category-code/primitive behavior, and arbitrary third-party packages are not guaranteed to work.
- **Exact class/package reproduction:** DocLaTeX does not execute a full TeX installation or arbitrary `.bst` files. Page geometry, font selection, headers/footers, spacing, and class-specific layout are only partially reproduced.
- **TeX-quality pagination:** Floats are emitted in their source positions rather than managed by TeX's float placement algorithm. Placement options are accepted but do not control Word pagination. Line breaking, hyphenation, widow/orphan control, and page optimization are left largely to the editor.
- **Graphics beyond basic images:** `tikzpicture` content is explicitly skipped. PGFPlots, TeX drawing code, EPS images, and conversion of arbitrary graphical environments are not currently rendered.
- **Lists and indexes:** `\listoffigures` and `\listoftables` are skipped. General index generation and `makeindex`/`xindy` processing are not implemented.
- **Full bibliography engines:** `biblatex`, Biber, CSL styles, and arbitrary BibTeX `.bst` programs are unsupported. Complex BibTeX field formatting may differ from TeX output.
- **Full glossary styles:** Glossary entries are generated directly, without external indexing tools; advanced sorting, hierarchy, and custom glossary styles are not fully implemented.
- **Every math construct:** The OMML converter covers common scholarly notation, but uncommon symbols, complex alignment, custom spacing, and package-specific math commands may be missing or rendered differently.
- **Advanced tables and floats:** `longtable`, complex multi-page tables, subfloat layouts, and all `array`/package extensions are not fully reproduced.
- **Byte-for-byte visual equivalence:** The output is a semantic Word document, not a replica of LaTeX-produced PDF pages.

Unsupported commands are often rendered literally or skipped with a warning so that conversion can continue. This best-effort behavior can produce incomplete documents; warnings should not be ignored for production use.

## Development

Run the unit tests and formatting check from the repository root:

```sh
cargo test
cargo fmt --check
```

The implementation is a single Rust binary crate. Its main stages are `token.rs` (lexing), `engine.rs` (macro expansion and class/package loading), `parse.rs` (document semantics), `math.rs` (OMML), `bib.rs` (bibliographies), `aux.rs` (two-pass data), and `docx.rs` (OOXML/DOCX output).

The checked-in `.docx` examples are illustrative outputs, not golden files compared by the test suite.

## Safety note

Conversion is not sandboxed. LaTeX input can read local files through includes, classes, packages, graphics, and bibliographies, and `filecontents` can write files. Review untrusted documents before compiling them, and use a temporary project copy when testing.
