mod aux;
mod bib;
mod docx;
mod engine;
mod math;
mod model;
mod parse;
mod token;

use aux::AuxData;
use bib::BibState;
use engine::{Expander, State};
use std::error::Error;
use std::path::{Path, PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("doclatex: {}", error);
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut verbose = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("doclatex — compile LaTeX directly to Microsoft Word\n\nUsage: doclatex <input.tex> [-o <output.docx>] [-v]\n\nOptions:\n  -o, --output <output.docx>  Set the output path\n  -v, --verbose              Show compilation progress\n  -h, --help                 Show this help\n  -V, --version              Show the version\n\nThe converter performs two passes and writes a .aux file beside the input.");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("doclatex {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-v" | "--verbose" => verbose = true,
            "-o" | "--output" => {
                output = Some(PathBuf::from(
                    args.next().ok_or("missing output path after -o")?,
                ));
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option: {}", arg).into()),
            _ if input.is_none() => input = Some(PathBuf::from(arg)),
            _ => return Err("only one input file may be specified".into()),
        }
    }
    let input = input.ok_or("usage: doclatex <input.tex> [-o <output.docx>] [-v]")?;
    let original_dir = std::env::current_dir()?;
    let input = absolute_path(&original_dir, &input);
    let output = match output {
        Some(path) => absolute_path(&original_dir, &path),
        None => input.with_extension("docx"),
    };
    if input.extension().and_then(|e| e.to_str()) == Some("docx") || input == output {
        return Err("input and output must be different files".into());
    }
    let source = std::fs::read_to_string(&input)
        .map_err(|e| format!("cannot read {}: {}", input.display(), e))?;
    let job_dir = input.parent().unwrap_or(Path::new(".")).to_path_buf();
    let aux_path = input.with_extension("aux");
    std::env::set_current_dir(&job_dir)?;

    progress(verbose, 1, "expanding and parsing source (first pass)");
    let mut first_state = State::new();
    first_state.jobname = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("doclatex")
        .to_string();
    let first_tokens = Expander::new(&mut first_state).expand_source(&source);
    let mut first_bib = BibState::new();
    let first = parse::parse_document(
        &mut first_state,
        first_tokens,
        &AuxData::default(),
        &mut first_bib,
    );
    progress(
        verbose,
        2,
        "resolving bibliography and writing auxiliary data",
    );
    AuxData::write(
        &aux_path,
        &first.doc.labels,
        &first_bib.cited,
        &first_bib.files,
        &first_bib.style_name(),
    );
    first_bib.resolve(Path::new("."));

    progress(verbose, 3, "expanding and parsing source (second pass)");
    let aux = AuxData::read(&aux_path);
    let mut second_state = State::new();
    second_state.jobname = first_state.jobname.clone();
    let second_tokens = Expander::new(&mut second_state).expand_source(&source);
    let mut second_bib = first_bib;
    let second = parse::parse_document(&mut second_state, second_tokens, &aux, &mut second_bib);
    second_bib.resolve(Path::new("."));
    AuxData::write(
        &aux_path,
        &second.doc.labels,
        &second_bib.cited,
        &second_bib.files,
        &second_bib.style_name(),
    );

    progress(verbose, 4, "writing DOCX output");
    docx::write_docx(&output, &second.doc)
        .map_err(|e| format!("cannot write {}: {}", output.display(), e))?;
    let mut warnings = second.doc.warnings.clone();
    warnings.retain(|w| !w.contains("undefined") || !w.contains("reference"));
    warnings.sort();
    warnings.dedup();
    for warning in warnings {
        eprintln!("warning: {}", warning);
    }
    eprintln!("wrote {}", output.display());
    Ok(())
}

fn progress(verbose: bool, step: u8, message: &str) {
    if verbose {
        eprintln!("doclatex: [{step}/4] {message}");
    }
}

fn absolute_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}
