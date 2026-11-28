# DocLaTeX guide for coding agents

## Project overview

DocLaTeX (`doclatex` 0.1.0) is a Rust command-line program that converts LaTeX source for scholarly documents directly into Microsoft Word `.docx` files. It does not invoke a LaTeX engine or BibTeX. Instead, it implements its own lexer, macro expander, document parser, bibliography processor, math-to-OMML converter, and OOXML writer.

The implementation supports a substantial but incomplete LaTeX subset. Unsupported input is often retained literally or skipped with a warning rather than causing conversion to fail. For example, `tikzpicture` content and the lists of figures/tables are explicitly unsupported. Do not describe the tool as a complete TeX implementation.

The repository is a single Cargo binary package using Rust edition 2021. There is no library target, web service, plugin system, or long-running runtime. `README.md` contains the English user documentation and `README.zh-Hans.md` is its Simplified Chinese counterpart; keep their feature and limitation descriptions synchronized. The source remains the authoritative description of current behavior.

### Key configuration files

- `Cargo.toml`: package metadata, the sole direct dependency, and Cargo profiles. Development builds disable debuginfo; release builds use `opt-level = 2` and no debuginfo.
- `Cargo.lock`: tracked dependency lockfile; keep it updated when dependencies change.
- `.gitignore`: ignores only `/target`. Example `.aux` and `.docx` files are tracked, not globally ignored.
- There is no `rust-toolchain` file, minimum supported Rust version declaration, rustfmt configuration, Clippy configuration, CI workflow, or release/deployment configuration.

The only direct Rust dependency is `zip` version 2 with default features disabled and the `deflate` feature enabled. XML and image metadata are handled with hand-written code rather than XML or image libraries.

## Source layout and responsibilities

All Rust code is under `src/` and is wired together as private modules of the binary in `src/main.rs`.

- `src/main.rs`: CLI parsing, path handling, working-directory setup, the two-pass compile orchestration, warning reporting, and final DOCX creation.
- `src/token.rs`: TeX tokens, category codes, and the line-lazy lexer. Lazy lexing matters because catcode changes such as `\makeatletter` must affect characters later in the same source stream.
- `src/engine.rs`: mutable compilation `State`, macro meanings and arguments, counters, conditionals, file inclusion, class/package loading, options, and the native-command table. This is the TeX expansion layer.
- `src/parse.rs`: conversion of expanded tokens into the document model. It handles headings, paragraphs, inline styles, environments, lists, floats, tables, graphics, references, citations, glossaries, theorem environments, and display-math boundaries. It also contains image sizing and PDF-to-SVG conversion helpers.
- `src/math.rs`: LaTeX math parser and direct emission of Office Math Markup Language (OMML) strings. It covers constructs including fractions, radicals, scripts, operators, delimiters, accents, matrices, cases, aligned rows, and user macros in math.
- `src/model.rs`: the intermediate representation shared by parsing and output. `Document` contains `Block` values; paragraphs and other containers contain `Inline` values. Math is already OMML at this layer. Labels and warnings are also carried here.
- `src/aux.rs`: reading and writing the project-specific `.aux` representation for labels, citations, bibliography files, and bibliography style.
- `src/bib.rs`: internal BibTeX-file parser, citation ordering and compression, label generation, and entry formatting for numeric, alpha, and author-year styles. It also supports manually written `thebibliography` content through the parser.
- `src/docx.rs`: OOXML string generation and ZIP packaging. It writes the document, relationships, styles, numbering, footnotes, settings, properties, content types, and media parts. Cross-references, equation sequence numbers, and the table of contents use Word fields with cached text.

`tests/` exists but is empty. All automated tests are inline `#[cfg(test)]` modules in the source files.

`examples/demo.tex` is the compact feature demonstration. It uses `refs.bib`, `pixel.png`, and `vector.pdf`; its checked-in `demo.aux` and `demo.docx` are example outputs, not automatically compared golden files. `examples/attention-is-all-you-need/` is a larger multi-file example with a custom `.sty`, local graphics, recursive `\input` files, and a checked-in `.docx`.

## Build and run commands

Use normal Cargo workflows from the repository root:

```sh
cargo build
cargo test
cargo fmt --check
```

Format changes with `cargo fmt`. Build an optimized local binary with:

```sh
cargo build --release
```

The resulting executable is `target/release/doclatex`; the debug executable is `target/debug/doclatex` after `cargo build` or `cargo run`. No exact Rust toolchain is pinned by the repository.

CLI usage is:

```text
doclatex <input.tex> [-o <output.docx>]
doclatex --help
doclatex --version
```

`--output` is the long form of `-o`. Without it, output is the input path with a `.docx` extension. Relative input and output paths are resolved against the caller's original working directory. The input and output must be different files.

To run the demo without modifying the tracked example outputs, copy its inputs to a temporary directory:

```sh
tmpdir=$(mktemp -d)
cp examples/demo.tex examples/refs.bib examples/pixel.png examples/vector.pdf "$tmpdir"/
cargo run -- "$tmpdir/demo.tex" --output "$tmpdir/result.docx"
# Inspect "$tmpdir/result.docx" and "$tmpdir/demo.aux" before removing the directory.
```

This demo includes a PDF figure, so it needs either `pdftocairo` or `mutool` on `PATH`. PNG, JPEG, and SVG handling does not invoke those converters. PDF conversion uses only the first page, tries `pdftocairo -svg` first, falls back to `mutool draw`, and uses a temporary SVG file.

## Runtime architecture and invariants

A compile is a single process with two full expansion/parse passes:

1. `main` reads the root `.tex` file, resolves input/output paths, and changes the process working directory to the input file's parent directory.
2. A fresh `State` and `Expander` expand the source. The expander lazily lexes source, processes user macros and primitives, loads local `.sty`/`.cls` files, and leaves parser-native commands in the token stream.
3. `parse_document` builds the first `Document`. The program writes the sibling `.aux`, then resolves bibliography entries.
4. The `.aux` is read back. A second fresh `State` expands and parses the original source with the first-pass labels available, while carrying forward the bibliography state. Bibliography resolution and `.aux` writing happen again.
5. `docx::write_docx` renders the second document and overwrites/creates the requested output. Warnings are sorted and deduplicated before being printed to stderr; undefined-reference warnings from the completed second pass are filtered out. Fatal errors are printed with a `doclatex:` prefix and exit status 1.

Preserve this two-pass behavior when changing numbering, labels, citations, or macro state. Do not accidentally reuse the first-pass `State` in the second pass.

File lookup is path-sensitive:

- The root input's directory becomes the process working directory.
- Nested `\input`, package, and class lookup considers the current source file's directory, the working directory, and colon-separated directories in `TEXINPUTS`.
- Graphics and bibliography paths are resolved in the job-directory context established by `main`.
- The `.aux` path is always the input path with its extension replaced by `.aux`, regardless of `-o`.

The DOCX writer emits Word fields marked dirty and enables `updateFields` in settings. Cached values are still written into the document. Visual verification in Word or another compatible editor may require updating fields, especially for the table of contents and page references.

## Development conventions

- Follow standard `rustfmt` formatting and the existing Rust naming conventions: `snake_case` functions/modules, `CamelCase` types, and uppercase constants.
- Match the existing module-level `//!` documentation and large section-comment style where useful. Most implementation code is intentionally direct and lightly commented.
- Keep the stages separate: tokenization in `token.rs`, expansion and mutable TeX state in `engine.rs`, document semantics in `parse.rs`, shared data types in `model.rs`, and Word packaging in `docx.rs`.
- A new parser-handled control sequence may need to be added to `is_native` in `engine.rs` so expansion does not consume or alter it. Add its semantics in `parse.rs`, reuse or extend model variants, and add corresponding OOXML rendering. Math-specific behavior belongs in `math.rs`.
- The expander and parser use token vectors and cursor positions extensively. When consuming, restoring, or splicing tokens, preserve group balance, source ordering, and unconsumed input.
- Nonfatal unsupported constructs and missing assets generally use `State::warn` and conversion continues. Reserve returned errors for failures that prevent the requested compile from completing. Keep warning messages consistent with the existing lowercase style.
- OOXML and OMML are assembled as strings. Escape dynamic text and attributes with the existing helpers, preserve namespace prefixes, and keep relationship IDs, content-type declarations, bookmark names, and media filenames synchronized with the parts actually written.
- Avoid introducing another XML/ZIP stack without a concrete need; the current writer and its tests assume direct string generation and `zip::ZipWriter`.

The current codebase compiles and tests successfully but emits existing compiler warnings, primarily for unused fields, variants, variables, and methods. Do not claim a warning-free build or make an unrelated warning cleanup part of a focused change. There is no configured `clippy -D warnings` gate.

## Testing instructions

Run the full suite after changes:

```sh
cargo test
cargo fmt --check
```

There are currently 50 unit tests, all in `src/main.rs`'s module tree. They cover lexer behavior, macro expansion, recursive class/package loading, counters, math OMML fragments, parser edge cases, image sizing, auxiliary data, bibliography formatting, and Word style selection.

Run a focused test by its full test name or a substring, for example:

```sh
cargo test engine::tests::def_with_args
cargo test math::tests::expanded_macros_preserve_math_structure
```

Add tests beside the module being changed, following the existing inline-test pattern. Test at the narrowest useful layer: token assertions for lexing, detokenized expansion for macros, `Document`/`Block`/`Inline` assertions for parsing, OMML substring/structure assertions for math, and OOXML helper or package assertions for Word output.

Unit tests are not sufficient for DOCX writer changes. Also compile a representative example and inspect the resulting archive and XML. At minimum, check that the ZIP is readable, XML parts are well-formed, expected document text/media are present, and relationship/content-type entries agree. Open the document in Word or a compatible editor when layout, fields, numbering, footnotes, or images change. The checked-in `.docx` files are not opened or validated by `cargo test`.

For tests that read or write files, use unique temporary directories and clean them up, as the engine tests do. Do not run examples in place unless intentionally updating their tracked `.aux` or `.docx` outputs.

## Security and file-safety considerations

- Conversion is not sandboxed. LaTeX input can cause local reads through `\input`, class/package loading, graphics, and bibliography paths. `TEXINPUTS` expands the search scope. Treat documents and custom style/class files as untrusted input when choosing what to compile.
- `filecontents` environments write files using the supplied name in the conversion context. Review path handling carefully before changing this behavior or running untrusted documents.
- The output file and sibling `.aux` are overwritten. Use temporary copies for smoke tests and never overwrite a user's input with the output; `main` explicitly rejects identical input/output paths.
- PDF conversion launches fixed executable names with separate argument arrays, not a shell. Preserve that separation and avoid interpolating document-controlled text into shell commands.
- Preserve XML escaping and anchor sanitization when adding text, links, bookmarks, relationships, or attributes. A syntactically valid ZIP alone does not prove that dynamic content is safely encoded.
- No network service, authentication code, telemetry, or secret-management flow is present in this repository.

## Release and deployment

There is no automated publishing or deployment process. Distribution is currently the Cargo-built native CLI. Use `cargo build --release` for a local optimized artifact. Do not assume CI checks, crates.io publication, installers, containers, or a server deployment exist unless they are added to the repository and documented here.
