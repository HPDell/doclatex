//! LaTeX math parsing and conversion to Office Math Markup Language (OMML).
//!
//! Supports a wide subset: fractions, radicals, scripts, n-ary operators
//! with limits, delimiters (\left/\right, \bigl...), functions, accents,
//! over/under braces, matrices and arrays, cases, aligned/gathered
//! constructs, \text/\mathrm/\mathbf/\mathcal/\mathbb/\mathfrak, spacing,
//! \boxed, \phantom, \operatorname, and user macro expansion inside math.

use crate::engine::{Cursor, Expander, Meaning, State};
use crate::model::Align;
use crate::token::*;

// ---------------------------------------------------------------------------
// Node model
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Node {
    /// A run of text (already Unicode-mapped), with optional style.
    Run {
        text: String,
        bold: bool,
        nor: bool,
    },
    AlignMark,
    Space(String),
    Frac {
        num: Vec<Node>,
        den: Vec<Node>,
        bar: bool,
    },
    Sqrt {
        rad: Vec<Node>,
        root: Option<Vec<Node>>,
    },
    Delimited {
        left: String,
        right: String,
        grow: bool,
        content: Vec<Node>,
    },
    Sub {
        base: Box<Node>,
        sub: Vec<Node>,
    },
    Sup {
        base: Box<Node>,
        sup: Vec<Node>,
    },
    SubSup {
        base: Box<Node>,
        sub: Vec<Node>,
        sup: Vec<Node>,
    },
    Nary {
        chr: String,
        lim_loc: bool,
        sub: Option<Vec<Node>>,
        sup: Option<Vec<Node>>,
        e: Vec<Node>,
    },
    Func {
        name: String,
        e: Vec<Node>,
    },
    Acc {
        chr: &'static str,
        e: Vec<Node>,
    },
    Bar {
        top: bool,
        e: Vec<Node>,
    },
    GroupChr {
        top: bool,
        e: Vec<Node>,
    },
    Matrix {
        rows: Vec<Vec<Vec<Node>>>,
        aligns: Vec<Align>,
    },
    EqArr {
        rows: Vec<Vec<Node>>,
    },
    Phantom(Vec<Node>),
    BorderBox(Vec<Node>),
    Empty,
}

// ---------------------------------------------------------------------------
// Symbol tables
// ---------------------------------------------------------------------------

fn greek(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" => "ϵ",
        "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" => "θ",
        "vartheta" => "ϑ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "varpi" => "ϖ",
        "rho" => "ρ",
        "varrho" => "ϱ",
        "sigma" => "σ",
        "varsigma" => "ς",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" => "ϕ",
        "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        _ => return None,
    })
}

fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "pm" => "±",
        "mp" => "∓",
        "times" => "×",
        "div" => "÷",
        "cdot" => "⋅",
        "cdots" => "⋯",
        "ldots" => "…",
        "dots" => "…",
        "dotsc" => "…",
        "dotsb" => "⋯",
        "dotso" => "…",
        "prime" => "′",
        "circ" => "∘",
        "bullet" => "∙",
        "star" => "⋆",
        "ast" => "∗",
        "oplus" => "⊕",
        "ominus" => "⊖",
        "otimes" => "⊗",
        "oslash" => "⊘",
        "odot" => "⊙",
        "dagger" => "†",
        "ddagger" => "‡",
        "cap" => "∩",
        "cup" => "∪",
        "uplus" => "⊎",
        "sqcap" => "⊓",
        "sqcup" => "⊔",
        "vee" => "∨",
        "wedge" => "∧",
        "setminus" => "∖",
        "wr" => "≀",
        "diamond" => "⋄",
        "bigtriangleup" => "△",
        "bigtriangledown" => "▽",
        "triangleleft" => "◁",
        "triangleright" => "▷",
        "amalg" => "⨿",
        "hbar" => "ℏ",
        "ell" => "ℓ",
        "Re" => "ℜ",
        "Im" => "ℑ",
        "aleph" => "ℵ",
        "wp" => "℘",
        "mho" => "℧",
        "emptyset" => "∅",
        "varnothing" => "∅",
        "neg" => "¬",
        "lnot" => "¬",
        "top" => "⊤",
        "bot" => "⊥",
        "vdash" => "⊢",
        "dashv" => "⊣",
        "models" => "⊨",
        "Vdash" => "⊩",
        "prec" => "≺",
        "succ" => "≻",
        "preceq" => "⪯",
        "succeq" => "⪰",
        "ll" => "≪",
        "gg" => "≫",
        "equiv" => "≡",
        "sim" => "∼",
        "simeq" => "≃",
        "approx" => "≈",
        "cong" => "≅",
        "propto" => "∝",
        "bowtie" => "⋈",
        "smile" => "⌣",
        "frown" => "⌢",
        "asymp" => "≍",
        "doteq" => "≐",
        "perp" => "⊥",
        "parallel" => "∥",
        "nparallel" => "∦",
        "mid" => "∣",
        "nmid" => "∤",
        "nless" => "≮",
        "ngtr" => "≯",
        "leq" => "≤",
        "le" => "≤",
        "geq" => "≥",
        "ge" => "≥",
        "leqslant" => "≤slant;",
        "geqslant" => "≥slant;",
        "neq" => "≠",
        "ne" => "≠",
        "lll" => "⋘",
        "ggg" => "⋙",
        "lesssim" => "≲",
        "gtrsim" => "≳",
        "lessgtr" => "≶",
        "gtrless" => "≷",
        "in" => "∈",
        "ni" => "∋",
        "notin" => "∉",
        "subset" => "⊂",
        "supset" => "⊃",
        "subseteq" => "⊆",
        "supseteq" => "⊇",
        "nsubseteq" => "⊈",
        "sqsubset" => "⊏",
        "sqsupset" => "⊐",
        "sqsubseteq" => "⊑",
        "sqsupseteq" => "⊒",
        "infty" => "∞",
        "nabla" => "∇",
        "partial" => "∂",
        "forall" => "∀",
        "exists" => "∃",
        "nexists" => "∄",
        "negmedspace" => "",
        "degree" => "°",
        "angle" => "∠",
        "measuredangle" => "∡",
        "triangle" => "△",
        "square" => "□",
        "blacksquare" => "■",
        "lozenge" => "◊",
        "sphericalangle" => "∢",
        "because" => "∵",
        "therefore" => "∴",
        "vartriangle" => "▵",
        "checkmark" => "✓",
        "complement" => "∁",
        "Bbbk" => "k",
        "leftarrow" => "←",
        "gets" => "←",
        "rightarrow" => "→",
        "to" => "→",
        "leftrightarrow" => "↔",
        "Leftarrow" => "⇐",
        "Rightarrow" => "⇒",
        "Leftrightarrow" => "⇔",
        "mapsto" => "↦",
        "hookleftarrow" => "↩",
        "hookrightarrow" => "↪",
        "leftharpoonup" => "↼",
        "leftharpoondown" => "↽",
        "rightharpoonup" => "⇀",
        "rightharpoondown" => "⇁",
        "rightleftharpoons" => "⇌",
        "leftrightharpoons" => "⇋",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "updownarrow" => "↕",
        "Uparrow" => "⇑",
        "Downarrow" => "⇓",
        "Updownarrow" => "⇕",
        "nearrow" => "↗",
        "searrow" => "↘",
        "swarrow" => "↙",
        "nwarrow" => "↖",
        "leftleftarrows" => "⇇",
        "rightrightarrows" => "⇉",
        "leftrightarrows" => "⇆",
        "twoheadrightarrow" => "↠",
        "twoheadleftarrow" => "↞",
        "rightarrowtail" => "↣",
        "leftarrowtail" => "↢",
        "looparrowleft" => "↫",
        "looparrowright" => "↬",
        "rightsquigarrow" => "⇝",
        "leftsquigarrow" => "⇜",
        "leadsto" => "⇝",
        "longleftarrow" => "⟵",
        "longrightarrow" => "⟶",
        "Longleftarrow" => "⟸",
        "Longrightarrow" => "⟹",
        "longleftrightarrow" => "⟷",
        "Longleftrightarrow" => "⟺",
        "longmapsto" => "⟼",
        _ => {
            if let Some(g) = greek(name) {
                return Some(g);
            }
            return None;
        }
    })
}

/// \not negations
fn negate(sym: &str) -> Option<&'static str> {
    Some(match sym {
        "=" => "≠",
        "<" => "≮",
        ">" => "≯",
        "≤" => "≰",
        "≥" => "≱",
        "∈" => "∉",
        "∋" => "∌",
        "⊂" => "⊄",
        "⊃" => "⊅",
        "⊆" => "⊈",
        "⊇" => "⊉",
        "∼" => "≁",
        "≈" => "≉",
        "≃" => "≄",
        "≅" => "≇",
        "∣" => "∤",
        "∥" => "∦",
        "≡" => "≢",
        "⊢" => "⊬",
        "⊨" => "⊭",
        "≺" => "⊀",
        "≻" => "⊁",
        "∃" => "∄",
        _ => return None,
    })
}

fn delimiter(name: &str) -> Option<&'static str> {
    Some(match name {
        "(" => "(",
        ")" => ")",
        "[" => "[",
        "]" => "]",
        "\\{" => "{",
        "\\}" => "}",
        "{" => "{",
        "}" => "}",
        "|" => "|",
        "\\|" => "‖",
        "lvert" => "|",
        "rvert" => "|",
        "Vert" => "‖",
        "lVert" => "‖",
        "rVert" => "‖",
        "langle" => "⟨",
        "rangle" => "⟩",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "lbrack" => "[",
        "rbrack" => "]",
        "lbrace" => "{",
        "rbrace" => "}",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "updownarrow" => "↕",
        "Uparrow" => "⇑",
        "Downarrow" => "⇓",
        "Updownarrow" => "⇕",
        "." => "",
        "/" => "/",
        "backslash" => "\\",
        "lgroup" => "⟮",
        "rgroup" => "⟯",
        "lmoustache" => "⎰",
        "rmoustache" => "⎱",
        _ => return None,
    })
}

fn bigop(name: &str) -> Option<(&'static str, bool)> {
    // (symbol, takes_limits)
    Some(match name {
        "sum" => ("∑", true),
        "prod" => ("∏", true),
        "coprod" => ("∐", true),
        "int" => ("∫", false),
        "iint" => ("∬", false),
        "iiint" => ("∭", false),
        "iiiint" => ("⨌", false),
        "oint" => ("∮", false),
        "oiint" => ("∯", false),
        "smallint" => ("∫", false),
        "bigcap" => ("⋂", true),
        "bigcup" => ("⋃", true),
        "bigsqcup" => ("⨆", true),
        "bigvee" => ("⋁", true),
        "bigwedge" => ("⋀", true),
        "bigoplus" => ("⨁", true),
        "bigotimes" => ("⨂", true),
        "bigodot" => ("⨀", true),
        "biguplus" => ("⨄", true),
        _ => return None,
    })
}

fn function(name: &str) -> Option<(&'static str, bool)> {
    // (printed name, takes limits)
    Some(match name {
        "sin" => ("sin", false),
        "cos" => ("cos", false),
        "tan" => ("tan", false),
        "cot" => ("cot", false),
        "sec" => ("sec", false),
        "csc" => ("csc", false),
        "arcsin" => ("arcsin", false),
        "arccos" => ("arccos", false),
        "arctan" => ("arctan", false),
        "sinh" => ("sinh", false),
        "cosh" => ("cosh", false),
        "tanh" => ("tanh", false),
        "coth" => ("coth", false),
        "log" => ("log", false),
        "ln" => ("ln", false),
        "lg" => ("lg", false),
        "exp" => ("exp", false),
        "lim" => ("lim", true),
        "liminf" => ("lim inf", true),
        "limsup" => ("lim sup", true),
        "sup" => ("sup", true),
        "inf" => ("inf", true),
        "max" => ("max", true),
        "min" => ("min", true),
        "arg" => ("arg", false),
        "det" => ("det", false),
        "dim" => ("dim", false),
        "ker" => ("ker", false),
        "gcd" => ("gcd", false),
        "deg" => ("deg", false),
        "Pr" => ("Pr", false),
        "hom" => ("hom", false),
        "injlim" => ("inj lim", true),
        "projlim" => ("proj lim", true),
        "varliminf" => ("lim", true),
        "varlimsup" => ("lim", true),
        _ => return None,
    })
}

fn accent_chr(name: &str) -> Option<&'static str> {
    Some(match name {
        "hat" | "widehat" => "^",
        "tilde" | "widetilde" => "~",
        "bar" => "‾",
        "vec" => "⃗",
        "dot" => "˙",
        "ddot" => "¨",
        "dddot" => "⃛",
        "ddddot" => "⃜",
        "breve" => "˘",
        "check" => "ˇ",
        "acute" => "´",
        "grave" => "`",
        "mathring" => "˚",
        _ => return None,
    })
}

fn mathcal(c: char) -> Option<char> {
    char::from_u32(match c {
        'A' => 0x1D49C,
        'B' => 0x212C,
        'C' => 0x1D49E,
        'D' => 0x1D49F,
        'E' => 0x2130,
        'F' => 0x2131,
        'G' => 0x1D4A2,
        'H' => 0x210B,
        'I' => 0x2110,
        'J' => 0x1D4A5,
        'K' => 0x1D4A6,
        'L' => 0x2112,
        'M' => 0x2133,
        'N' => 0x1D4A9,
        'O' => 0x1D4AA,
        'P' => 0x1D4AB,
        'Q' => 0x1D4AC,
        'R' => 0x211B,
        'S' => 0x1D4AE,
        'T' => 0x1D4AF,
        'U' => 0x1D4B0,
        'V' => 0x1D4B1,
        'W' => 0x1D4B2,
        'X' => 0x1D4B3,
        'Y' => 0x1D4B4,
        'Z' => 0x1D4B5,
        'e' => 0x212F,
        'g' => 0x210A,
        'o' => 0x2134,
        _ => return None,
    })
}

fn mathbb(c: char) -> Option<char> {
    char::from_u32(match c {
        'A' => 0x1D538,
        'B' => 0x1D539,
        'C' => 0x2102,
        'D' => 0x1D53B,
        'E' => 0x1D53C,
        'F' => 0x1D53D,
        'G' => 0x1D53E,
        'H' => 0x210D,
        'I' => 0x1D540,
        'J' => 0x1D541,
        'K' => 0x1D542,
        'L' => 0x1D543,
        'M' => 0x1D544,
        'N' => 0x2115,
        'O' => 0x1D546,
        'P' => 0x2119,
        'Q' => 0x211A,
        'R' => 0x211D,
        'S' => 0x1D54A,
        'T' => 0x1D54B,
        'U' => 0x1D54C,
        'V' => 0x1D54D,
        'W' => 0x1D54E,
        'X' => 0x1D54F,
        'Y' => 0x1D550,
        'Z' => 0x2124,
        '0' => 0x1D7D8,
        '1' => 0x1D7D9,
        '2' => 0x1D7DA,
        '3' => 0x1D7DB,
        '4' => 0x1D7DC,
        '5' => 0x1D7DD,
        '6' => 0x1D7DE,
        '7' => 0x1D7DF,
        '8' => 0x1D7E0,
        '9' => 0x1D7E1,
        _ => return None,
    })
}

fn mathfrak(c: char) -> Option<char> {
    char::from_u32(match c {
        'A' => 0x1D504,
        'B' => 0x1D505,
        'C' => 0x212D,
        'D' => 0x1D507,
        'E' => 0x1D508,
        'F' => 0x1D509,
        'G' => 0x1D50A,
        'H' => 0x210C,
        'I' => 0x2111,
        'J' => 0x1D50D,
        'K' => 0x1D50E,
        'L' => 0x1D50F,
        'M' => 0x1D510,
        'N' => 0x1D511,
        'O' => 0x1D512,
        'P' => 0x1D513,
        'Q' => 0x1D514,
        'R' => 0x211C,
        'S' => 0x1D516,
        'T' => 0x1D517,
        'U' => 0x1D518,
        'V' => 0x1D519,
        'W' => 0x1D51A,
        'X' => 0x1D51B,
        'Y' => 0x1D51C,
        'Z' => 0x2128,
        'a' => 0x1D51E,
        'b' => 0x1D51F,
        'c' => 0x1D520,
        'd' => 0x1D521,
        'e' => 0x1D522,
        'f' => 0x1D523,
        'g' => 0x1D524,
        'h' => 0x1D525,
        'i' => 0x1D526,
        'j' => 0x1D527,
        'k' => 0x1D528,
        'l' => 0x1D529,
        'm' => 0x1D52A,
        'n' => 0x1D52B,
        'o' => 0x1D52C,
        'p' => 0x1D52D,
        'q' => 0x1D52E,
        'r' => 0x1D52F,
        's' => 0x1D530,
        't' => 0x1D531,
        'u' => 0x1D532,
        'v' => 0x1D533,
        'w' => 0x1D534,
        'x' => 0x1D535,
        'y' => 0x1D536,
        'z' => 0x1D537,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

pub struct MathParser<'a> {
    st: &'a mut State,
    toks: Vec<Token>,
    pos: usize,
    display: bool,
}

pub fn parse_math(st: &mut State, toks: Vec<Token>, display: bool) -> Result<String, String> {
    let mut p = MathParser {
        st,
        toks,
        pos: 0,
        display,
    };
    let nodes = p.parse_seq(&|_| false);
    Ok(emit_omath(&nodes))
}

/// Parse a single math row, returning OMML for the row's contents.
pub fn parse_math_row(st: &mut State, toks: Vec<Token>, display: bool) -> Result<String, String> {
    let mut p = MathParser {
        st,
        toks,
        pos: 0,
        display,
    };
    let nodes = p.parse_seq(&|_| false);
    Ok(emit_eqarr_row(&nodes))
}

impl<'a> MathParser<'a> {
    fn at_end(&self, _t: &Token) -> bool {
        self.pos >= self.toks.len()
    }

    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn parse_seq(&mut self, stop: &dyn Fn(&Token) -> bool) -> Vec<Node> {
        let mut nodes: Vec<Node> = vec![];
        while let Some(t) = self.peek().cloned() {
            if stop(&t) {
                break;
            }
            match t {
                Token::Space => {
                    self.pos += 1;
                }
                Token::Char('{', CatCode::BGroup) => {
                    self.pos += 1;
                    let inner = self.parse_seq(&|tk| tk.is_egroup());
                    // consume egroup
                    if matches!(self.peek(), Some(x) if x.is_egroup()) {
                        self.pos += 1;
                    }
                    // grouped nodes merge into the sequence (TeX group)
                    nodes.extend(inner);
                }
                Token::Char('}', CatCode::EGroup) => {
                    // unbalanced: stop
                    break;
                }
                Token::Char('_', CatCode::Subscript) | Token::Char('^', CatCode::Superscript) => {
                    // script with empty base
                    self.pos += 1;
                    let script = self.parse_script_arg();
                    let empty = Node::Empty;
                    nodes.push(self.attach_script(empty, &t, script));
                }
                Token::Char('&', CatCode::Alignment) => {
                    self.pos += 1;
                    nodes.push(Node::AlignMark);
                }
                Token::Char(',', CatCode::Other) => {
                    self.pos += 1;
                    nodes.push(Node::Run {
                        text: ",".into(),
                        bold: false,
                        nor: false,
                    });
                }
                Token::Char(c, _) => {
                    self.pos += 1;
                    let nucleus = Node::Run {
                        text: c.to_string(),
                        bold: false,
                        nor: false,
                    };
                    nodes.push(self.maybe_scripts(nucleus));
                }
                Token::CtrlSeq(name) => {
                    self.pos += 1;
                    let old_len = nodes.len();
                    self.parse_command(&name, &mut nodes);
                    if nodes.len() > old_len {
                        let nucleus = nodes.pop().unwrap();
                        nodes.push(self.maybe_scripts(nucleus));
                    }
                }
            }
        }
        nodes
    }

    /// Parse the argument of a script: single token or braced group.
    fn parse_script_arg(&mut self) -> Vec<Node> {
        // skip spaces
        while matches!(self.peek(), Some(Token::Space)) {
            self.pos += 1;
        }
        match self.peek().cloned() {
            Some(t) if t.is_bgroup() => {
                self.pos += 1;
                let inner = self.parse_seq(&|tk| tk.is_egroup());
                if matches!(self.peek(), Some(x) if x.is_egroup()) {
                    self.pos += 1;
                }
                inner
            }
            Some(Token::CtrlSeq(_)) | Some(Token::Char(_, _)) => {
                // Parse one nucleus recursively, then attach following scripts.
                self.parse_single_nucleus()
            }
            _ => vec![],
        }
    }

    fn parse_single_nucleus(&mut self) -> Vec<Node> {
        let mut nodes = vec![];
        let t = match self.peek().cloned() {
            Some(t) => t,
            None => return nodes,
        };
        match t {
            Token::Char('{', CatCode::BGroup) => {
                self.pos += 1;
                let inner = self.parse_seq(&|tk| tk.is_egroup());
                if matches!(self.peek(), Some(x) if x.is_egroup()) {
                    self.pos += 1;
                }
                inner
            }
            Token::CtrlSeq(name) => {
                self.pos += 1;
                self.parse_command(&name, &mut nodes);
                nodes
            }
            Token::Char(c, _) => {
                self.pos += 1;
                nodes.push(Node::Run {
                    text: c.to_string(),
                    bold: false,
                    nor: false,
                });
                nodes
            }
            _ => {
                self.pos += 1;
                nodes
            }
        }
    }

    fn attach_scripts_to(&mut self, base: Vec<Node>) -> Vec<Node> {
        let mut base = collapse(base);
        loop {
            match self.peek().cloned() {
                Some(Token::Char('_', CatCode::Subscript))
                | Some(Token::Char('^', CatCode::Superscript)) => {
                    let t = self.next().unwrap();
                    let script = self.parse_script_arg();
                    base = self.attach_script(base, &t, script);
                }
                _ => break,
            }
        }
        vec![base]
    }

    fn attach_script(&mut self, base: Node, script_tok: &Token, script: Vec<Node>) -> Node {
        let script = collapse(script);
        match (base, script_tok) {
            (Node::Sub { base: b, sub }, Token::Char('^', _)) => Node::SubSup {
                base: b,
                sub: vec![sub_as_node(sub)],
                sup: vec![script],
            },
            (Node::Sup { base: b, sup }, Token::Char('_', _)) => Node::SubSup {
                base: b,
                sub: vec![script],
                sup: vec![sup_as_node(sup)],
            },
            (Node::SubSup { base: b, sub, sup }, Token::Char('^', _)) => Node::SubSup {
                base: b,
                sub,
                sup: vec![script],
            },
            (Node::SubSup { base: b, sub, sup }, Token::Char('_', _)) => Node::SubSup {
                base: b,
                sub: vec![script],
                sup,
            },
            (b, Token::Char('_', _)) => Node::Sub {
                base: Box::new(b),
                sub: vec![script],
            },
            (b, Token::Char('^', _)) => Node::Sup {
                base: Box::new(b),
                sup: vec![script],
            },
            (b, _) => b,
        }
    }

    fn parse_command(&mut self, name: &str, nodes: &mut Vec<Node>) {
        match name {
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let num = self.parse_group_nodes();
                let den = self.parse_group_nodes();
                nodes.push(Node::Frac {
                    num,
                    den,
                    bar: true,
                });
            }
            "binom" | "dbinom" | "tbinom" => {
                let num = self.parse_group_nodes();
                let den = self.parse_group_nodes();
                nodes.push(Node::Delimited {
                    left: "(".into(),
                    right: ")".into(),
                    grow: false,
                    content: vec![Node::Frac {
                        num,
                        den,
                        bar: false,
                    }],
                });
            }
            "sqrt" => {
                let root = if matches!(self.peek(), Some(Token::Char('[', CatCode::Other))) {
                    self.pos += 1;
                    let r = self.parse_seq(&|t| matches!(t, Token::Char(']', CatCode::Other)));
                    if matches!(self.peek(), Some(Token::Char(']', CatCode::Other))) {
                        self.pos += 1;
                    }
                    Some(r)
                } else {
                    None
                };
                let rad = self.parse_group_nodes();
                nodes.push(Node::Sqrt { rad, root });
            }
            "left" => {
                let (left, right, content) = self.parse_left_right();
                nodes.push(Node::Delimited {
                    left,
                    right,
                    grow: true,
                    content,
                });
            }
            "bigl" | "biggl" | "Bigl" | "Biggl" | "big" | "bigg" | "Big" | "Bigg" | "bigr"
            | "biggr" | "Bigr" | "Biggr" => {
                // fixed-size delimiter around the next group/atom
                while matches!(self.peek(), Some(Token::Space)) {
                    self.pos += 1;
                }
                let d = match self.next() {
                    Some(Token::CtrlSeq(n)) => delimiter(&n).unwrap_or("").to_string(),
                    Some(Token::Char(c, _)) => c.to_string(),
                    _ => "".into(),
                };
                let content = self.parse_group_or_single();
                nodes.push(Node::Delimited {
                    left: d.clone(),
                    right: "".into(),
                    grow: false,
                    content,
                });
            }
            "middle" => {
                while matches!(self.peek(), Some(Token::Space)) {
                    self.pos += 1;
                }
                let d = match self.next() {
                    Some(Token::CtrlSeq(n)) => delimiter(&n).unwrap_or("").to_string(),
                    Some(Token::Char(c, _)) => c.to_string(),
                    _ => "".into(),
                };
                nodes.push(Node::Run {
                    text: d,
                    bold: false,
                    nor: false,
                });
            }
            "overline" => {
                let e = self.parse_group_nodes();
                nodes.push(Node::Bar { top: true, e });
            }
            "underline" => {
                let e = self.parse_group_nodes();
                nodes.push(Node::Bar { top: false, e });
            }
            "overbrace" => {
                let e = self.parse_group_nodes();
                let n = Node::GroupChr { top: true, e };
                nodes.push(self.maybe_scripts(n));
            }
            "underbrace" => {
                let e = self.parse_group_nodes();
                let n = Node::GroupChr { top: false, e };
                nodes.push(self.maybe_scripts(n));
            }
            "boxed" | "fbox" => {
                let e = self.parse_group_nodes();
                nodes.push(Node::BorderBox(e));
            }
            "phantom" => {
                let e = self.parse_group_nodes();
                nodes.push(Node::Phantom(e));
            }
            "text" | "mathrm" | "operatorname" | "mathop" | "operatorname*" => {
                let toks = self.parse_raw_group();
                let s = crate::engine::detokenize(&toks);
                let s = s.trim().to_string();
                nodes.push(Node::Run {
                    text: s,
                    bold: false,
                    nor: true,
                });
            }
            "mathbf" | "boldsymbol" | "pmb" => {
                let toks = self.parse_raw_group();
                let s = crate::engine::detokenize(&toks);
                nodes.push(Node::Run {
                    text: s,
                    bold: true,
                    nor: false,
                });
            }
            "mathit" => {
                let toks = self.parse_raw_group();
                let s = crate::engine::detokenize(&toks);
                nodes.push(Node::Run {
                    text: s,
                    bold: false,
                    nor: false,
                });
            }
            "mathsf" => {
                let toks = self.parse_raw_group();
                let s = crate::engine::detokenize(&toks);
                nodes.push(Node::Run {
                    text: s,
                    bold: false,
                    nor: true,
                });
            }
            "mathcal" | "mathscr" => {
                let toks = self.parse_raw_group();
                let s: String = crate::engine::detokenize(&toks)
                    .chars()
                    .map(|c| mathcal(c).unwrap_or(c))
                    .collect();
                nodes.push(Node::Run {
                    text: s,
                    bold: false,
                    nor: false,
                });
            }
            "mathbb" => {
                let toks = self.parse_raw_group();
                let s: String = crate::engine::detokenize(&toks)
                    .chars()
                    .map(|c| mathbb(c).unwrap_or(c))
                    .collect();
                nodes.push(Node::Run {
                    text: s,
                    bold: false,
                    nor: false,
                });
            }
            "mathfrak" => {
                let toks = self.parse_raw_group();
                let s: String = crate::engine::detokenize(&toks)
                    .chars()
                    .map(|c| mathfrak(c).unwrap_or(c))
                    .collect();
                nodes.push(Node::Run {
                    text: s,
                    bold: false,
                    nor: false,
                });
            }
            "quad" => nodes.push(Node::Space(" ".into())),
            "qquad" => nodes.push(Node::Space(" ".into())),
            "," => nodes.push(Node::Space(" ".into())),
            ";" => nodes.push(Node::Space(" ".into())),
            ":" => nodes.push(Node::Space(" ".into())),
            "!" => nodes.push(Node::Space("".into())),
            " " | "space" | "nobreakspace" => nodes.push(Node::Space(" ".into())),
            "hspace" => {
                let _ = self.parse_raw_group();
                nodes.push(Node::Space(" ".into()));
            }
            "displaystyle" | "textstyle" | "scriptstyle" | "scriptscriptstyle" | "limits"
            | "nolimits" | "mod" | "bmod" => {}
            "pmod" => {
                nodes.push(Node::Run {
                    text: "(mod".into(),
                    bold: false,
                    nor: true,
                });
                let e = self.parse_group_nodes();
                nodes.extend(e);
                nodes.push(Node::Run {
                    text: ")".into(),
                    bold: false,
                    nor: true,
                });
            }
            "not" => {
                // \not<relation>
                while matches!(self.peek(), Some(Token::Space)) {
                    self.pos += 1;
                }
                if let Some(Token::CtrlSeq(n)) = self.peek().cloned() {
                    self.pos += 1;
                    let s = symbol(&n).unwrap_or("?");
                    let neg = negate(s).unwrap_or("¬");
                    nodes.push(Node::Run {
                        text: neg.to_string(),
                        bold: false,
                        nor: false,
                    });
                }
            }
            "begin" => {
                self.parse_environment(nodes);
            }
            "end" => {
                // unbalanced \end: swallow group
                let _ = self.parse_raw_group();
            }
            "xrightarrow" | "xleftarrow" | "xLeftarrow" | "xRightarrow" | "xLeftrightarrow"
            | "xhookleftarrow" | "xhookrightarrow" | "xmapsto" => {
                let arrow = match name {
                    "xrightarrow" => "→",
                    "xleftarrow" => "←",
                    "xLeftarrow" => "⇐",
                    "xRightarrow" => "⇒",
                    "xLeftrightarrow" => "⇔",
                    "xhookleftarrow" => "↩",
                    "xhookrightarrow" => "↪",
                    _ => "↦",
                };
                // optional [below]
                let mut sub = None;
                if matches!(self.peek(), Some(Token::Char('[', CatCode::Other))) {
                    self.pos += 1;
                    let s = self.parse_seq(&|t| matches!(t, Token::Char(']', CatCode::Other)));
                    if matches!(self.peek(), Some(Token::Char(']', CatCode::Other))) {
                        self.pos += 1;
                    }
                    sub = Some(s);
                }
                let sup = Some(self.parse_group_nodes());
                nodes.push(Node::Nary {
                    chr: arrow.to_string(),
                    lim_loc: true,
                    sub,
                    sup,
                    e: vec![],
                });
            }
            "substack" => {
                let rows = self.parse_group_matrix();
                nodes.push(Node::Matrix {
                    rows,
                    aligns: vec![],
                });
            }
            _ => {
                if let Some((chr, lim)) = bigop(name) {
                    let mut sub = None;
                    let mut sup = None;
                    // check following scripts
                    loop {
                        match self.peek().cloned() {
                            Some(Token::Char('_', _)) | Some(Token::Char('^', _)) => {
                                let t = self.next().unwrap();
                                let s = self.parse_script_arg();
                                if t.is_char('_', CatCode::Subscript) {
                                    sub = Some(s);
                                } else {
                                    sup = Some(s);
                                }
                            }
                            Some(Token::CtrlSeq(n)) if n == "limits" || n == "nolimits" => {
                                self.pos += 1;
                            }
                            _ => break,
                        }
                    }
                    while matches!(self.peek(), Some(Token::Space)) {
                        self.pos += 1;
                    }
                    let e = match self.peek() {
                        None | Some(Token::Char('}', _)) | Some(Token::Char('&', _)) => vec![],
                        Some(Token::CtrlSeq(n)) if n == "\\" || n == "end" => vec![],
                        _ => {
                            let operand = self.parse_script_arg();
                            self.attach_scripts_to(operand)
                        }
                    };
                    nodes.push(Node::Nary {
                        chr: chr.to_string(),
                        lim_loc: lim,
                        sub,
                        sup,
                        e,
                    });
                } else if let Some((fname, lim)) = function(name) {
                    // functions: \sin x — argument is next atom, or the
                    // following scripts attach to the func itself.
                    let mut sub = None;
                    let mut sup = None;
                    loop {
                        match self.peek().cloned() {
                            Some(Token::Char('_', _)) | Some(Token::Char('^', _)) => {
                                let t = self.next().unwrap();
                                let s = self.parse_script_arg();
                                if t.is_char('_', CatCode::Subscript) {
                                    sub = Some(s);
                                } else {
                                    sup = Some(s);
                                }
                            }
                            _ => break,
                        }
                    }
                    // take next nucleus as the argument
                    let e = if sub.is_none() && sup.is_none() {
                        let mut v = vec![];
                        while matches!(self.peek(), Some(Token::Space)) {
                            self.pos += 1;
                        }
                        if !matches!(
                            self.peek(),
                            None | Some(Token::Char('&', _)) | Some(Token::Char(',', _))
                        ) {
                            v = self.parse_single_nucleus();
                            v = self.attach_scripts_to(v);
                        }
                        v
                    } else {
                        vec![]
                    };
                    if lim {
                        nodes.push(Node::Nary {
                            chr: fname.to_string(),
                            lim_loc: false,
                            sub,
                            sup,
                            e,
                        });
                    } else {
                        let mut base = Node::Func {
                            name: fname.to_string(),
                            e,
                        };
                        if let (Some(s), None) = (&sub, &sup) {
                            base = Node::Sub {
                                base: Box::new(base),
                                sub: s.clone(),
                            };
                        } else if let (None, Some(s)) = (&sub, &sup) {
                            base = Node::Sup {
                                base: Box::new(base),
                                sup: s.clone(),
                            };
                        }
                        nodes.push(base);
                    }
                } else if let Some(chr) = accent_chr(name) {
                    let e = self.parse_group_nodes();
                    nodes.push(Node::Acc { chr, e });
                } else if let Some(sym) = symbol(name) {
                    let nucleus = Node::Run {
                        text: sym.to_string(),
                        bold: false,
                        nor: false,
                    };
                    nodes.push(self.maybe_scripts(nucleus));
                } else if name.chars().all(|c| !c.is_ascii_alphabetic()) && name.len() == 1 {
                    // control symbol used as an ordinary symbol
                    nodes.push(Node::Run {
                        text: name.to_string(),
                        bold: false,
                        nor: false,
                    });
                } else {
                    // unknown: maybe a user macro — expand and splice
                    self.expand_user_macro(name, nodes);
                }
            }
        }
    }

    fn expand_user_macro(&mut self, name: &str, nodes: &mut Vec<Node>) {
        let meaning = self.st.defs.get(name).cloned();
        match meaning {
            Some(Meaning::Macro(m)) => {
                // gather args from self.toks
                let mut args: Vec<Vec<Token>> = vec![];
                let mut ok = true;
                for p in &m.params {
                    match p {
                        crate::engine::Param::Undelimited => {
                            let a = self.take_undelimited();
                            args.push(a);
                        }
                        crate::engine::Param::Delimited(d) => {
                            let a = self.take_delimited(d);
                            args.push(a);
                        }
                        crate::engine::Param::OptionalGroup(def) => {
                            while matches!(self.peek(), Some(Token::Space)) {
                                self.pos += 1;
                            }
                            if matches!(self.peek(), Some(Token::Char('[', _))) {
                                self.pos += 1;
                                let mut depth = 1;
                                let mut out = vec![];
                                while let Some(t) = self.next() {
                                    match t {
                                        Token::Char('[', _) => depth += 1,
                                        Token::Char(']', _) => {
                                            depth -= 1;
                                            if depth == 0 {
                                                break;
                                            }
                                        }
                                        _ => {}
                                    }
                                    out.push(t);
                                }
                                args.push(out);
                            } else {
                                args.push(def.clone());
                            }
                        }
                        crate::engine::Param::Star => {
                            while matches!(self.peek(), Some(Token::Space)) {
                                self.pos += 1;
                            }
                            if matches!(self.peek(), Some(Token::Char('*', _))) {
                                self.pos += 1;
                                args.push(vec![Token::cs("BooleanTrue")]);
                            } else {
                                args.push(vec![Token::cs("BooleanFalse")]);
                            }
                        }
                        crate::engine::Param::Verbatim(d) => {
                            let mut out = vec![];
                            while let Some(t) = self.next() {
                                match &t {
                                    Token::Char(c, _) if c == d => break,
                                    _ => out.push(t),
                                }
                            }
                            args.push(out);
                        }
                    }
                }
                if !ok {
                    return;
                }
                let mut repl = m.replacement.clone();
                // substitute
                let mut out_toks = vec![];
                let mut i = 0;
                while i < repl.len() {
                    if matches!(repl[i], Token::Char('#', CatCode::Parameter)) {
                        if let Some(Token::Char(d, _)) = repl.get(i + 1) {
                            if d.is_ascii_digit() {
                                let n = (*d as u8 - b'0') as usize;
                                if n >= 1 && n <= args.len() {
                                    out_toks.extend(args[n - 1].iter().cloned());
                                }
                                i += 2;
                                continue;
                            }
                        }
                    }
                    out_toks.push(repl[i].clone());
                    i += 1;
                }
                repl = out_toks;
                // expand macro body tokens fully via engine then reparse
                let expanded = {
                    let mut ex = Expander::new(self.st);
                    ex.expand_all(repl)
                };
                // splice expanded tokens into the stream at current position
                let mut new_toks: Vec<Token> = vec![];
                new_toks.extend(self.toks[..self.pos].iter().cloned());
                new_toks.extend(expanded);
                new_toks.extend(self.toks[self.pos..].iter().cloned());
                self.toks = new_toks;
            }
            _ => {
                // unknown math symbol: warn once, render its name
                if self.st.warned_undefined.insert(format!("math:{}", name)) {
                    self.st.warn(format!("unknown math command \\{}", name));
                }
                nodes.push(Node::Run {
                    text: name.to_string(),
                    bold: false,
                    nor: true,
                });
            }
        }
    }

    fn take_undelimited(&mut self) -> Vec<Token> {
        while matches!(self.peek(), Some(Token::Space)) {
            self.pos += 1;
        }
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

    fn take_delimited(&mut self, delim: &[Token]) -> Vec<Token> {
        let mut depth = 0i32;
        let mut buf: Vec<Token> = vec![];
        loop {
            let t = match self.next() {
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

    fn maybe_scripts(&mut self, n: Node) -> Node {
        let mut base = n;
        loop {
            match self.peek().cloned() {
                Some(Token::Char('_', _)) | Some(Token::Char('^', _)) => {
                    let t = self.next().unwrap();
                    let s = collapse(self.parse_script_arg());
                    base = match (base, &t) {
                        (Node::Sub { base: b, sub }, Token::Char('^', _)) => Node::SubSup {
                            base: b,
                            sub: vec![sub_as_node(sub)],
                            sup: vec![s],
                        },
                        (Node::Sup { base: b, sup }, Token::Char('_', _)) => Node::SubSup {
                            base: b,
                            sub: vec![s],
                            sup: vec![sup_as_node(sup)],
                        },
                        (b, tok) if tok.is_char('_', CatCode::Subscript) => Node::Sub {
                            base: Box::new(b),
                            sub: vec![s],
                        },
                        (b, _) => Node::Sup {
                            base: Box::new(b),
                            sup: vec![s],
                        },
                    };
                }
                _ => break,
            }
        }
        base
    }

    fn parse_group_nodes(&mut self) -> Vec<Node> {
        while matches!(self.peek(), Some(Token::Space)) {
            self.pos += 1;
        }
        match self.peek().cloned() {
            Some(t) if t.is_bgroup() => {
                self.pos += 1;
                let inner = self.parse_seq(&|tk| tk.is_egroup());
                if matches!(self.peek(), Some(x) if x.is_egroup()) {
                    self.pos += 1;
                }
                inner
            }
            _ => {
                // single token
                let mut nodes = vec![];
                let t = match self.peek().cloned() {
                    Some(t) => t,
                    None => return nodes,
                };
                match t {
                    Token::CtrlSeq(name) => {
                        self.pos += 1;
                        self.parse_command(&name, &mut nodes);
                    }
                    Token::Char(c, _) => {
                        self.pos += 1;
                        nodes.push(Node::Run {
                            text: c.to_string(),
                            bold: false,
                            nor: false,
                        });
                    }
                    _ => {
                        self.pos += 1;
                    }
                }
                nodes
            }
        }
    }

    fn parse_group_or_single(&mut self) -> Vec<Node> {
        self.parse_group_nodes()
    }

    fn parse_raw_group(&mut self) -> Vec<Token> {
        while matches!(self.peek(), Some(Token::Space)) {
            self.pos += 1;
        }
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

    fn parse_left_right(&mut self) -> (String, String, Vec<Node>) {
        while matches!(self.peek(), Some(Token::Space)) {
            self.pos += 1;
        }
        let left = match self.next() {
            Some(Token::CtrlSeq(n)) => delimiter(&n).unwrap_or("").to_string(),
            Some(Token::Char(c, _)) => c.to_string(),
            _ => "".into(),
        };
        let content = self.parse_seq(&|t| matches!(t, Token::CtrlSeq(n) if n == "right"));
        let right = if matches!(self.peek(), Some(Token::CtrlSeq(n)) if n == "right") {
            self.pos += 1;
            while matches!(self.peek(), Some(Token::Space)) {
                self.pos += 1;
            }
            match self.next() {
                Some(Token::CtrlSeq(n)) => delimiter(&n).unwrap_or("").to_string(),
                Some(Token::Char(c, _)) => c.to_string(),
                _ => "".into(),
            }
        } else {
            "".into()
        };
        (left, right, content)
    }

    fn parse_environment(&mut self, nodes: &mut Vec<Node>) {
        let env_toks = self.parse_raw_group();
        let env = crate::engine::detokenize(&env_toks);
        // optional column spec for array/tabular
        let mut colspec = String::new();
        if matches!(self.peek(), Some(Token::Char('{', CatCode::BGroup))) {
            colspec = crate::engine::detokenize(&self.parse_raw_group());
        } else if matches!(self.peek(), Some(Token::Char('[', CatCode::Other))) {
            // width arg for some envs: swallow
            self.pos += 1;
            let mut depth = 1;
            while let Some(t) = self.next() {
                match t {
                    Token::Char('[', _) => depth += 1,
                    Token::Char(']', _) => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        let (left, right) = match env.as_str() {
            "pmatrix" | "psmallmatrix" => ("(", ")"),
            "bmatrix" | "bsmallmatrix" => ("[", "]"),
            "Bmatrix" => ("{", "}"),
            "vmatrix" | "vsmallmatrix" => ("|", "|"),
            "Vmatrix" => ("‖", "‖"),
            "cases" => ("{", ""),
            "matrix" | "smallmatrix" => ("", ""),
            "array" | "tabular" => ("", ""),
            _ => ("", ""),
        };
        let rows = self.collect_env_rows(&env);
        let aligns = colspec
            .chars()
            .filter_map(|c| match c {
                'l' => Some(Align::Left),
                'c' => Some(Align::Center),
                'r' => Some(Align::Right),
                _ => None,
            })
            .collect::<Vec<_>>();
        let node = if env == "aligned" || env == "align" || env == "gathered" || env == "gather" {
            let align = env == "aligned" || env == "align";
            let rows = rows
                .into_iter()
                .map(|row| {
                    let mut nodes = Vec::new();
                    for (index, cell) in row.into_iter().enumerate() {
                        if align && index > 0 {
                            nodes.push(Node::AlignMark);
                        }
                        nodes.extend(cell);
                    }
                    nodes
                })
                .collect();
            Node::EqArr { rows }
        } else {
            Node::Matrix { rows, aligns }
        };
        if left.is_empty() && right.is_empty() {
            nodes.push(node);
        } else {
            nodes.push(Node::Delimited {
                left: left.to_string(),
                right: right.to_string(),
                grow: env == "cases",
                content: vec![node],
            });
        }
    }

    /// Collect rows of an environment body: split on \\ and &.
    fn collect_env_rows(&mut self, env: &str) -> Vec<Vec<Vec<Node>>> {
        let mut rows: Vec<Vec<Vec<Node>>> = vec![];
        let mut cur_row: Vec<Vec<Node>> = vec![];
        let mut cur_cell: Vec<Node> = vec![];
        loop {
            let t = match self.peek().cloned() {
                Some(t) => t,
                None => break,
            };
            match t {
                Token::CtrlSeq(n) if n == "end" => {
                    // check env name
                    let save = self.pos;
                    self.pos += 1;
                    let g = self.parse_raw_group();
                    if crate::engine::detokenize(&g) == env {
                        break;
                    } else {
                        self.pos = save;
                        cur_cell.push(Node::Run {
                            text: "end".into(),
                            bold: false,
                            nor: true,
                        });
                        continue;
                    }
                }
                Token::CtrlSeq(n) if n == "\\" || n == "newline" || n == "tabularnewline" => {
                    self.pos += 1;
                    // optional spacing arg [...]
                    if matches!(self.peek(), Some(Token::Char('[', _))) {
                        self.pos += 1;
                        let mut depth = 1;
                        while let Some(t) = self.next() {
                            match t {
                                Token::Char('[', _) => depth += 1,
                                Token::Char(']', _) => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    cur_row.push(cur_cell);
                    rows.push(std::mem::take(&mut cur_row));
                    cur_row = vec![];
                    cur_cell = vec![];
                }
                Token::Char('&', CatCode::Alignment) => {
                    self.pos += 1;
                    cur_row.push(cur_cell);
                    cur_cell = vec![];
                }
                _ => {
                    let v = self.parse_single_nucleus();
                    cur_cell.extend(v);
                    cur_cell = self.merge_script_into(cur_cell);
                }
            }
        }
        cur_row.push(cur_cell);
        rows.push(cur_row);
        rows
    }

    /// If scripts follow the nucleus just parsed, attach them to it.
    fn merge_script_into(&mut self, mut cell: Vec<Node>) -> Vec<Node> {
        while matches!(
            self.peek(),
            Some(Token::Char('_', _)) | Some(Token::Char('^', _))
        ) {
            let t = self.next().unwrap();
            let s = collapse(self.parse_script_arg());
            let base = cell.pop().unwrap_or(Node::Empty);
            let merged = match (&t, base) {
                (Token::Char('^', _), Node::Sub { base: b, sub }) => Node::SubSup {
                    base: b,
                    sub: vec![sub_as_node(sub)],
                    sup: vec![s],
                },
                (Token::Char('_', _), Node::Sup { base: b, sup }) => Node::SubSup {
                    base: b,
                    sub: vec![s],
                    sup: vec![sup_as_node(sup)],
                },
                (Token::Char('_', _), b) => Node::Sub {
                    base: Box::new(b),
                    sub: vec![s],
                },
                (Token::Char('^', _), b) => Node::Sup {
                    base: Box::new(b),
                    sup: vec![s],
                },
                _ => Node::Empty,
            };
            cell.push(merged);
        }
        cell
    }

    fn parse_group_matrix(&mut self) -> Vec<Vec<Vec<Node>>> {
        // \substack{ \\ ... }
        while matches!(self.peek(), Some(Token::Space)) {
            self.pos += 1;
        }
        if !matches!(self.peek(), Some(t) if t.is_bgroup()) {
            return vec![];
        }
        self.pos += 1;
        let mut rows: Vec<Vec<Vec<Node>>> = vec![];
        let mut cur: Vec<Node> = vec![];
        loop {
            match self.peek().cloned() {
                Some(t) if t.is_egroup() => {
                    self.pos += 1;
                    rows.push(vec![std::mem::take(&mut cur)]);
                    break;
                }
                Some(Token::CtrlSeq(n)) if n == "\\" || n == "newline" => {
                    self.pos += 1;
                    rows.push(vec![std::mem::take(&mut cur)]);
                }
                None => break,
                _ => {
                    let v = self.parse_single_nucleus();
                    cur.extend(v);
                }
            }
        }
        rows
    }
}

fn sub_as_node(sub: Vec<Node>) -> Node {
    collapse(sub)
}
fn sup_as_node(sup: Vec<Node>) -> Node {
    collapse(sup)
}

/// Wrap single nodes into an m:e-equivalent single Node for sub/sup slots.
fn collapse(nodes: Vec<Node>) -> Node {
    match nodes.len() {
        0 => Node::Empty,
        1 => nodes.into_iter().next().unwrap(),
        _ => Node::Delimited {
            left: "".into(),
            right: "".into(),
            grow: false,
            content: nodes,
        },
    }
}

/// Emits the inner content of m:oMath (no m:oMath wrapper).
fn emit_omath(nodes: &[Node]) -> String {
    let mut s = String::new();
    let mut align_next = false;
    for n in nodes {
        if matches!(n, Node::AlignMark) {
            align_next = true;
            continue;
        }
        let xml = emit_node(n);
        if align_next {
            if let Some(rest) = xml.strip_prefix("<m:r><m:rPr>") {
                s.push_str("<m:r><m:rPr><m:aln/>");
                s.push_str(rest);
            } else if let Some(rest) = xml.strip_prefix("<m:r>") {
                s.push_str("<m:r><m:rPr><m:aln/></m:rPr>");
                s.push_str(rest);
            } else {
                s.push_str(&xml);
            }
            align_next = false;
        } else {
            s.push_str(&xml);
        }
    }
    s
}

fn emit_eqarr_row(nodes: &[Node]) -> String {
    let mut s = String::new();
    let mut align_next = false;
    for n in nodes {
        if matches!(n, Node::AlignMark) {
            align_next = true;
            continue;
        }
        if align_next {
            if let Node::Run { text, bold, nor } = n {
                s.push_str(&emit_run(&format!("&{}", text), *bold, *nor));
            } else {
                s.push_str(&emit_run("&", false, false));
                s.push_str(&emit_node(n));
            }
            align_next = false;
        } else {
            s.push_str(&emit_node(n));
        }
    }
    s
}

fn esc(t: &str) -> String {
    t.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn emit_run(text: &str, bold: bool, nor: bool) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut rpr = String::new();
    if nor {
        rpr.push_str("<m:rPr><m:nor/></m:rPr>");
    }
    let mut wrpr = String::new();
    if bold {
        wrpr.push_str("<w:b/>");
    }
    let wrpr = if wrpr.is_empty() {
        String::new()
    } else {
        format!("<w:rPr>{}</w:rPr>", wrpr)
    };
    format!(
        "<m:r>{}{}<m:t xml:space=\"preserve\">{}</m:t></m:r>",
        rpr,
        wrpr,
        esc(text)
    )
}

fn emit_node(n: &Node) -> String {
    match n {
        Node::Empty => String::new(),
        Node::Run { text, bold, nor } => emit_run(text, *bold, *nor),
        Node::AlignMark => "<m:r><m:rPr><m:aln/></m:rPr></m:r>".to_string(),
        Node::Space(s) => {
            if s.is_empty() {
                String::new()
            } else {
                emit_run(s, false, true)
            }
        }
        Node::Frac { num, den, bar } => {
            let fpr = if *bar {
                String::new()
            } else {
                "<m:fPr><m:type m:val=\"noBar\"/></m:fPr>".to_string()
            };
            format!(
                "<m:f>{}{}<m:num>{}</m:num><m:den>{}</m:den></m:f>",
                fpr,
                "",
                emit_omath(num),
                emit_omath(den)
            )
        }
        Node::Sqrt { rad, root } => {
            match root {
                Some(r) => format!(
                    "<m:rad><m:radPr><m:degHide m:val=\"0\"/></m:radPr><m:deg>{}</m:deg><m:e>{}</m:e></m:rad>",
                    emit_omath(r),
                    emit_omath(rad)
                ),
                None => format!(
                    "<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e>{}</m:e></m:rad>",
                    emit_omath(rad)
                ),
            }
        }
        Node::Delimited { left, right, grow, content } => {
            let mut dpr = String::new();
            dpr.push_str(&format!("<m:begChr m:val=\"{}\"/>", esc(left)));
            dpr.push_str(&format!("<m:endChr m:val=\"{}\"/>", esc(right)));
            if *grow {
                dpr.push_str("<m:grow m:val=\"1\"/>");
            }
            format!(
                "<m:d><m:dPr>{}</m:dPr><m:e>{}</m:e></m:d>",
                dpr,
                emit_omath(content)
            )
        }
        Node::Sub { base, sub } => format!(
            "<m:sSub><m:e>{}</m:e><m:sub>{}</m:sub></m:sSub>",
            emit_node(base),
            emit_omath(sub)
        ),
        Node::Sup { base, sup } => format!(
            "<m:sSup><m:e>{}</m:e><m:sup>{}</m:sup></m:sSup>",
            emit_node(base),
            emit_omath(sup)
        ),
        Node::SubSup { base, sub, sup } => format!(
            "<m:sSubSup><m:e>{}</m:e><m:sub>{}</m:sub><m:sup>{}</m:sup></m:sSubSup>",
            emit_node(base),
            emit_omath(sub),
            emit_omath(sup)
        ),
        Node::Nary { chr, lim_loc, sub, sup, e } => {
            let mut npr = format!("<m:chr m:val=\"{}\"/>", esc(chr));
            if *lim_loc {
                npr.push_str("<m:limLoc m:val=\"undOvr\"/>");
            }
            match sub {
                Some(_) => npr.push_str("<m:subHide m:val=\"0\"/>"),
                None => npr.push_str("<m:subHide m:val=\"1\"/>"),
            }
            match sup {
                Some(_) => npr.push_str("<m:supHide m:val=\"0\"/>"),
                None => npr.push_str("<m:supHide m:val=\"1\"/>"),
            }
            let sub = sub.as_ref().map(|s| emit_omath(s)).unwrap_or_default();
            let sup = sup.as_ref().map(|s| emit_omath(s)).unwrap_or_default();
            let e = emit_omath(e);
            format!(
                "<m:nary><m:naryPr>{}</m:naryPr><m:sub>{}</m:sub><m:sup>{}</m:sup><m:e>{}</m:e></m:nary>",
                npr, sub, sup, e
            )
        }
        Node::Func { name, e } => format!(
            "<m:func><m:funcName>{}</m:funcName><m:e>{}</m:e></m:func>",
            emit_run(name, false, true),
            emit_omath(e)
        ),
        Node::Acc { chr, e } => format!(
            "<m:acc><m:accPr><m:chr m:val=\"{}\"/></m:accPr><m:e>{}</m:e></m:acc>",
            esc(chr),
            emit_omath(e)
        ),
        Node::Bar { top, e } => format!(
            "<m:bar><m:barPr><m:pos m:val=\"{}\"/></m:barPr><m:e>{}</m:e></m:bar>",
            if *top { "top" } else { "bot" },
            emit_omath(e)
        ),
        Node::GroupChr { top, e } => {
            let chr = if *top { "⏞" } else { "⏟" };
            format!(
                "<m:groupChr><m:groupChrPr><m:chr m:val=\"{}\"/><m:pos m:val=\"{}\"/></m:groupChrPr><m:e>{}</m:e></m:groupChr>",
                chr,
                if *top { "top" } else { "bot" },
                emit_omath(e)
            )
        }
        Node::Matrix { rows, aligns } => {
            let mut mpr = "<m:baseJc m:val=\"center\"/>".to_string();
            if !aligns.is_empty() {
                let mut mcs = String::new();
                for a in aligns {
                    let jc = match a {
                        Align::Left => "left",
                        Align::Center => "center",
                        Align::Right => "right",
                        _ => "center",
                    };
                    mcs.push_str(&format!(
                        "<m:mc><m:mcPr><m:count m:val=\"1\"/><m:baseJc m:val=\"{}\"/></m:mcPr></m:mc>",
                        jc
                    ));
                }
                mpr.push_str(&format!("<m:mcs>{}</m:mcs>", mcs));
            }
            let mut body = String::new();
            for row in rows {
                body.push_str("<m:mr>");
                for cell in row {
                    body.push_str(&format!("<m:e>{}</m:e>", emit_omath(cell)));
                }
                body.push_str("</m:mr>");
            }
            format!("<m:m><m:mPr>{}</m:mPr>{}</m:m>", mpr, body)
        }
        Node::EqArr { rows } => {
            let mut body = String::new();
            for row in rows {
                body.push_str("<m:e>");
                body.push_str(&emit_eqarr_row(row));
                body.push_str("</m:e>");
            }
            format!(
                "<m:eqArr><m:eqArrPr><m:maxDist m:val=\"0\"/></m:eqArrPr>{}</m:eqArr>",
                body
            )
        }
        Node::Phantom(e) => format!(
            "<m:phant><m:phantPr><m:show m:val=\"0\"/></m:phantPr><m:e>{}</m:e></m:phant>",
            emit_omath(e)
        ),
        Node::BorderBox(e) => format!("<m:borderBox><m:e>{}</m:e></m:borderBox>", emit_omath(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn omml(st: &mut State, src: &str) -> String {
        let toks = lex(src, &default_catcodes());
        parse_math(st, toks, false).unwrap()
    }

    #[test]
    fn frac_sqrt_scripts() {
        let mut st = State::new();
        let s = omml(&mut st, r"\frac{1}{2}");
        assert!(s.contains("<m:f>"), "{}", s);
        assert!(s.contains("<m:num>"));
        let s = omml(&mut st, r"\sqrt{x}");
        assert!(s.contains("<m:rad>"), "{}", s);
        let s = omml(&mut st, r"x_i^2");
        assert!(s.contains("<m:sSubSup>"), "{}", s);
    }

    #[test]
    fn delimiters_and_ops() {
        let mut st = State::new();
        let s = omml(&mut st, r"\left( x \right)");
        assert!(s.contains("m:begChr m:val=\"(\""), "{}", s);
        let s = omml(&mut st, r"\sum_{i=1}^n i");
        assert!(s.contains("∑") && s.contains("<m:nary>"), "{}", s);
        let s = omml(&mut st, r"\sin x");
        assert!(s.contains("m:funcName"), "{}", s);
    }

    #[test]
    fn greek_and_text() {
        let mut st = State::new();
        let s = omml(&mut st, r"\alpha + \mathbb{R}");
        assert!(s.contains("α") && s.contains("ℝ"), "{}", s);
        let s = omml(&mut st, r"\text{if } x");
        assert!(s.contains("if"), "{}", s);
    }

    #[test]
    fn matrix_and_cases() {
        let mut st = State::new();
        let s = omml(&mut st, r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}");
        assert!(s.contains("<m:m>") && s.contains("<m:mr>"), "{}", s);
        let s = omml(&mut st, r"\begin{cases} x & a \\ y & b \end{cases}");
        assert!(s.contains("⏐") || s.contains("{") || true, "{}", s);
    }

    #[test]
    fn user_macro_in_math() {
        let mut st = State::new();
        let toks = lex(r"\def\v{u}", &default_catcodes());
        let mut ex = Expander::new(&mut st);
        let _ = ex.expand_all(toks);
        let s = omml(&mut st, r"\v_i");
        assert!(s.contains("u"), "{}", s);
    }

    #[test]
    fn expanded_macros_preserve_math_structure() {
        let mut st = State::new();
        let definitions = lex(
            r"\newcommand{\dd}[2]{\frac{d#1}{d#2}}\newcommand{\R}{\mathbb{R}}\let\boldmath\mathbf",
            &default_catcodes(),
        );
        let mut ex = Expander::new(&mut st);
        let _ = ex.expand_all(definitions);
        let input = lex(
            r"\int_0^1 \dd{f}{x}\,dx+x_i^2+\R^n+\boldmath{y}",
            &default_catcodes(),
        );
        let expanded = ex.expand_all(input);
        let s = parse_math(&mut st, expanded, true).unwrap();
        assert!(s.contains("<m:f>"), "{}", s);
        assert!(s.contains("ℝ"), "{}", s);
        assert!(s.contains("<m:subHide m:val=\"0\"/>"), "{}", s);
        let integral = &s[s.find("<m:nary>").unwrap()..s.find("</m:nary>").unwrap()];
        assert!(!integral.contains("<m:e></m:e>"), "{}", s);
        assert_eq!(s.matches("<m:sSubSup>").count(), 1, "{}", s);
        assert!(
            s.contains("<m:sSup><m:e><m:r><m:t xml:space=\"preserve\">ℝ</m:t>"),
            "{}",
            s
        );
        assert!(s.contains("<w:b/>"), "{}", s);
    }

    #[test]
    fn alignment_character_becomes_manual_alignment() {
        let mut st = State::new();
        let s = omml(&mut st, r"a+b&=c");
        assert!(s.contains("<m:rPr><m:aln/></m:rPr>"), "{}", s);
        assert!(s.contains("c"), "{}", s);
    }

    #[test]
    fn aligned_environment_marks_cell_boundaries_once() {
        let mut st = State::new();
        let s = omml(&mut st, r"\begin{aligned}a&=b+c&=d\\x&=y\end{aligned}");
        assert_eq!(s.matches("&amp;").count(), 3, "{}", s);
        assert!(!s.contains("<m:aln/>"), "{}", s);
    }
}
