//! The macro-expansion engine: TeX's "mouth".
//!
//! This is what "directly supports the LaTeX macro system": user definitions
//! (`\def`, `\newcommand`, `\NewDocumentCommand`, `\newenvironment`, ...) are
//! real token-level macros with undelimited/delimited/optional arguments,
//! fully expanded by an iterative expander. `\edef`-style full expansion,
//! `\expandafter`, `\csname`, the `\if...` family, `\let`, `\futurelet` and
//! LaTeX counters are all implemented.

use crate::token::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum Param {
    /// #1 — one token or one balanced group.
    Undelimited,
    /// #1<delim> — tokens up to the delimiter.
    Delimited(Vec<Token>),
    /// [default] — optional braced argument (also used for xparse o/O/d/D).
    OptionalGroup(Vec<Token>),
    /// s — star flag, replacement sees \BooleanTrue/\BooleanFalse.
    Star,
    /// v<char> — verbatim argument delimited by char.
    Verbatim(char),
}

#[derive(Clone, Debug)]
pub struct Macro {
    pub params: Vec<Param>,
    pub replacement: Vec<Token>,
    /// false => does not expand inside \edef (like \protected).
    pub expand_in_edef: bool,
    pub is_xparse: bool,
}

impl Macro {
    pub fn simple(replacement: Vec<Token>) -> Macro {
        Macro {
            params: vec![],
            replacement,
            expand_in_edef: true,
            is_xparse: false,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Meaning {
    Macro(Macro),
    /// `\let` alias: frozen copy of another meaning (or a character token).
    Let(Box<Meaning>),
    LetToken(Token),
    /// Handled natively by the engine or the parser; not expandable.
    Native,
}

#[derive(Clone, Debug)]
pub struct Counter {
    pub value: i64,
    pub within: Option<String>,
    /// Token template used by \the<counter>; contains expandable bits like
    /// \arabic{section}, so the value is always current at expansion time.
    pub the_tokens: Vec<Token>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClassKind {
    Article,
    Report,
    Book,
}

#[derive(Clone, Debug)]
pub struct GlossEntry {
    pub short: String,
    pub long: String,
    pub first: Option<String>,
    pub short_plural: Option<String>,
    pub long_plural: Option<String>,
    pub description: Option<String>,
    pub used: bool,
    /// Citation-style: print short form on first use.
    pub is_acronym: bool,
}

impl GlossEntry {
    pub fn plural_short(&self) -> String {
        self.short_plural
            .clone()
            .unwrap_or_else(|| format!("{}s", self.short))
    }
    pub fn plural_long(&self) -> String {
        self.long_plural
            .clone()
            .unwrap_or_else(|| format!("{}s", self.long))
    }
}

#[derive(Default, Debug)]
pub struct GlossStore {
    pub entries: HashMap<String, GlossEntry>,
}

#[derive(Clone, Debug)]
pub struct RefTarget {
    pub counter: String,
    /// Fully formatted number (e.g. "2.3"), what \ref prints.
    pub number: String,
    /// e.g. heading title or caption text, for \nameref.
    pub title: Option<String>,
}

#[derive(Debug)]
pub struct State {
    pub catcodes: CatCodeTable,
    pub defs: HashMap<String, Meaning>,
    pub actives: HashMap<char, Meaning>,
    pub counters: HashMap<String, Counter>,
    pub conds: HashMap<String, bool>,
    pub gloss: GlossStore,
    /// Target of the most recent \refstepcounter / heading.
    pub current_ref: Option<RefTarget>,
    /// Warnings collected during the run.
    pub warnings: Vec<String>,
    pub warned_undefined: std::collections::HashSet<String>,
    pub class: ClassKind,
    pub chinese: bool,
    pub font_size_pt: f64,
    pub paper: String, // "a4" | "letter"
    pub two_column: bool,
    pub jobname: String,
    /// hyperref loaded? (links colored)
    pub hyperref_loaded: bool,
    pub hidelinks: bool,
    /// Theorem-like environments from \newtheorem.
    /// env -> (thmname, Some(within) if numbered; Some("\0") = numbered w/o parent counter reset)
    pub theorems: HashMap<String, (String, Option<String>)>,
    /// User environments from \newenvironment.
    pub user_envs: HashMap<String, (usize, Option<Vec<Token>>, Vec<Token>, Vec<Token>)>,
    pub in_preamble: bool,
    /// subequations state: parent number
    pub subeq_parent: Option<i64>,
    /// active subequations: (parent number string, current letter)
    pub subeq_active: Option<(String, u8)>,
    /// appendix mode: sections lettered
    pub appendix: bool,
    /// \crefname overrides: counter -> (singular, plural)
    pub crefnames: HashMap<String, (String, String)>,
    /// approximation of math mode nesting, for \ifmmode
    pub math_depth: usize,
}

impl State {
    pub fn new() -> State {
        let mut st = State {
            catcodes: default_catcodes(),
            defs: HashMap::new(),
            actives: HashMap::new(),
            counters: HashMap::new(),
            conds: HashMap::new(),
            gloss: GlossStore::default(),
            current_ref: None,
            warnings: vec![],
            warned_undefined: std::collections::HashSet::new(),
            class: ClassKind::Article,
            chinese: false,
            font_size_pt: 12.0,
            paper: "letter".into(),
            two_column: false,
            jobname: "texput".into(),
            hyperref_loaded: false,
            hidelinks: false,
            theorems: HashMap::new(),
            user_envs: HashMap::new(),
            in_preamble: true,
            subeq_parent: None,
            subeq_active: None,
            appendix: false,
            crefnames: HashMap::new(),
            math_depth: 0,
        };
        st.actives.insert(
            '~',
            Meaning::Macro(Macro::simple(vec![Token::cs("nobreakspace")])),
        );
        install_kernel(&mut st);
        st
    }

    pub fn warn(&mut self, msg: impl Into<String>) {
        let m = msg.into();
        if !self.warnings.contains(&m) {
            self.warnings.push(m);
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&Meaning> {
        self.defs.get(name)
    }

    pub fn new_counter(&mut self, name: &str, within: Option<String>) {
        if self.counters.contains_key(name) {
            return;
        }
        let the_tokens = match within.clone() {
            Some(w) => {
                let mut v = self.the_tokens_for(&w);
                v.push(Token::Char('.', CatCode::Other));
                v.extend(counter_ref_tokens(name, "arabic"));
                v
            }
            None => counter_ref_tokens(name, "arabic"),
        };
        self.counters.insert(
            name.to_string(),
            Counter {
                value: 0,
                within,
                the_tokens,
            },
        );
    }

    /// Register \the<name> as an expandable macro using the template.
    fn the_tokens_for(&self, name: &str) -> Vec<Token> {
        self.counters
            .get(name)
            .map(|c| c.the_tokens.clone())
            .unwrap_or_default()
    }

    pub fn set_the_format(&mut self, name: &str, tokens: Vec<Token>) {
        if let Some(c) = self.counters.get_mut(name) {
            c.the_tokens = tokens;
        }
        self.defs.insert(
            format!("the{}", name),
            Meaning::Macro(Macro::simple(
                self.counters
                    .get(name)
                    .map(|c| c.the_tokens.clone())
                    .unwrap_or_default(),
            )),
        );
    }

    pub fn step_counter(&mut self, name: &str) {
        if self.counters.get(name).is_none() {
            self.new_counter(name, None);
        }
        if let Some(c) = self.counters.get_mut(name) {
            c.value += 1;
        }
        // reset children
        let names: Vec<String> = self.counters.keys().cloned().collect();
        for n in names {
            let within = self.counters.get(&n).and_then(|c| c.within.clone());
            if within.as_deref() == Some(name) {
                if let Some(c) = self.counters.get_mut(&n) {
                    c.value = 0;
                }
            }
        }
    }

    pub fn counter_value(&self, name: &str) -> i64 {
        self.counters.get(name).map(|c| c.value).unwrap_or(0)
    }

    /// Format counter with a style: arabic, roman, Roman, alph, Alph,
    /// chinese, fnsymbol.
    pub fn format_counter(&self, name: &str, style: &str) -> String {
        format_number(self.counter_value(name), style)
    }

    /// The value \ref prints (the fully formatted counter).
    pub fn the_counter(&self, name: &str) -> String {
        match self.counters.get(name) {
            Some(c) => expand_tokens_to_string(self, c.the_tokens.clone(), 0),
            None => String::new(),
        }
    }

    pub fn refstep(&mut self, counter: &str) {
        self.step_counter(counter);
        let number = self.the_counter(counter);
        self.current_ref = Some(RefTarget {
            counter: counter.into(),
            number,
            title: None,
        });
    }
}

pub fn counter_ref_tokens(counter: &str, style: &str) -> Vec<Token> {
    let mut v = vec![Token::cs(style)];
    v.push(Token::Char('{', CatCode::BGroup));
    v.extend(counter.chars().map(|c| Token::Char(c, CatCode::Letter)));
    v.push(Token::Char('}', CatCode::EGroup));
    v
}

/// Expand a token list (typically a `the_`-template) to a string, following
/// \the<counter> macros recursively.
pub fn expand_tokens_to_string(st: &State, toks: Vec<Token>, depth: usize) -> String {
    if depth > 8 {
        return detokenize(&toks);
    }
    let mut out = String::new();
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Token::Char(c, _) => {
                out.push(*c);
                i += 1;
            }
            Token::Space => {
                out.push(' ');
                i += 1;
            }
            Token::CtrlSeq(name) => {
                let style = [
                    "arabic", "roman", "Roman", "alph", "Alph", "chinese", "fnsymbol",
                ]
                .iter()
                .find(|s| **s == name)
                .copied();
                if let Some(style) = style {
                    if i + 1 < toks.len() && toks[i + 1].is_bgroup() {
                        let mut name_buf = String::new();
                        let mut j = i + 2;
                        while j < toks.len() && !toks[j].is_egroup() {
                            name_buf.push_str(&toks[j].to_text());
                            j += 1;
                        }
                        out.push_str(&st.format_counter(&name_buf, style));
                        i = j + 1;
                        continue;
                    }
                }
                if let Some(stripped) = name.strip_prefix("the") {
                    if let Some(c) = st.counters.get(stripped) {
                        out.push_str(&expand_tokens_to_string(
                            st,
                            c.the_tokens.clone(),
                            depth + 1,
                        ));
                        i += 1;
                        continue;
                    }
                }
                if let Some(Meaning::Macro(m)) = st.lookup(name) {
                    if m.params.is_empty() {
                        out.push_str(&expand_tokens_to_string(
                            st,
                            m.replacement.clone(),
                            depth + 1,
                        ));
                        i += 1;
                        continue;
                    }
                }
                i += 1;
            }
        }
    }
    out
}

pub fn format_number(v: i64, style: &str) -> String {
    match style {
        "arabic" => v.to_string(),
        "roman" => to_roman(v).to_lowercase(),
        "Roman" => to_roman(v),
        "alph" => to_alpha(v).to_lowercase(),
        "Alph" => to_alpha(v),
        "chinese" => to_chinese(v),
        "fnsymbol" => {
            const S: [&str; 9] = ["*", "†", "‡", "§", "¶", "‖", "**", "††", "‡‡"];
            S[((v - 1).rem_euclid(9)) as usize].to_string()
        }
        _ => v.to_string(),
    }
}

fn to_roman(mut v: i64) -> String {
    if v <= 0 {
        return v.to_string();
    }
    const T: [(i64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut s = String::new();
    for (n, r) in T {
        while v >= n {
            s.push_str(r);
            v -= n;
        }
    }
    s
}

fn to_alpha(mut v: i64) -> String {
    if v <= 0 {
        return v.to_string();
    }
    let mut s = String::new();
    while v > 0 {
        let d = ((v - 1) % 26) as u8;
        s.push((b'A' + d) as char);
        v = (v - 1) / 26;
    }
    s.chars().rev().collect()
}

fn to_chinese(v: i64) -> String {
    if v <= 0 {
        return v.to_string();
    }
    const D: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    const U: [&str; 4] = ["", "十", "百", "千"];
    if v < 10 {
        return D[v as usize].to_string();
    }
    if v < 100 {
        let t = v / 10;
        let o = v % 10;
        let mut s = String::new();
        if t > 1 {
            s.push_str(D[t as usize]);
        }
        s.push_str(U[1]);
        if o > 0 {
            s.push_str(D[o as usize]);
        }
        return s;
    }
    if v < 1000 {
        let h = v / 100;
        let r = v % 100;
        let mut s = D[h as usize].to_string() + U[2];
        if r > 0 {
            if r < 10 {
                s.push_str("零");
            }
            s.push_str(&to_chinese(r));
        }
        return s;
    }
    let k = v / 1000;
    let r = v % 1000;
    let mut s = D[k as usize].to_string() + U[3];
    if r > 0 {
        if r < 100 {
            s.push_str("零");
        }
        s.push_str(&to_chinese(r));
    }
    s
}

// ---------------------------------------------------------------------------
// Kernel installation: standard counters and formats per class.
// ---------------------------------------------------------------------------

fn install_counter(st: &mut State, name: &str, within: Option<&str>, the: Vec<Token>) {
    st.counters.insert(
        name.to_string(),
        Counter {
            value: 0,
            within: within.map(|s| s.to_string()),
            the_tokens: the.clone(),
        },
    );
    st.defs
        .insert(format!("the{}", name), Meaning::Macro(Macro::simple(the)));
}

pub fn install_kernel(st: &mut State) {
    install_counter(st, "part", None, counter_ref_tokens("part", "Roman"));
    if st.class != ClassKind::Article {
        install_counter(
            st,
            "chapter",
            None,
            if st.chinese {
                counter_ref_tokens("chapter", "chinese")
            } else {
                counter_ref_tokens("chapter", "arabic")
            },
        );
        install_counter(st, "section", Some("chapter"), {
            let mut v = vec![Token::cs("thechapter"), Token::Char('.', CatCode::Other)];
            v.extend(counter_ref_tokens("section", "arabic"));
            v
        });
    } else {
        install_counter(st, "section", None, counter_ref_tokens("section", "arabic"));
    }
    install_counter(st, "subsection", Some("section"), {
        let mut v = vec![Token::cs("thesection"), Token::Char('.', CatCode::Other)];
        v.extend(counter_ref_tokens("subsection", "arabic"));
        v
    });
    install_counter(st, "subsubsection", Some("subsection"), {
        let mut v = vec![Token::cs("thesubsection"), Token::Char('.', CatCode::Other)];
        v.extend(counter_ref_tokens("subsubsection", "arabic"));
        v
    });
    install_counter(st, "paragraph", Some("subsubsection"), {
        let mut v = vec![
            Token::cs("thesubsubsection"),
            Token::Char('.', CatCode::Other),
        ];
        v.extend(counter_ref_tokens("paragraph", "arabic"));
        v
    });
    install_counter(st, "subparagraph", Some("paragraph"), {
        let mut v = vec![Token::cs("theparagraph"), Token::Char('.', CatCode::Other)];
        v.extend(counter_ref_tokens("subparagraph", "arabic"));
        v
    });

    let chained = |first: &str| {
        let mut v = vec![Token::cs(first), Token::Char('.', CatCode::Other)];
        v.extend(counter_ref_tokens(
            if first == "thechapter" {
                "figure"
            } else {
                "figure"
            },
            "arabic",
        ));
        v
    };
    let _ = chained;
    let fig_the = if st.class == ClassKind::Article {
        counter_ref_tokens("figure", "arabic")
    } else {
        let mut v = vec![Token::cs("thechapter"), Token::Char('.', CatCode::Other)];
        v.extend(counter_ref_tokens("figure", "arabic"));
        v
    };
    install_counter(st, "figure", None, fig_the);
    install_counter(
        st,
        "table",
        None,
        if st.class == ClassKind::Article {
            counter_ref_tokens("table", "arabic")
        } else {
            let mut v = vec![Token::cs("thechapter"), Token::Char('.', CatCode::Other)];
            v.extend(counter_ref_tokens("table", "arabic"));
            v
        },
    );
    let eq_the = if st.class == ClassKind::Article {
        counter_ref_tokens("equation", "arabic")
    } else {
        let mut v = vec![Token::cs("thechapter"), Token::Char('.', CatCode::Other)];
        v.extend(counter_ref_tokens("equation", "arabic"));
        v
    };
    install_counter(st, "equation", None, eq_the);
    install_counter(
        st,
        "footnote",
        None,
        counter_ref_tokens("footnote", "arabic"),
    );
    install_counter(st, "page", None, counter_ref_tokens("page", "arabic"));
    let levels = ["i", "ii", "iii", "iv"];
    let styles = ["arabic", "alph", "roman", "Alph"];
    for l in 0..4 {
        let parent: Vec<Token> = if l == 0 {
            vec![]
        } else {
            let mut v = vec![Token::cs(&format!("theenum{}", levels[l - 1]))];
            v.push(Token::Char('.', CatCode::Other));
            v
        };
        let mut v = parent;
        v.extend(counter_ref_tokens(&format!("enum{}", levels[l]), styles[l]));
        let enum_name = format!("enum{}", levels[l]);
        let within_name = if l == 0 {
            None
        } else {
            Some(format!("enum{}", levels[l - 1]))
        };
        install_counter(st, &enum_name, within_name.as_deref(), v);
    }
    install_counter(
        st,
        "mpfootnote",
        None,
        counter_ref_tokens("mpfootnote", "arabic"),
    );
}

// ---------------------------------------------------------------------------
// Native command table: commands the parser (or engine) handles directly and
// that must survive expansion untouched.
// ---------------------------------------------------------------------------

pub fn is_native(name: &str) -> bool {
    matches!(
        name,
        // document
        "documentclass" | "begin" | "end" | "title" | "author" | "date" | "thanks"
        | "maketitle" | "tableofcontents" | "listoffigures" | "listoftables" | "appendix"
        | "frontmatter" | "mainmatter" | "backmatter" | "linespread" | "geometry"
        | "hypersetup" | "pagestyle" | "thispagestyle" | "pagenumbering" | "setlength"
        | "addtolength" | "addcontentsline" | "addtocontents" | "markboth" | "markright"
        | "selectlanguage" | "otherlanguage" | "setmainfont" | "setsansfont" | "setmonofont"
        | "setCJKmainfont" | "today" | "index" | "glossary" | "shorttoc" | "savegeometry"
        | "loadgeometry" | "usepackage" | "RequirePackage" | "LoadClass" | "ExecuteOptions"
        | "ProcessOptions" | "DeclareOption" | "CurrentOption" | "PassOptionsToPackage"
        | "baselinestretch" | "flushbottom" | "raggedbottom" | "sloppy" | "fussy"
        | "onecolumn" | "twocolumn" | "twoside" | "oneside" | "openright" | "openany"
        | "cleardoublepage" | "suppressfloats" | "displaybreak" | "allowdisplaybreaks"
        | "errorcontextlines" | "stop" | "stopmode"
        // sections
        | "part" | "chapter" | "section" | "subsection" | "subsubsection" | "paragraph"
        | "subparagraph" | "minisec" | "addsec"
        // references
        | "label" | "ref" | "eqref" | "pageref" | "autoref" | "cref" | "Cref" | "nameref"
        | "crefrange" | "Crefrange" | "labelcref" | "thref"
        // citations
        | "cite" | "citet" | "citep" | "citealp" | "citealt" | "citeauthor" | "citeyear"
        | "citeyearpar" | "citeonline" | "nocite" | "bibpunct" | "bibliography"
        | "bibliographystyle" | "bibitem" | "citestyle"
        // glossary / acronyms
        | "newacronym" | "newglossaryentry" | "gls" | "Gls" | "glspl" | "Glspl" | "glslink"
        | "acrshort" | "acrlong" | "acrfull" | "Acrshort" | "Acrlong" | "Acrfull" | "ac" | "acp"
        | "Ac" | "Acp" | "acs" | "acl" | "acf" | "printglossaries" | "printglossary"
        | "printacronyms" | "makeglossaries" | "makeindex" | "glsadd" | "glsreset"
        | "glsunset" | "acrshortpl" | "acrlongpl" | "acrfullpl"
        // inline text & fonts
        | "textbf" | "textit" | "textsl" | "textsc" | "textsf" | "textrm" | "texttt"
        | "textmd" | "textup" | "emph" | "underline" | "overline" | "sout" | "st" | "uwave"
        | "textcolor" | "color" | "colorbox" | "fcolorbox" | "rmfamily" | "sffamily"
        | "ttfamily" | "bfseries" | "mdseries" | "itshape" | "slshape" | "scshape" | "upshape"
        | "tiny" | "scriptsize" | "footnotesize" | "small" | "normalsize" | "large" | "Large"
        | "LARGE" | "huge" | "Huge" | "centering" | "raggedright" | "raggedleft" | "quad"
        | "qquad" | "hspace" | "vspace" | "smallskip" | "medskip" | "bigskip" | "noindent"
        | "indent" | "par" | "newline" | "linebreak" | "nolinebreak" | "pagebreak"
        | "nopagebreak" | "dotfill" | "hrulefill" | "hfill" | "vfill" | "stretch" | "space"
        | "nobreakspace" | "textasciitilde" | "textasciicircum" | "textbackslash"
        | "textbar" | "textquotedbl" | "textless" | "textgreater" | "textquotesingle"
        | "textemdash" | "textendash" | "textexclamdown" | "textquestiondown" | "textsterling"
        | "textcurrency" | "S" | "P" | "copyright" | "textcopyright" | "textregistered"
        | "texttrademark" | "textbullet" | "dag" | "ddag" | "dots" | "ldots" | "textellipsis"
        | "textperiodcentered" | "ae" | "AE" | "oe" | "OE" | "aa" | "AA" | "o" | "O" | "l"
        | "L" | "ss" | "i" | "j" | "lq" | "rq" | "deg" | "textdegree" | "ensuremath"
        | "sb" | "sp" | "textsubscript" | "textsuperscript" | "oldstylenums"
        | "text" | "mbox" | "makebox" | "fbox" | "framebox" | "raisebox" | "smash" | "phantom"
        | "hphantom" | "vphantom" | "mathstrut" | "strut" | "cong"
        // lists
        | "item" | "verb" | "lstinline"
        // floats & includes
        | "caption" | "includegraphics" | "footnote" | "footnotemark" | "footnotetext"
        | "url" | "href" | "hyperref" | "nolinkurl" | "path" | "email" | "marginpar"
        | "captionof" | "captionsetup" | "lstset" | "lstinputlisting" | "ContinuedFloat"
        // math delimiters as commands
        | "(" | ")" | "[" | "]"
    ) || name.chars().all(|c| !c.is_ascii_alphabetic())
}

// ---------------------------------------------------------------------------
// Token cursor (shared by engine and parser)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Cursor {
    pub toks: Vec<Token>,
    pub pos: usize,
}

impl Cursor {
    pub fn new(toks: Vec<Token>) -> Cursor {
        Cursor { toks, pos: 0 }
    }
    pub fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }
    pub fn peek_at(&self, k: usize) -> Option<&Token> {
        self.toks.get(self.pos + k)
    }
    pub fn next(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
    pub fn at_end(&self) -> bool {
        self.pos >= self.toks.len()
    }
    pub fn rest(&self) -> Vec<Token> {
        self.toks[self.pos..].to_vec()
    }
    pub fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(Token::Space)) {
            self.pos += 1;
        }
    }
    /// Read a braced group (or single token); returns inner tokens.
    pub fn read_group(&mut self) -> Vec<Token> {
        self.skip_spaces();
        match self.next() {
            Some(t) if t.is_bgroup() => {
                let mut depth = 1;
                let mut out = vec![];
                while let Some(t) = self.next() {
                    if t.is_bgroup() {
                        depth += 1;
                    } else if t.is_egroup() {
                        depth -= 1;
                        if depth == 0 {
                            return out;
                        }
                    }
                    out.push(t);
                }
                out
            }
            Some(t) => vec![t],
            None => vec![],
        }
    }
    /// Read an optional [..] group; returns contents if present.
    pub fn read_optional(&mut self) -> Option<Vec<Token>> {
        self.skip_spaces();
        if matches!(self.peek(), Some(Token::Char('[', CatCode::Other))) {
            self.pos += 1;
            let mut depth = 1;
            let mut out = vec![];
            while let Some(t) = self.next() {
                match t {
                    Token::Char('[', CatCode::Other) => depth += 1,
                    Token::Char(']', CatCode::Other) => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(out);
                        }
                    }
                    _ => {}
                }
                out.push(t);
            }
            Some(out)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Integer / dimension scanning
// ---------------------------------------------------------------------------

/// Scan an integer expression (supports + - * / ( ) and \value{c}).
pub fn scan_int(st: &State, cur: &mut Cursor) -> i64 {
    fn expr(st: &State, cur: &mut Cursor) -> i64 {
        let mut v = term(st, cur);
        loop {
            cur.skip_spaces();
            match cur.peek() {
                Some(Token::Char('+', _)) => {
                    cur.pos += 1;
                    v = v.wrapping_add(term(st, cur));
                }
                Some(Token::Char('-', _)) => {
                    cur.pos += 1;
                    v = v.wrapping_sub(term(st, cur));
                }
                _ => break,
            }
        }
        v
    }
    fn term(st: &State, cur: &mut Cursor) -> i64 {
        let mut v = factor(st, cur);
        loop {
            cur.skip_spaces();
            match cur.peek() {
                Some(Token::Char('*', _)) => {
                    cur.pos += 1;
                    v = v.wrapping_mul(factor(st, cur));
                }
                Some(Token::Char('/', _)) => {
                    cur.pos += 1;
                    let d = factor(st, cur);
                    if d != 0 {
                        v /= d;
                    }
                }
                _ => break,
            }
        }
        v
    }
    fn factor(st: &State, cur: &mut Cursor) -> i64 {
        cur.skip_spaces();
        match cur.next() {
            Some(Token::Char('(', _)) => {
                let v = expr(st, cur);
                cur.skip_spaces();
                if matches!(cur.peek(), Some(Token::Char(')', _))) {
                    cur.pos += 1;
                }
                v
            }
            Some(Token::Char('-', _)) => -factor(st, cur),
            Some(Token::Char('+', _)) => factor(st, cur),
            Some(Token::CtrlSeq(name)) => match name.as_str() {
                "value" => {
                    let g = cur.read_group();
                    st.counter_value(&detokenize(&g))
                }
                "numexpr" => {
                    let v = scan_int(st, cur);
                    cur.skip_spaces();
                    if matches!(cur.peek(), Some(Token::CtrlSeq(n)) if n == "relax") {
                        cur.pos += 1;
                    }
                    v
                }
                _ => {
                    if let Some(Meaning::Macro(m)) = st.lookup(&name) {
                        if m.params.is_empty() {
                            let mut c2 = Cursor::new(m.replacement.clone());
                            return expr(st, &mut c2);
                        }
                    }
                    0
                }
            },
            Some(Token::Char(c, _)) if c.is_ascii_digit() => {
                let mut s = String::new();
                s.push(c);
                while let Some(Token::Char(d, _)) = cur.peek() {
                    if d.is_ascii_digit() {
                        s.push(*d);
                        cur.pos += 1;
                    } else {
                        break;
                    }
                }
                s.parse().unwrap_or(0)
            }
            Some(Token::Char('`', _)) => match cur.next() {
                Some(Token::Char(c, _)) => c as i64,
                _ => 0,
            },
            _ => 0,
        }
    }
    expr(st, cur)
}

/// Convert tokens to a plain string (chars + space tokens).
pub fn detokenize(toks: &[Token]) -> String {
    let mut s = String::new();
    for t in toks {
        match t {
            Token::Char(c, _) => s.push(*c),
            Token::Space => s.push(' '),
            Token::CtrlSeq(n) => {
                if n == " " {
                    s.push(' ');
                } else {
                    s.push('\\');
                    s.push_str(n);
                }
            }
        }
    }
    s
}

/// Scan a dimension, return value in points.
pub fn scan_dimen(st: &State, cur: &mut Cursor) -> f64 {
    cur.skip_spaces();
    let v = scan_number_f64(st, cur);
    cur.skip_spaces();
    let mut unit = String::new();
    while let Some(Token::Char(c, CatCode::Letter)) = cur.peek() {
        unit.push(*c);
        cur.pos += 1;
    }
    let pt = match unit.as_str() {
        "" | "pt" => v,
        "pc" => v * 12.0,
        "in" => v * 72.27,
        "bp" => v * (72.27 / 72.0),
        "cm" => v * (72.27 / 2.54),
        "mm" => v * (72.27 / 25.4),
        "em" => v * 10.0,
        "ex" => v * 4.305,
        "dd" => v * 1.070,
        "cc" => v * 12.84,
        "sp" => v / 65536.0,
        _ => v,
    };
    pt
}

fn scan_number_f64(st: &State, cur: &mut Cursor) -> f64 {
    cur.skip_spaces();
    let mut s = String::new();
    let mut seen_dot = false;
    while let Some(t) = cur.peek() {
        match t {
            Token::Char(c, _) if c.is_ascii_digit() => {
                s.push(*c);
                cur.pos += 1;
            }
            Token::Char('.', _) if !seen_dot => {
                seen_dot = true;
                s.push('.');
                cur.pos += 1;
            }
            Token::Char('-', _) if s.is_empty() => {
                s.push('-');
                cur.pos += 1;
            }
            Token::Char('+', _) if s.is_empty() => {
                cur.pos += 1;
            }
            _ => break,
        }
    }
    if s.is_empty() || s == "-" {
        if let Some(Token::CtrlSeq(name)) = cur.peek().cloned() {
            if name == "value" {
                cur.pos += 1;
                let g = cur.read_group();
                return st.counter_value(&detokenize(&g)) as f64;
            }
        }
        return 0.0;
    }
    s.parse().unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// The expander
// ---------------------------------------------------------------------------

/// Items of the expander's input: token lists (macro expansions etc.) and
/// lazy line-lexer sources (files). Sources are tokenized on demand so that
/// catcode changes apply to not-yet-read characters.
pub enum InItem {
    /// Reversed token vector: pop() yields the front.
    Toks(Vec<Token>),
    Lines {
        lexer: Lexer,
        directory: Option<PathBuf>,
    },
}

pub struct InputStack {
    items: Vec<InItem>,
}

impl InputStack {
    pub fn new(toks: Vec<Token>) -> InputStack {
        let mut t = toks;
        t.reverse();
        InputStack {
            items: vec![InItem::Toks(t)],
        }
    }
    pub fn with_source(src: &str) -> InputStack {
        Self::with_source_at(src, None)
    }
    fn with_source_at(src: &str, directory: Option<PathBuf>) -> InputStack {
        InputStack {
            items: vec![InItem::Lines {
                lexer: Lexer::new(src),
                directory,
            }],
        }
    }
    pub fn pop_tok(&mut self, cats: &CatCodeTable) -> Option<Token> {
        loop {
            match self.items.last_mut() {
                Some(InItem::Toks(v)) => {
                    if let Some(t) = v.pop() {
                        return Some(t);
                    }
                    self.items.pop();
                }
                Some(InItem::Lines { lexer, .. }) => {
                    if let Some(t) = lexer.next_tok(cats) {
                        return Some(t);
                    }
                    self.items.pop();
                }
                None => return None,
            }
        }
    }
    pub fn peek_tok(&mut self, cats: &CatCodeTable) -> Option<Token> {
        let t = self.pop_tok(cats)?;
        self.push(t.clone());
        Some(t)
    }
    fn top_toks(&mut self) -> &mut Vec<Token> {
        let need_new = !matches!(self.items.last(), Some(InItem::Toks(_)));
        if need_new {
            self.items.push(InItem::Toks(vec![]));
        }
        match self.items.last_mut() {
            Some(InItem::Toks(v)) => v,
            _ => unreachable!(),
        }
    }
    pub fn push(&mut self, t: Token) {
        self.top_toks().push(t);
    }
    pub fn push_all(&mut self, toks: &[Token]) {
        for t in toks.iter().rev() {
            self.top_toks().push(t.clone());
        }
    }
    pub fn push_source(&mut self, src: &str, directory: Option<PathBuf>) {
        self.items.push(InItem::Lines {
            lexer: Lexer::new(src),
            directory,
        });
    }
    fn current_directory(&self) -> Option<&Path> {
        self.items.iter().rev().find_map(|item| match item {
            InItem::Lines {
                directory: Some(directory),
                ..
            } => Some(directory.as_path()),
            _ => None,
        })
    }
    /// Read a raw line from the top source (verbatim support).
    pub fn read_raw_line(&mut self) -> Option<String> {
        match self.items.last_mut() {
            Some(InItem::Lines { lexer, .. }) => lexer.read_raw_line(),
            _ => None,
        }
    }
    pub fn read_raw_char(&mut self) -> Option<char> {
        match self.items.last_mut() {
            Some(InItem::Lines { lexer, .. }) => lexer.read_raw_char(),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum ExpandMode {
    Normal,
    /// Inside \edef: protected macros and natives are kept as tokens.
    Edef,
}

#[derive(Clone, Copy, PartialEq)]
enum CondState {
    /// We are in the TRUE branch of a conditional; a later \else must skip to \fi.
    TrueBranch,
    /// We are in the FALSE branch (after \else); a later \fi ends it.
    FalseBranch,
}

pub struct Expander<'a> {
    pub st: &'a mut State,
    cond_stack: Vec<CondState>,
    loaded_packages: HashSet<PathBuf>,
    source_directories: Vec<PathBuf>,
}

/// Drain input stack into a cursor (caller MUST restack the remainder).
fn cursor_from_stack(input: &mut InputStack, cats: &CatCodeTable) -> Cursor {
    let mut toks = vec![];
    while let Some(t) = input.pop_tok(cats) {
        toks.push(t);
    }
    Cursor::new(toks)
}

fn restack(input: &mut InputStack, cur: &Cursor) {
    let rest = cur.rest();
    input.push_all(&rest);
}

impl<'a> Expander<'a> {
    pub fn new(st: &'a mut State) -> Expander<'a> {
        Expander {
            st,
            cond_stack: vec![],
            loaded_packages: HashSet::new(),
            source_directories: vec![],
        }
    }

    #[inline]
    fn cats(&self) -> &CatCodeTable {
        &self.st.catcodes
    }

    /// Expand a whole document source (lazy line lexing with live catcodes).
    pub fn expand_source(&mut self, src: &str) -> Vec<Token> {
        let mut input = InputStack::with_source(src);
        self.expand_loop(&mut input, ExpandMode::Normal)
    }

    /// Expand user macros throughout `toks`; native commands and their
    /// arguments are passed through untouched for the parser.
    pub fn expand_all(&mut self, toks: Vec<Token>) -> Vec<Token> {
        self.expand_mode(toks, ExpandMode::Normal)
    }

    pub fn expand_mode(&mut self, toks: Vec<Token>, mode: ExpandMode) -> Vec<Token> {
        let mut input = InputStack::new(toks);
        self.expand_loop(&mut input, mode)
    }

    fn expand_loop(&mut self, input: &mut InputStack, mode: ExpandMode) -> Vec<Token> {
        let mut out: Vec<Token> = Vec::new();
        let mut inhibit_next = false;
        while let Some(t) = input.pop_tok(self.cats()) {
            match t.clone() {
                Token::Char(c, cat) if cat == CatCode::Active => {
                    if inhibit_next {
                        out.push(t);
                        inhibit_next = false;
                        continue;
                    }
                    match self.st.actives.get(&c).cloned() {
                        Some(Meaning::Macro(m)) => {
                            let repl = m.replacement.clone();
                            input.push_all(&repl);
                        }
                        _ => out.push(t),
                    }
                }
                Token::Char(_, cat)
                    if matches!(
                        cat,
                        CatCode::BGroup
                            | CatCode::EGroup
                            | CatCode::MathShift
                            | CatCode::Alignment
                            | CatCode::Subscript
                            | CatCode::Superscript
                    ) =>
                {
                    out.push(t);
                }
                Token::Char(c, cat) => {
                    let _ = cat;
                    out.push(Token::Char(c, CatCode::Other));
                }
                Token::Space => out.push(t),
                Token::CtrlSeq(name) => {
                    if inhibit_next {
                        out.push(t);
                        inhibit_next = false;
                        continue;
                    }
                    self.expand_cs(input, &mut out, &name, mode, &mut inhibit_next);
                }
            }
        }
        out
    }

    fn expand_cs(
        &mut self,
        input: &mut InputStack,
        out: &mut Vec<Token>,
        name: &str,
        mode: ExpandMode,
        inhibit_next: &mut bool,
    ) {
        // Engine-level primitives first.
        match name {
            "relax" | "protect" | "protect@" | "@protect" | "empty" | "@empty" | " @"
            | "space@" | "newline@" => return,
            "expandafter" => {
                let saved = match input.pop_tok(self.cats()) {
                    Some(t) => t,
                    None => return,
                };
                let second = match input.pop_tok(self.cats()) {
                    Some(t) => t,
                    None => {
                        out.push(saved);
                        return;
                    }
                };
                // Expand `second` ONCE and put its expansion back on the
                // input BEFORE `saved` — TeX semantics.
                self.expand_once(input, second, mode);
                input.push(saved);
                return;
            }
            "noexpand" => {
                *inhibit_next = true;
                return;
            }
            "csname" => {
                // The constructed control sequence must be expanded by the
                // main loop, so feed it back to the input, not the output.
                let mut tmp: Vec<Token> = vec![];
                self.do_csname(input, &mut tmp);
                input.push_all(&tmp);
                return;
            }
            "string" => {
                if let Some(t) = input.pop_tok(self.cats()) {
                    if let Token::CtrlSeq(n) = t {
                        for c in n.chars() {
                            out.push(Token::Char(c, CatCode::Other));
                        }
                    } else {
                        out.push(t);
                    }
                }
                return;
            }
            "meaning" => {
                if let Some(Token::CtrlSeq(n)) = input.pop_tok(self.cats()) {
                    let s = match self.st.defs.get(&n) {
                        Some(Meaning::Macro(m)) => format!("macro:{}", detokenize(&m.replacement)),
                        Some(_) => format!("{}: native", n),
                        None => format!("{}: undefined", n),
                    };
                    for c in s.chars() {
                        out.push(Token::Char(c, CatCode::Other));
                    }
                }
                return;
            }
            "the" => {
                self.do_the(input, out);
                return;
            }
            "number" => {
                let mut cur = cursor_from_stack(input, self.cats());
                let v = scan_int(self.st, &mut cur);
                restack(input, &cur);
                for c in v.to_string().chars() {
                    out.push(Token::Char(c, CatCode::Other));
                }
                return;
            }
            "romannumeral" => {
                let mut cur = cursor_from_stack(input, self.cats());
                let v = scan_int(self.st, &mut cur);
                restack(input, &cur);
                let s = to_roman(v).to_lowercase();
                for c in s.chars() {
                    out.push(Token::Char(c, CatCode::Other));
                }
                return;
            }
            "arabic" | "roman" | "Roman" | "alph" | "Alph" | "chinese" | "fnsymbol" => {
                let style = name.to_string();
                let mut cur = cursor_from_stack(input, self.cats());
                let g = cur.read_group();
                let cname = detokenize(&g);
                restack(input, &cur);
                let s = self.st.format_counter(&cname, &style);
                for c in s.chars() {
                    out.push(Token::Char(c, CatCode::Other));
                }
                return;
            }
            "value" => {
                let mut cur = cursor_from_stack(input, self.cats());
                let g = cur.read_group();
                let cname = detokenize(&g);
                restack(input, &cur);
                let v = self.st.counter_value(&cname);
                for c in v.to_string().chars() {
                    out.push(Token::Char(c, CatCode::Other));
                }
                return;
            }
            "iftrue" | "iffalse" => {
                let cond = name == "iftrue";
                self.start_conditional(input, cond);
                return;
            }
            "ifmmode" => {
                let cond = self.st.math_depth > 0;
                self.start_conditional(input, cond);
                return;
            }
            "ifhmode" | "ifvmode" | "ifinner" | "ifcat" => {
                let cond = name == "ifhmode" && !self.st.in_preamble;
                self.start_conditional(input, cond);
                return;
            }
            "ifnum" => {
                let mut cur = cursor_from_stack(input, self.cats());
                let a = scan_int(self.st, &mut cur);
                cur.skip_spaces();
                let op = match cur.next() {
                    Some(Token::Char('<', _)) => {
                        if matches!(cur.peek(), Some(Token::Char('=', _))) {
                            cur.pos += 1;
                            1
                        } else {
                            0
                        }
                    }
                    Some(Token::Char('>', _)) => {
                        if matches!(cur.peek(), Some(Token::Char('=', _))) {
                            cur.pos += 1;
                            3
                        } else {
                            2
                        }
                    }
                    Some(Token::Char('=', _)) => 4,
                    _ => 4,
                };
                let b = scan_int(self.st, &mut cur);
                restack(input, &cur);
                let cond = match op {
                    0 => a < b,
                    1 => a <= b,
                    2 => a > b,
                    3 => a >= b,
                    _ => a == b,
                };
                self.start_conditional(input, cond);
                return;
            }
            "ifodd" => {
                let mut cur = cursor_from_stack(input, self.cats());
                let v = scan_int(self.st, &mut cur);
                restack(input, &cur);
                self.start_conditional(input, v % 2 != 0);
                return;
            }
            "ifx" => {
                let a = input.pop_tok(self.cats());
                let b = input.pop_tok(self.cats());
                let cond = match (&a, &b) {
                    (Some(Token::CtrlSeq(x)), Some(Token::CtrlSeq(y))) => {
                        if x == y {
                            true
                        } else {
                            match (self.st.defs.get(x), self.st.defs.get(y)) {
                                (None, None) => false,
                                (Some(mx), Some(my)) => {
                                    std::mem::discriminant(mx) == std::mem::discriminant(my)
                                        && format!("{:?}", mx) == format!("{:?}", my)
                                }
                                _ => false,
                            }
                        }
                    }
                    (Some(x), Some(y)) => x == y,
                    _ => false,
                };
                self.start_conditional(input, cond);
                return;
            }
            "ifdim" => {
                let mut cur = cursor_from_stack(input, self.cats());
                let a = scan_dimen(self.st, &mut cur);
                cur.skip_spaces();
                let op = match cur.next() {
                    Some(Token::Char('<', _)) => 0,
                    Some(Token::Char('>', _)) => 2,
                    _ => 4,
                };
                let b = scan_dimen(self.st, &mut cur);
                restack(input, &cur);
                self.start_conditional(
                    input,
                    match op {
                        0 => a < b,
                        2 => a > b,
                        _ => (a - b).abs() < 0.01,
                    },
                );
                return;
            }
            "ifundefined" => {
                let mut cur = cursor_from_stack(input, self.cats());
                let g = cur.read_group();
                let nm = detokenize(&g);
                restack(input, &cur);
                let cond = !self.st.defs.contains_key(&nm) && !is_native(&nm);
                self.start_conditional(input, cond);
                return;
            }
            "else" => {
                // Belongs to the innermost open conditional.
                match self.cond_stack.last().copied() {
                    Some(CondState::TrueBranch) => {
                        // true branch done: skip false branch to \fi
                        self.cond_stack.pop();
                        self.skip_to_fi(input);
                    }
                    Some(CondState::FalseBranch) => {
                        // stray \else inside false branch: skip to \fi
                        self.cond_stack.pop();
                        self.skip_to_fi(input);
                    }
                    None => {}
                }
                return;
            }
            "fi" => {
                self.cond_stack.pop();
                return;
            }
            _ => {}
        }

        // \newif conditionals and their <name>true/<name>false setters.
        if let Some(stripped) = name.strip_prefix("if") {
            if let Some(&v) = self.st.conds.get(stripped) {
                self.start_conditional(input, v);
                return;
            }
        }
        if let Some(stem) = name.strip_suffix("true") {
            if self.st.conds.contains_key(stem) {
                self.st.conds.insert(stem.to_string(), true);
                return;
            }
        }
        if let Some(stem) = name.strip_suffix("false") {
            if self.st.conds.contains_key(stem) {
                self.st.conds.insert(stem.to_string(), false);
                return;
            }
        }

        if name == "usepackage" || name == "RequirePackage" {
            self.do_usepackage(input, out, name);
            return;
        }
        if name == "ProvidesPackage" || name == "NeedsTeXFormat" {
            let _ = self.scan_undelimited(input);
            if matches!(self.take_spaces_and_peek(input), Some(Token::Char('[', _))) {
                let _ = self.scan_bracket(input);
            }
            return;
        }
        // \def, \newcommand, ...
        if self.try_define(input, name) {
            return;
        }

        if name == "let" {
            self.do_let(input);
            return;
        }
        if name == "futurelet" {
            self.do_futurelet(input);
            return;
        }
        if name == "catcode" {
            self.do_catcode(input);
            return;
        }
        if name == "makeatletter" {
            self.st.catcodes.insert('@', 11);
            return;
        }
        if name == "makeatother" {
            self.st.catcodes.insert('@', 12);
            return;
        }
        if name == "newif" {
            self.do_newif(input);
            return;
        }
        if name == "newcounter" || name == "@addtoreset" {
            self.do_newcounter(input);
            return;
        }
        if name == "setcounter" {
            self.do_setcounter(input, false);
            return;
        }
        if name == "addtocounter" {
            self.do_setcounter(input, true);
            return;
        }
        if name == "stepcounter" {
            self.do_stepcounter(input, false);
            return;
        }
        if name == "refstepcounter" {
            self.do_stepcounter(input, true);
            return;
        }
        if name == "newenvironment" || name == "renewenvironment" {
            self.do_newenvironment(input);
            return;
        }
        if name == "newtheorem" {
            self.do_newtheorem(input);
            return;
        }
        if name == "newtoks" {
            if let Some(Token::CtrlSeq(nm)) = input.pop_tok(self.cats()) {
                self.st
                    .defs
                    .insert(nm, Meaning::Macro(Macro::simple(vec![])));
            }
            return;
        }
        if name == "global" || name == "long" || name == "outer" {
            if let Some(t) = input.pop_tok(self.cats()) {
                self.expand_once(input, t, mode);
            }
            return;
        }
        if name == "afterassignment" || name == "aftergroup" {
            let _ = input.pop_tok(self.cats());
            return;
        }
        if name == "input" || name == "include" || name == "@input" {
            self.do_input(input);
            return;
        }
        if name == "InputIfFileExists" {
            // \InputIfFileExists{file}{true}{false}
            let mut cur = cursor_from_stack(input, self.cats());
            let g = cur.read_group();
            let t_branch = cur.read_group();
            let _f_branch = cur.read_group();
            restack(input, &cur);
            let name = detokenize(&g);
            let base = input
                .current_directory()
                .map(Path::to_path_buf)
                .or_else(|| self.source_directories.last().cloned());
            if find_input_file_at(self.st, &name, base.as_deref(), "tex").is_some() {
                let expanded = self.expand_all(t_branch);
                input.push_all(&expanded);
                self.do_input_named(input, &name);
            }
            return;
        }
        if name == "@ifnextchar" {
            self.do_ifnextchar(input);
            return;
        }
        if name == "@ifstar" {
            self.do_ifstar(input);
            return;
        }
        if name == "@ifundefined" {
            self.do_atifundefined(input);
            return;
        }
        if name == "@namedef" {
            let mut cur = cursor_from_stack(input, self.cats());
            let g = cur.read_group();
            let nm = detokenize(&g);
            let body = cur.read_group();
            restack(input, &cur);
            self.st.defs.insert(nm, Meaning::Macro(Macro::simple(body)));
            return;
        }
        if name == "nameuse" || name == "@nameuse" {
            let mut cur = cursor_from_stack(input, self.cats());
            let g = cur.read_group();
            restack(input, &cur);
            let nm = detokenize(&g);
            let meaning = self.st.defs.get(&nm).cloned();
            match meaning {
                Some(Meaning::Macro(m)) => {
                    let repl = m.replacement.clone();
                    input.push_all(&repl);
                }
                Some(Meaning::Native) => out.push(Token::CtrlSeq(nm)),
                _ => {}
            }
            return;
        }
        if name == "if@compatibility"
            || name == "if@twoside"
            || name == "if@twocolumn"
            || name == "if@oneside"
            || name == "if@titlepage"
            || name == "if@openright"
            || name == "if@openbib"
            || name == "if@restonecol"
            || name == "if@mainmatter"
        {
            self.start_conditional(input, false);
            return;
        }

        // Raw environments: verbatim must NOT be macro-expanded. We intercept
        // \begin{...}, consume raw lines, and re-emit them as marker tokens.
        if name == "begin" {
            let peeked = self.take_spaces_and_peek(input);
            if let Some(tok) = peeked {
                if tok.is_bgroup() {
                    let g = self.scan_undelimited(input);
                    let env = detokenize(&g);
                    if is_raw_env(&env) {
                        let file_name = if env.trim_end_matches('*') == "filecontents" {
                            Some(detokenize(&self.scan_undelimited(input)))
                        } else {
                            None
                        };
                        self.read_raw_env(input, out, &env, file_name);
                        return;
                    }
                    // not raw: put \begin and the group back for the parser
                    out.push(Token::cs("begin"));
                    out.push(Token::Char('{', CatCode::BGroup));
                    out.extend(g);
                    out.push(Token::Char('}', CatCode::EGroup));
                    return;
                }
            }
            out.push(Token::cs("begin"));
            return;
        }
        if name == "verb" || name == "lstinline" {
            self.read_inline_verb(input, out, name == "lstinline");
            return;
        }

        // Look up definition.
        let meaning = match self.st.defs.get(name).cloned() {
            Some(m) => m,
            None => {
                out.push(Token::CtrlSeq(name.to_string()));
                return;
            }
        };

        match meaning {
            Meaning::Native => {
                out.push(Token::CtrlSeq(name.to_string()));
            }
            Meaning::LetToken(t) => {
                out.push(t);
            }
            Meaning::Let(inner) => match *inner {
                Meaning::Macro(m) => self.expand_macro(input, out, m, mode),
                _ => out.push(Token::CtrlSeq(name.to_string())),
            },
            Meaning::Macro(m) => {
                if mode == ExpandMode::Edef && !m.expand_in_edef {
                    out.push(Token::CtrlSeq(name.to_string()));
                } else {
                    self.expand_macro(input, out, m, mode);
                }
            }
        }
    }

    /// Expand `t` once, putting the result back on the input (before any
    /// token the caller pushes afterwards). Used by \expandafter.
    fn expand_once(&mut self, input: &mut InputStack, t: Token, mode: ExpandMode) {
        match t {
            Token::CtrlSeq(name) => {
                match self.st.defs.get(&name).cloned() {
                    Some(Meaning::Macro(m)) if !(mode == ExpandMode::Edef && !m.expand_in_edef) => {
                        // Expand the macro exactly one level: scan its
                        // arguments and push the replacement (unevaluated).
                        let mut args: Vec<Vec<Token>> = Vec::with_capacity(m.params.len());
                        for p in &m.params {
                            match p {
                                Param::Undelimited => args.push(self.scan_undelimited(input)),
                                Param::Delimited(d) => args.push(self.scan_delimited(input, d)),
                                Param::OptionalGroup(def) => {
                                    let peeked = self.take_spaces_and_peek(input);
                                    if matches!(peeked, Some(Token::Char('[', CatCode::Other))) {
                                        args.push(self.scan_bracket(input));
                                    } else {
                                        args.push(def.clone());
                                    }
                                }
                                Param::Star => {
                                    let peeked = self.take_spaces_and_peek(input);
                                    if matches!(peeked, Some(Token::Char('*', _))) {
                                        let _ = input.pop_tok(self.cats());
                                        args.push(vec![Token::cs("BooleanTrue")]);
                                    } else {
                                        args.push(vec![Token::cs("BooleanFalse")]);
                                    }
                                }
                                Param::Verbatim(d) => {
                                    let mut a = vec![];
                                    while let Some(tk) = input.pop_tok(self.cats()) {
                                        match &tk {
                                            Token::Char(c, _) if c == d => break,
                                            _ => a.push(tk),
                                        }
                                    }
                                    args.push(a);
                                }
                            }
                        }
                        let repl = self.substitute(&m.replacement, &args);
                        input.push_all(&repl);
                    }
                    _ => {
                        // Engine primitives (\csname, \the, \if...) write their
                        // result to an out-buffer; feed it back to the input.
                        // Natives and unknowns come back unchanged.
                        let mut tmp: Vec<Token> = vec![];
                        let mut dummy_inhibit = false;
                        self.expand_cs(input, &mut tmp, &name, mode, &mut dummy_inhibit);
                        input.push_all(&tmp);
                    }
                }
            }
            other => input.push(other),
        }
    }

    fn expand_macro(
        &mut self,
        input: &mut InputStack,
        out: &mut Vec<Token>,
        m: Macro,
        mode: ExpandMode,
    ) {
        let mut args: Vec<Vec<Token>> = Vec::with_capacity(m.params.len());
        for p in &m.params {
            match p {
                Param::Undelimited => {
                    let a = self.scan_undelimited(input);
                    args.push(a);
                }
                Param::Delimited(delim) => {
                    let a = self.scan_delimited(input, delim);
                    args.push(a);
                }
                Param::OptionalGroup(def) => {
                    let peeked = self.take_spaces_and_peek(input);
                    match peeked {
                        Some(Token::Char('[', CatCode::Other)) => {
                            let a = self.scan_bracket(input);
                            args.push(a);
                        }
                        _ => args.push(def.clone()),
                    }
                }
                Param::Star => {
                    let peeked = self.take_spaces_and_peek(input);
                    match peeked {
                        Some(Token::Char('*', CatCode::Other)) => {
                            let _ = input.pop_tok(self.cats());
                            args.push(vec![Token::cs("BooleanTrue")]);
                        }
                        _ => args.push(vec![Token::cs("BooleanFalse")]),
                    }
                }
                Param::Verbatim(delim) => {
                    let mut a = vec![];
                    while let Some(t) = input.pop_tok(self.cats()) {
                        match &t {
                            Token::Char(c, _) if c == delim => break,
                            _ => a.push(t),
                        }
                    }
                    args.push(a);
                }
            }
        }
        let produced = self.substitute(&m.replacement, &args);
        let expanded = self.expand_mode(produced, mode);
        out.extend(expanded);
    }

    fn substitute(&mut self, repl: &[Token], args: &[Vec<Token>]) -> Vec<Token> {
        let mut out = Vec::with_capacity(repl.len());
        let mut i = 0;
        while i < repl.len() {
            match &repl[i] {
                Token::Char('#', CatCode::Parameter) => {
                    if let Some(Token::Char(d, _)) = repl.get(i + 1) {
                        if d.is_ascii_digit() {
                            let n = (*d as u8 - b'0') as usize;
                            if n >= 1 && n <= args.len() {
                                out.extend(args[n - 1].iter().cloned());
                            }
                            i += 2;
                            continue;
                        }
                    }
                    out.push(Token::Char('#', CatCode::Parameter));
                    i += 1;
                }
                t => {
                    out.push(t.clone());
                    i += 1;
                }
            }
        }
        out
    }

    fn scan_undelimited(&mut self, input: &mut InputStack) -> Vec<Token> {
        while matches!(input.peek_tok(self.cats()), Some(Token::Space)) {
            input.pop_tok(self.cats());
        }
        match input.pop_tok(self.cats()) {
            Some(t) if t.is_bgroup() => {
                let mut depth = 1;
                let mut out = vec![];
                while let Some(t) = input.pop_tok(self.cats()) {
                    if t.is_bgroup() {
                        depth += 1;
                    } else if t.is_egroup() {
                        depth -= 1;
                        if depth == 0 {
                            return out;
                        }
                    }
                    out.push(t);
                }
                out
            }
            Some(t) => vec![t],
            None => vec![],
        }
    }

    fn scan_delimited(&mut self, input: &mut InputStack, delim: &[Token]) -> Vec<Token> {
        let mut depth = 0i32;
        let mut buf: Vec<Token> = vec![];
        loop {
            let t = match input.pop_tok(self.cats()) {
                Some(t) => t,
                None => break,
            };
            if t.is_bgroup() {
                depth += 1;
                buf.push(t);
                continue;
            }
            if t.is_egroup() {
                depth -= 1;
                if depth < 0 {
                    input.push(t);
                    return buf;
                }
                buf.push(t);
                continue;
            }
            buf.push(t);
            if depth == 0 && buf.len() >= delim.len() {
                let tail = &buf[buf.len() - delim.len()..];
                if tail == delim {
                    buf.truncate(buf.len() - delim.len());
                    return buf;
                }
            }
        }
        buf
    }

    fn take_spaces_and_peek(&mut self, input: &mut InputStack) -> Option<Token> {
        let mut tmp: Vec<Token> = vec![];
        let mut result: Option<Token> = None;
        while let Some(t) = input.pop_tok(self.cats()) {
            if matches!(t, Token::Space) {
                tmp.push(t);
            } else {
                result = Some(t.clone());
                input.push(t);
                break;
            }
        }
        for t in tmp.into_iter().rev() {
            input.push(t);
        }
        result
    }

    fn scan_bracket(&mut self, input: &mut InputStack) -> Vec<Token> {
        while matches!(input.peek_tok(self.cats()), Some(Token::Space)) {
            input.pop_tok(self.cats());
        }
        let _ = input.pop_tok(self.cats()); // '['
        let mut depth = 1;
        let mut out = vec![];
        while let Some(t) = input.pop_tok(self.cats()) {
            match t {
                Token::Char('[', CatCode::Other) => depth += 1,
                Token::Char(']', CatCode::Other) => {
                    depth -= 1;
                    if depth == 0 {
                        return out;
                    }
                }
                _ => {}
            }
            out.push(t);
        }
        out
    }

    // ----- conditionals -----

    /// Start a conditional: `cond` is the evaluated condition. Tokens of the
    /// non-taken branch are skipped (for false) or marked to be skipped when
    /// \else is reached (for true).
    fn start_conditional(&mut self, input: &mut InputStack, cond: bool) {
        if cond {
            self.cond_stack.push(CondState::TrueBranch);
        } else {
            // skip to \else (start false branch) or \fi (done)
            if self.skip_to_else_or_fi(input) {
                self.cond_stack.push(CondState::FalseBranch);
            }
        }
    }

    /// Skip until matching \fi; consume it. Nested \if* increase depth.
    fn skip_to_fi(&mut self, input: &mut InputStack) {
        let mut depth = 0i32;
        while let Some(t) = input.pop_tok(self.cats()) {
            match &t {
                Token::CtrlSeq(n)
                    if n.starts_with("if") && !matches!(n.as_str(), "fi" | "firstmark") =>
                {
                    depth += 1;
                }
                Token::CtrlSeq(n) if n == "fi" => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
    }

    /// Skip tokens until \else or \fi at depth 0.
    /// Returns true if \else found (positioned after it), false if \fi consumed.
    fn skip_to_else_or_fi(&mut self, input: &mut InputStack) -> bool {
        let mut depth = 0i32;
        while let Some(t) = input.pop_tok(self.cats()) {
            match &t {
                Token::CtrlSeq(n)
                    if n.starts_with("if") && !matches!(n.as_str(), "fi" | "firstmark") =>
                {
                    depth += 1;
                }
                Token::CtrlSeq(n) if n == "fi" => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                Token::CtrlSeq(n) if n == "else" && depth == 0 => {
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    // ----- primitives -----

    fn do_csname(&mut self, input: &mut InputStack, out: &mut Vec<Token>) {
        let mut body = vec![];
        let mut depth = 0;
        while let Some(t) = input.pop_tok(self.cats()) {
            match &t {
                Token::CtrlSeq(n) if n == "endcsname" && depth == 0 => break,
                Token::Char('{', CatCode::BGroup) => depth += 1,
                Token::Char('}', CatCode::EGroup) => depth -= 1,
                _ => {}
            }
            body.push(t);
        }
        let expanded = self.expand_all(body);
        let mut name = String::new();
        for t in expanded {
            match t {
                Token::Char(c, _) => name.push(c),
                Token::Space => name.push(' '),
                Token::CtrlSeq(n) => name.push_str(&n),
            }
        }
        out.push(Token::CtrlSeq(name));
    }

    fn do_the(&mut self, input: &mut InputStack, out: &mut Vec<Token>) {
        if let Some(t) = input.pop_tok(self.cats()) {
            match t {
                Token::CtrlSeq(n) => {
                    if let Some(Meaning::Macro(m)) = self.st.defs.get(&n).cloned() {
                        let repl = self.expand_all(m.replacement.clone());
                        out.extend(repl);
                    } else if n == "value" {
                        let mut cur = cursor_from_stack(input, self.cats());
                        let g = cur.read_group();
                        restack(input, &cur);
                        let v = self.st.counter_value(&detokenize(&g));
                        for c in v.to_string().chars() {
                            out.push(Token::Char(c, CatCode::Other));
                        }
                    } else if let Some(stripped) = n.strip_prefix("the") {
                        if self.st.counters.contains_key(stripped) {
                            let v = self.st.the_counter(stripped);
                            for c in v.chars() {
                                out.push(Token::Char(c, CatCode::Other));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn do_let(&mut self, input: &mut InputStack) {
        while matches!(input.peek_tok(self.cats()), Some(Token::Space)) {
            input.pop_tok(self.cats());
        }
        let target = match input.pop_tok(self.cats()) {
            Some(Token::CtrlSeq(n)) => n,
            _ => return,
        };
        while matches!(input.peek_tok(self.cats()), Some(Token::Space)) {
            input.pop_tok(self.cats());
        }
        if matches!(input.peek_tok(self.cats()), Some(Token::Char('=', _))) {
            input.pop_tok(self.cats());
        }
        while matches!(input.peek_tok(self.cats()), Some(Token::Space)) {
            input.pop_tok(self.cats());
        }
        match input.pop_tok(self.cats()) {
            Some(Token::CtrlSeq(src)) => {
                let meaning = match self.st.defs.get(&src).cloned() {
                    Some(m) => m,
                    None => {
                        if is_native(&src) {
                            Meaning::Native
                        } else {
                            Meaning::LetToken(Token::CtrlSeq(src))
                        }
                    }
                };
                self.st.defs.insert(target, meaning);
            }
            Some(t) => {
                self.st.defs.insert(target, Meaning::LetToken(t));
            }
            None => {}
        }
    }

    fn do_futurelet(&mut self, input: &mut InputStack) {
        let a = match input.pop_tok(self.cats()) {
            Some(Token::CtrlSeq(n)) => n,
            _ => return,
        };
        let b = match input.pop_tok(self.cats()) {
            Some(Token::CtrlSeq(n)) => n,
            Some(t) => {
                input.push(t);
                String::new()
            }
            None => return,
        };
        while matches!(input.peek_tok(self.cats()), Some(Token::Space)) {
            input.pop_tok(self.cats());
        }
        let c = input.pop_tok(self.cats());
        if let Some(ct) = c.clone() {
            match ct {
                Token::CtrlSeq(src) => {
                    let meaning = match self.st.defs.get(&src).cloned() {
                        Some(m) => m,
                        None => {
                            if is_native(&src) {
                                Meaning::Native
                            } else {
                                Meaning::LetToken(Token::CtrlSeq(src))
                            }
                        }
                    };
                    self.st.defs.insert(a, meaning);
                }
                t => {
                    self.st.defs.insert(a, Meaning::LetToken(t));
                }
            }
        }
        if !b.is_empty() {
            input.push(Token::CtrlSeq(b));
        }
        if let Some(ct) = c {
            input.push(ct);
        }
    }

    fn do_catcode(&mut self, input: &mut InputStack) {
        while matches!(input.peek_tok(self.cats()), Some(Token::Space)) {
            input.pop_tok(self.cats());
        }
        if let Some(Token::Char('`', _)) = input.pop_tok(self.cats()) {
            if let Some(Token::Char(c, _)) = input.pop_tok(self.cats()) {
                while matches!(input.peek_tok(self.cats()), Some(Token::Space)) {
                    input.pop_tok(self.cats());
                }
                if matches!(input.peek_tok(self.cats()), Some(Token::Char('=', _))) {
                    input.pop_tok(self.cats());
                }
                let mut cur = cursor_from_stack(input, self.cats());
                let v = scan_int(self.st, &mut cur);
                restack(input, &cur);
                if (0..=15).contains(&v) {
                    self.st.catcodes.insert(c, v as u8);
                }
            }
        }
    }

    fn do_newif(&mut self, input: &mut InputStack) {
        if let Some(Token::CtrlSeq(name)) = input.pop_tok(self.cats()) {
            if let Some(stripped) = name.strip_prefix("if") {
                self.st.conds.entry(stripped.to_string()).or_insert(false);
            }
        }
    }

    fn do_newcounter(&mut self, input: &mut InputStack) {
        let mut cur = cursor_from_stack(input, self.cats());
        let g = cur.read_group();
        let name = detokenize(&g);
        let within = cur.read_optional().map(|v| detokenize(&v));
        restack(input, &cur);
        self.st.new_counter(&name, within.filter(|s| !s.is_empty()));
    }

    fn do_setcounter(&mut self, input: &mut InputStack, add: bool) {
        let mut cur = cursor_from_stack(input, self.cats());
        let g = cur.read_group();
        let name = detokenize(&g);
        let v = scan_int(self.st, &mut cur);
        restack(input, &cur);
        if self.st.counters.get(&name).is_none() {
            self.st.new_counter(&name, None);
        }
        if let Some(c) = self.st.counters.get_mut(&name) {
            if add {
                c.value += v;
            } else {
                c.value = v;
            }
        }
    }

    fn do_stepcounter(&mut self, input: &mut InputStack, reff: bool) {
        let mut cur = cursor_from_stack(input, self.cats());
        let g = cur.read_group();
        let name = detokenize(&g);
        restack(input, &cur);
        if reff {
            self.st.refstep(&name);
        } else {
            self.st.step_counter(&name);
        }
    }

    fn do_newenvironment(&mut self, input: &mut InputStack) {
        let mut cur = cursor_from_stack(input, self.cats());
        let name = detokenize(&cur.read_group());
        let nargs = cur
            .read_optional()
            .map(|v| detokenize(&v).trim().parse().unwrap_or(0))
            .unwrap_or(0);
        let optdefault = cur.read_optional();
        let begin_toks = cur.read_group();
        let end_toks = cur.read_group();
        restack(input, &cur);
        self.st
            .user_envs
            .insert(name, (nargs, optdefault, begin_toks, end_toks));
    }

    fn do_newtheorem(&mut self, input: &mut InputStack) {
        let mut cur = cursor_from_stack(input, self.cats());
        let env = detokenize(&cur.read_group());
        let mut title;
        let mut within = None;
        let mut numbered = true;
        cur.skip_spaces();
        if matches!(cur.peek(), Some(Token::Char('*', _))) {
            cur.pos += 1;
            numbered = false;
            title = detokenize(&cur.read_group());
            within = Some(detokenize(&cur.read_group()));
        } else {
            title = detokenize(&cur.read_group());
            if let Some(g) = cur.read_optional() {
                within = Some(detokenize(&g));
            }
        }
        restack(input, &cur);
        let thmname = title;
        title = String::new();
        let _ = title;
        if numbered && !self.st.counters.contains_key(&env) {
            let the_tokens = match &within {
                Some(w) if w != "\0" => {
                    let mut v = self
                        .st
                        .counters
                        .get(w)
                        .map(|c| c.the_tokens.clone())
                        .unwrap_or_else(|| counter_ref_tokens(w, "arabic"));
                    v.push(Token::Char('.', CatCode::Other));
                    v.extend(counter_ref_tokens(&env, "arabic"));
                    v
                }
                _ => counter_ref_tokens(&env, "arabic"),
            };
            self.st.counters.insert(
                env.clone(),
                Counter {
                    value: 0,
                    within: within.clone().filter(|w| w != "\0"),
                    the_tokens: the_tokens.clone(),
                },
            );
            self.st.defs.insert(
                format!("the{}", env),
                Meaning::Macro(Macro::simple(the_tokens)),
            );
        }
        self.st.theorems.insert(
            env,
            (
                thmname,
                if numbered {
                    within
                } else {
                    Some("\0".to_string())
                },
            ),
        );
    }

    fn read_raw_env(
        &mut self,
        input: &mut InputStack,
        out: &mut Vec<Token>,
        env: &str,
        file_name: Option<String>,
    ) {
        let env_clean = env.trim_end_matches('*');
        let end_marker = format!("\\end{{{}}}", env_clean);
        // Token-source fallback (verbatim inside a macro expansion): scan
        // tokens until \end{env}.
        let from_toks = matches!(input.peek_tok(self.cats()), Some(t) if !t.is_cs("@none@"));
        let mut lines: Vec<String> = vec![];
        let mut tok_buf: Vec<Token> = vec![];
        let mut ok = true;
        loop {
            match input.read_raw_line() {
                Some(line) => {
                    if line.trim_start().starts_with(end_marker.as_str())
                        || line
                            .trim_start()
                            .starts_with(format!("\\end {{{}", env_clean).as_str())
                    {
                        break;
                    }
                    lines.push(line);
                }
                None => {
                    // token fallback
                    let mut found_end = false;
                    while let Some(t) = input.pop_tok(self.cats()) {
                        if t.is_cs("end") {
                            // check group
                            let g = self.scan_undelimited(input);
                            if detokenize(&g).trim_end_matches('*') == env_clean {
                                found_end = true;
                                break;
                            }
                            tok_buf.push(Token::cs("end"));
                            tok_buf.extend(g);
                        } else {
                            tok_buf.push(t);
                        }
                    }
                    ok = found_end;
                    break;
                }
            }
        }
        if !ok {
            self.st.warn(format!("unterminated {} environment", env));
        }
        match env_clean {
            "comment" | "tikzpicture" => {
                if env_clean == "tikzpicture" {
                    self.st.warn("tikzpicture content skipped (not supported)");
                }
            }
            "filecontents" => {
                let name = file_name.unwrap_or_else(|| format!("{}.tex", self.st.jobname));
                let content = lines.join("\n");
                if let Err(error) = std::fs::write(&name, content) {
                    self.st
                        .warn(format!("cannot write filecontents `{}`: {}", name, error));
                }
            }
            _ => {
                // verbatim, verbatim*, lstlisting
                let _ = tok_buf;
                out.push(Token::cs("@verbstart"));
                for l in lines {
                    out.push(Token::cs("@verbline"));
                    for c in l.chars() {
                        out.push(Token::Char(c, CatCode::Other));
                    }
                }
                if !tok_buf.is_empty() {
                    // token fallback: split on \\ control sequence
                    let mut cur = String::new();
                    let mut all: Vec<String> = vec![];
                    for t in &tok_buf {
                        if t.is_cs("\\") || t.is_cs("newline") {
                            all.push(cur.clone());
                            cur.clear();
                        } else {
                            cur.push_str(&detokenize(&[t.clone()]));
                        }
                    }
                    all.push(cur);
                    for l in all {
                        out.push(Token::cs("@verbline"));
                        for c in l.chars() {
                            out.push(Token::Char(c, CatCode::Other));
                        }
                    }
                }
                out.push(Token::cs("@verbend"));
            }
        }
    }

    fn read_inline_verb(&mut self, input: &mut InputStack, out: &mut Vec<Token>, lst: bool) {
        // read delimiter char raw
        let mut delim = match input.read_raw_char() {
            Some(c) => c,
            None => {
                self.st.warn("unterminated \\verb");
                return;
            }
        };
        if delim == '*' {
            delim = match input.read_raw_char() {
                Some(c) => c,
                None => return,
            };
        }
        if lst && delim == '[' {
            // skip optional language arg
            while let Some(c) = input.read_raw_char() {
                if c == ']' {
                    break;
                }
            }
            delim = match input.read_raw_char() {
                Some(c) => c,
                None => return,
            };
        }
        let mut content = String::new();
        while let Some(c) = input.read_raw_char() {
            if c == delim {
                break;
            }
            content.push(c);
        }
        out.push(Token::cs("@verbstart"));
        out.push(Token::cs("@verbline"));
        for c in content.chars() {
            out.push(Token::Char(c, CatCode::Other));
        }
        out.push(Token::cs("@verbend"));
    }

    fn do_usepackage(&mut self, input: &mut InputStack, out: &mut Vec<Token>, name: &str) {
        let optional = if matches!(self.take_spaces_and_peek(input), Some(Token::Char('[', _))) {
            Some(self.scan_bracket(input))
        } else {
            None
        };
        let packages = self.scan_undelimited(input);

        out.push(Token::cs(name));
        if let Some(optional) = optional {
            out.push(Token::Char('[', CatCode::Other));
            out.extend(optional);
            out.push(Token::Char(']', CatCode::Other));
        }
        out.push(Token::Char('{', CatCode::BGroup));
        out.extend(packages.iter().cloned());
        out.push(Token::Char('}', CatCode::EGroup));

        let base = input
            .current_directory()
            .map(Path::to_path_buf)
            .or_else(|| self.source_directories.last().cloned());
        for package in detokenize(&packages)
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            let path = match find_input_file_at(self.st, package, base.as_deref(), "sty") {
                Some(path) => path.canonicalize().unwrap_or(path),
                None if is_builtin_package(package) => continue,
                None => {
                    self.st.warn(format!(
                        "package not found: {} (expected {}.sty)",
                        package, package
                    ));
                    continue;
                }
            };
            if !self.loaded_packages.insert(path.clone()) {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(src) => {
                    let previous_at = cat_of(self.cats(), '@');
                    self.st.catcodes.insert('@', 11);
                    let directory = path.parent().unwrap_or(Path::new(".")).to_path_buf();
                    let mut package_input =
                        InputStack::with_source_at(&src, Some(directory.clone()));
                    self.source_directories.push(directory);
                    let expanded = self.expand_loop(&mut package_input, ExpandMode::Normal);
                    self.source_directories.pop();
                    self.st.catcodes.insert('@', previous_at);
                    out.extend(expanded);
                }
                Err(_) => self
                    .st
                    .warn(format!("cannot read package {}", path.display())),
            }
        }
    }

    fn do_input(&mut self, input: &mut InputStack) {
        let mut cur = cursor_from_stack(input, self.cats());
        let g = cur.read_group();
        restack(input, &cur);
        let name = detokenize(&g);
        self.do_input_named(input, &name);
    }

    fn do_input_named(&mut self, input: &mut InputStack, name: &str) {
        let base = input
            .current_directory()
            .map(Path::to_path_buf)
            .or_else(|| self.source_directories.last().cloned());
        let path = find_input_file_at(self.st, name, base.as_deref(), "tex");
        match path {
            Some(p) => match std::fs::read_to_string(&p) {
                Ok(src) => {
                    // Lazy source: catcode changes inside the file apply as
                    // the file is read, like TeX.
                    input.push_source(&src, p.parent().map(Path::to_path_buf));
                }
                Err(_) => self.st.warn(format!("cannot read file {}", p.display())),
            },
            None => self.st.warn(format!("file not found: {}", name)),
        }
    }

    fn do_ifnextchar(&mut self, input: &mut InputStack) {
        let mut cur = cursor_from_stack(input, self.cats());
        let probe = cur.read_group();
        let t_branch = cur.read_group();
        let f_branch = cur.read_group();
        while matches!(cur.peek(), Some(Token::Space)) {
            cur.pos += 1;
        }
        let matches_probe = match (probe.first(), cur.peek()) {
            (Some(Token::CtrlSeq(a)), Some(Token::CtrlSeq(b))) => a == b,
            (Some(Token::Char(a, _)), Some(Token::Char(b, _))) => a == b,
            _ => false,
        };
        restack(input, &cur);
        let branch = if matches_probe { t_branch } else { f_branch };
        let expanded = self.expand_all(branch);
        input.push_all(&expanded);
    }

    fn do_ifstar(&mut self, input: &mut InputStack) {
        let mut cur = cursor_from_stack(input, self.cats());
        let t_branch = cur.read_group();
        let f_branch = cur.read_group();
        while matches!(cur.peek(), Some(Token::Space)) {
            cur.pos += 1;
        }
        let is_star = matches!(cur.peek(), Some(Token::Char('*', _)));
        if is_star {
            cur.pos += 1;
        }
        restack(input, &cur);
        let branch = if is_star { t_branch } else { f_branch };
        let expanded = self.expand_all(branch);
        input.push_all(&expanded);
    }

    fn do_atifundefined(&mut self, input: &mut InputStack) {
        let mut cur = cursor_from_stack(input, self.cats());
        let g = cur.read_group();
        let nm = detokenize(&g);
        let t_branch = cur.read_group();
        let f_branch = cur.read_group();
        let defined = self.st.defs.contains_key(&nm) || is_native(&nm);
        restack(input, &cur);
        let branch = if !defined { t_branch } else { f_branch };
        let expanded = self.expand_all(branch);
        input.push_all(&expanded);
    }

    // ----- defining commands -----

    /// Handle \def, \gdef, \edef, \xdef, \newcommand(+star), \renewcommand,
    /// \providecommand, \DeclareRobustCommand, and xparse \NewDocumentCommand
    /// family. Returns true if `name` was a defining command.
    fn try_define(&mut self, input: &mut InputStack, name: &str) -> bool {
        let kind = match name {
            "def" | "gdef" | "edef" | "xdef" => "def",
            "newcommand"
            | "renewcommand"
            | "providecommand"
            | "DeclareRobustCommand"
            | "DeclareMathOperator" => "newcommand",
            "NewDocumentCommand"
            | "RenewDocumentCommand"
            | "ProvideDocumentCommand"
            | "DeclareDocumentCommand" => "xparse",
            _ => return false,
        };
        let edef = matches!(name, "edef" | "xdef");
        let mut cur = cursor_from_stack(input, self.cats());
        cur.skip_spaces();
        if matches!(cur.peek(), Some(Token::Char('*', _))) {
            cur.pos += 1;
        }
        let cmd_name = {
            cur.skip_spaces();
            match cur.peek() {
                Some(Token::CtrlSeq(_)) => match cur.next() {
                    Some(Token::CtrlSeq(n)) => n,
                    _ => return true,
                },
                Some(t) if t.is_bgroup() => {
                    // \newcommand{\foo} style: command name wrapped in braces.
                    let inner = cur.read_group();
                    match inner.first() {
                        Some(Token::CtrlSeq(n)) => n.clone(),
                        _ => return true,
                    }
                }
                _ => return true,
            }
        };

        let mut params: Vec<Param> = vec![];
        let mut replacement: Vec<Token> = vec![];

        match kind {
            "def" => {
                let mut ptext = vec![];
                while let Some(t) = cur.peek() {
                    if t.is_bgroup() {
                        break;
                    }
                    ptext.push(t.clone());
                    cur.pos += 1;
                }
                params = parse_param_text(&ptext);
                replacement = cur.read_group();
                normalize_hashes(&mut replacement);
            }
            "newcommand" => {
                let nargs = cur
                    .read_optional()
                    .map(|v| detokenize(&v).trim().parse().unwrap_or(0))
                    .unwrap_or(0);
                let optdefault = cur.read_optional();
                for i in 0..nargs {
                    if i == 0 {
                        if let Some(d) = &optdefault {
                            params.push(Param::OptionalGroup(d.clone()));
                            continue;
                        }
                    }
                    params.push(Param::Undelimited);
                }
                replacement = cur.read_group();
            }
            "xparse" => {
                let spec = detokenize(&cur.read_group());
                params = parse_xparse_spec(&spec);
                replacement = cur.read_group();
            }
            _ => {}
        }
        restack(input, &cur);

        let mut mac = Macro {
            params,
            replacement,
            expand_in_edef: true,
            is_xparse: kind == "xparse",
        };
        if name.starts_with("DeclareRobustCommand") {
            mac.expand_in_edef = false;
        }
        if name.starts_with("DeclareMathOperator") {
            // treat as \operatorname wrapper: define macro producing \operatorname{...}
            let inner = mac.replacement.clone();
            let mut repl = vec![Token::cs("operatorname"), Token::Char('{', CatCode::BGroup)];
            repl.extend(inner);
            repl.push(Token::Char('}', CatCode::EGroup));
            mac.replacement = repl;
        }
        if name.starts_with("providecommand") && self.st.defs.contains_key(&cmd_name) {
            return true;
        }
        if edef {
            let repl = self.expand_mode(mac.replacement.clone(), ExpandMode::Edef);
            mac.replacement = repl;
        }
        self.st.defs.insert(cmd_name, Meaning::Macro(mac));
        true
    }
}

/// Parse a \def parameter text like `#1#2` or `#1[#2]` into Params.
fn parse_param_text(ptext: &[Token]) -> Vec<Param> {
    let mut params = vec![];
    let mut i = 0;
    while i < ptext.len() {
        match &ptext[i] {
            Token::Char('#', CatCode::Parameter) => {
                if let Some(Token::Char(d, _)) = ptext.get(i + 1) {
                    if d.is_ascii_digit() {
                        let mut j = i + 2;
                        let mut delim = vec![];
                        while j < ptext.len() {
                            match &ptext[j] {
                                Token::Char('#', CatCode::Parameter) => break,
                                t => delim.push(t.clone()),
                            }
                            j += 1;
                        }
                        if delim.is_empty() {
                            params.push(Param::Undelimited);
                        } else {
                            params.push(Param::Delimited(delim));
                        }
                        i = j;
                        continue;
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    params
}

/// Collapse ## to # in a replacement text being defined.
fn normalize_hashes(toks: &mut Vec<Token>) {
    let mut out = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        if matches!(toks[i], Token::Char('#', CatCode::Parameter))
            && matches!(toks.get(i + 1), Some(Token::Char('#', CatCode::Parameter)))
        {
            out.push(Token::Char('#', CatCode::Parameter));
            i += 2;
        } else {
            out.push(toks[i].clone());
            i += 1;
        }
    }
    *toks = out;
}

fn parse_xparse_spec(spec: &str) -> Vec<Param> {
    let chars: Vec<char> = spec.chars().collect();
    let mut params = vec![];
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            'm' => params.push(Param::Undelimited),
            'o' => params.push(Param::OptionalGroup(vec![])),
            'O' => {
                if i + 1 < chars.len() && chars[i + 1] == '{' {
                    let mut depth = 1;
                    let mut j = i + 2;
                    let mut def = String::new();
                    while j < chars.len() && depth > 0 {
                        match chars[j] {
                            '{' => depth += 1,
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            c => def.push(c),
                        }
                        j += 1;
                    }
                    params.push(Param::OptionalGroup(lex(&def, &default_catcodes())));
                    i = j;
                }
            }
            's' => params.push(Param::Star),
            'v' => {
                if i + 1 < chars.len() {
                    params.push(Param::Verbatim(chars[i + 1]));
                    i += 1;
                }
            }
            'd' | 'D' => {
                if i + 2 < chars.len() {
                    let mut def: Vec<Token> = vec![Token::cs("NoValue")];
                    let mut j = i + 3;
                    if chars[i] == 'D' && j < chars.len() && chars[j] == '{' {
                        let mut depth = 1;
                        let mut s = String::new();
                        j += 1;
                        while j < chars.len() && depth > 0 {
                            match chars[j] {
                                '{' => depth += 1,
                                '}' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                c => s.push(c),
                            }
                            j += 1;
                        }
                        def = lex(&s, &default_catcodes());
                        i = j - 1;
                    } else {
                        i += 2;
                    }
                    params.push(Param::OptionalGroup(def));
                }
            }
            't' => {
                if i + 1 < chars.len() {
                    params.push(Param::OptionalGroup(vec![Token::Char(
                        chars[i + 1],
                        CatCode::Other,
                    )]));
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    params
}

/// Search for an input file like TeX would: relative to cwd, and TEXINPUTS paths.
pub fn find_input_file(st: &State, name: &str) -> Option<PathBuf> {
    find_input_file_at(st, name, None, "tex")
}

fn find_input_file_at(
    _st: &State,
    name: &str,
    base: Option<&Path>,
    extension: &str,
) -> Option<PathBuf> {
    let n = if Path::new(name).extension().is_none() {
        format!("{}.{}", name, extension)
    } else {
        name.to_string()
    };
    let file = PathBuf::from(n);
    let mut candidates = vec![];
    if file.is_absolute() {
        candidates.push(file.clone());
    } else {
        if let Some(base) = base {
            candidates.push(base.join(&file));
        }
        candidates.push(file.clone());
        if let Ok(texinputs) = std::env::var("TEXINPUTS") {
            for dir in texinputs.split(':') {
                if !dir.is_empty() {
                    candidates.push(Path::new(dir).join(&file));
                }
            }
        }
    }
    candidates.push(PathBuf::from(name));
    candidates.into_iter().find(|p| p.is_file())
}

fn is_builtin_package(package: &str) -> bool {
    matches!(
        package,
        "amsmath"
            | "amssymb"
            | "amsfonts"
            | "amsthm"
            | "mathtools"
            | "bm"
            | "hyperref"
            | "graphicx"
            | "glossaries"
            | "glossaries-extra"
            | "natbib"
            | "apalike"
            | "ctex"
            | "xeCJK"
            | "geometry"
            | "fontspec"
            | "xcolor"
            | "color"
            | "booktabs"
            | "array"
            | "longtable"
            | "multirow"
            | "cleveref"
            | "url"
            | "listings"
            | "inputenc"
            | "fontenc"
            | "babel"
            | "polyglossia"
            | "enumitem"
            | "caption"
            | "subcaption"
            | "float"
            | "placeins"
            | "tabularx"
            | "makecell"
            | "hhline"
            | "tikz"
            | "pgfplots"
            | "algorithm"
            | "algorithmic"
            | "algpseudocode"
            | "listingsutf8"
            | "microtype"
            | "setspace"
            | "siunitx"
    )
}

/// Environments read raw (no macro expansion inside).
pub fn is_raw_env(env: &str) -> bool {
    matches!(
        env,
        "verbatim"
            | "verbatim*"
            | "lstlisting"
            | "comment"
            | "tikzpicture"
            | "filecontents"
            | "filecontents*"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(st: &mut State, src: &str) -> String {
        let toks = lex(src, &default_catcodes());
        let mut ex = Expander::new(st);
        let out = ex.expand_all(toks);
        detokenize(&out)
    }

    fn run_src(st: &mut State, src: &str) -> String {
        let mut ex = Expander::new(st);
        let out = ex.expand_source(src);
        detokenize(&out)
    }

    #[test]
    fn def_with_args() {
        let mut st = State::new();
        run(&mut st, r"\def\foo#1{bar(#1)}");
        assert_eq!(run(&mut st, r"\foo{xyz}"), "bar(xyz)");
    }

    #[test]
    fn newcommand_optional() {
        let mut st = State::new();
        run(&mut st, r"\newcommand{\foo}[1][dft]{A[#1]}");
        assert_eq!(run(&mut st, r"\foo"), "A[dft]");
        assert_eq!(run(&mut st, r"\foo[x]"), "A[x]");
    }

    #[test]
    fn expandafter() {
        let mut st = State::new();
        run(&mut st, r"\def\a{b}\def\b{c}");
        // \expandafter\a\b with \a=b, \b=c: \b expands in place -> \a c -> "bc"
        assert_eq!(run(&mut st, r"\expandafter\a\b"), "bc");
        // classic: \expandafter\def\csname ...\endcsname{...}
        run(&mut st, r"\expandafter\def\csname m\endcsname{9}");
        assert_eq!(run(&mut st, r"\csname m\endcsname"), "9");
    }

    #[test]
    fn csname_creates_macro() {
        let mut st = State::new();
        run(&mut st, r"\expandafter\def\csname mymac\endcsname{42}");
        assert_eq!(run(&mut st, r"\csname mymac\endcsname"), "42");
    }

    #[test]
    fn ifnum_branching() {
        let mut st = State::new();
        assert_eq!(run(&mut st, r"\ifnum 2>1 yes\else no\fi"), "yes");
        assert_eq!(run(&mut st, r"\ifnum 1>2 yes\else no\fi"), "no");
    }

    #[test]
    fn if_else_nested() {
        let mut st = State::new();
        assert_eq!(
            run(&mut st, r"\ifnum 1=1 a\ifnum 2=2 b\else c\fi d\else e\fi"),
            "abd"
        );
    }

    #[test]
    fn edef_freezes() {
        let mut st = State::new();
        run(&mut st, r"\def\inner{I}");
        run(&mut st, r"\edef\outmac{\inner}");
        run(&mut st, r"\def\inner{X}");
        assert_eq!(run(&mut st, r"\outmac"), "I");
    }

    #[test]
    fn protected_survives_edef() {
        let mut st = State::new();
        run(&mut st, r"\DeclareRobustCommand{\foo}{X}");
        run(&mut st, r"\edef\bar{\foo}");
        assert_eq!(run(&mut st, r"\bar"), "X");
    }

    #[test]
    fn stepcounter_resets_children() {
        let mut st = State::new();
        st.refstep("section");
        st.refstep("subsection");
        assert_eq!(st.the_counter("subsection"), "1.1");
        st.refstep("section");
        assert_eq!(st.the_counter("subsection"), "2.0");
        assert_eq!(st.the_counter("section"), "2");
    }

    #[test]
    fn roman_and_chinese() {
        assert_eq!(format_number(2024, "roman"), "mmxxiv");
        assert_eq!(format_number(2024, "Roman"), "MMXXIV");
        assert_eq!(format_number(42, "chinese"), "四十二");
        assert_eq!(format_number(13, "chinese"), "十三");
        assert_eq!(format_number(1, "alph"), "a");
        assert_eq!(format_number(27, "alph"), "aa");
    }

    #[test]
    fn xparse_command() {
        let mut st = State::new();
        run(&mut st, r"\NewDocumentCommand{\foo}{mo}{A:#1|#2|}");
        assert_eq!(run(&mut st, r"\foo{x}"), "A:x||");
        assert_eq!(run(&mut st, r"\foo{x}[y]"), "A:x|y|");
    }

    #[test]
    fn ifnextchar_detects_bracket() {
        let mut st = State::new();
        let out = run_src(&mut st, r"\makeatletter \@ifnextchar[ {HAS}{NO} [x]");
        assert!(out.starts_with("HAS"), "got {}", out);
    }

    #[test]
    fn makeatletter_same_line() {
        let mut st = State::new();
        let toks = {
            let mut ex = Expander::new(&mut st);
            ex.expand_source(r"\makeatletter \def\@foo{OK} \makeatother")
        };
        // re-enter with @ as letter via source again
        let mut st2 = State::new();
        let mut ex = Expander::new(&mut st2);
        let out = ex.expand_source("\\makeatletter \\def\\@foo{BAR}\\@foo\\makeatother");
        let _ = toks;
        assert!(detokenize(&out).contains("BAR"));
    }

    #[test]
    fn loads_recursive_package_macros_without_cycles() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "doclatex-packages-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("parent.sty"),
            r"\ProvidesPackage{parent}[2026/01/01]
\newcommand{\parent@inner}[1]{Inner:#1}
\newcommand{\fromparent}[1]{\parent@inner{#1}}
\RequirePackage{child}",
        )
        .unwrap();
        std::fs::write(
            dir.join("child.sty"),
            r"\ProvidesPackage{child}
\NewDocumentCommand{\fromchild}{O{D} m}{C(#2)/#1}
\RequirePackage{parent}",
        )
        .unwrap();

        let parent = dir.join("parent").display().to_string();
        let source = format!(
            "\\usepackage[option]{{{0},{0}}}\\fromparent{{X}}\\fromchild{{Y}}\\fromchild[Z]{{Y}}",
            parent
        );
        let mut st = State::new();
        let out = run_src(&mut st, &source);

        assert!(out.contains("Inner:X"), "got {}", out);
        assert!(out.contains("C(Y)/D"), "got {}", out);
        assert!(out.contains("C(Y)/Z"), "got {}", out);
        assert!(!out.contains("ProvidesPackage"), "got {}", out);
        assert!(!out.contains("2026/01/01"), "got {}", out);
        assert_eq!(cat_of(&st.catcodes, '@'), 12);
        assert!(st.warnings.is_empty(), "got {:?}", st.warnings);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_custom_package_warns_but_builtin_does_not() {
        let mut st = State::new();
        let _ = run_src(
            &mut st,
            r"\usepackage{amsmath,hyperref}\RequirePackage{definitely_missing_doclatex_package}",
        );
        assert_eq!(st.warnings.len(), 1, "got {:?}", st.warnings);
        assert!(st.warnings[0].contains("definitely_missing_doclatex_package"));
    }
}
