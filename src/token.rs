//! TeX tokens and the input lexer (TeX's "mouth", first stage).
//!
//! The lexer implements TeX's category-code state machine:
//!   * control sequences (`\foo`, `\%`, control symbols, and `^^` escapes)
//!   * comment handling (`%` swallows to end of line; the line's newline is lost)
//!   * space gobbling after control words
//!   * blank lines become `\par` tokens
//!
//! The lexer is line-lazy: it converts characters to tokens on demand, so
//! catcode changes made by `\makeatletter` etc. apply to characters that have
//! not been tokenized yet — exactly like TeX.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatCode {
    Escape,      // 0  backslash
    BGroup,      // 1  {
    EGroup,      // 2  }
    MathShift,   // 3  $
    Alignment,   // 4  &
    EndOfLine,   // 5  (handled structurally by the line machine)
    Parameter,   // 6  #
    Superscript, // 7  ^
    Subscript,   // 8  _
    Ignored,     // 9
    Space,       // 10 space, tab
    Letter,      // 11 a-z A-Z
    Other,       // 12 digits, punctuation
    Active,      // 13 ~
    Comment,     // 14 %
    Invalid,     // 15
}

impl CatCode {
    pub fn from_u8(v: u8) -> Option<CatCode> {
        use CatCode::*;
        Some(match v {
            0 => Escape,
            1 => BGroup,
            2 => EGroup,
            3 => MathShift,
            4 => Alignment,
            6 => Parameter,
            7 => Superscript,
            8 => Subscript,
            9 => Ignored,
            10 => Space,
            11 => Letter,
            12 => Other,
            13 => Active,
            14 => Comment,
            15 => Invalid,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// Control sequence, name WITHOUT the leading backslash.
    /// For control symbols the name is the single char (e.g. "%", "{", " ").
    CtrlSeq(String),
    /// Character with its catcode.
    Char(char, CatCode),
    /// An explicit space token.
    Space,
}

impl Token {
    pub fn cs(name: &str) -> Token {
        Token::CtrlSeq(name.to_string())
    }
    pub fn is_cs(&self, name: &str) -> bool {
        matches!(self, Token::CtrlSeq(n) if n == name)
    }
    pub fn is_char(&self, ch: char, cat: CatCode) -> bool {
        matches!(self, Token::Char(c, k) if *c == ch && *k == cat)
    }
    pub fn is_bgroup(&self) -> bool {
        matches!(self, Token::Char(_, CatCode::BGroup))
    }
    pub fn is_egroup(&self) -> bool {
        matches!(self, Token::Char(_, CatCode::EGroup))
    }
    pub fn to_text(&self) -> String {
        match self {
            Token::CtrlSeq(n) => n.clone(),
            Token::Char(c, _) => c.to_string(),
            Token::Space => " ".into(),
        }
    }
}

pub type CatCodeTable = std::collections::HashMap<char, u8>;

/// Default LaTeX catcodes.
pub fn default_catcodes() -> CatCodeTable {
    let mut t = CatCodeTable::new();
    for c in 'a'..='z' {
        t.insert(c, 11);
    }
    for c in 'A'..='Z' {
        t.insert(c, 11);
    }
    for c in '0'..='9' {
        t.insert(c, 12);
    }
    for c in "()+,-./:;!?<>[]|\"'=@`".chars() {
        t.insert(c, 12);
    }
    t.insert('\\', 0);
    t.insert('{', 1);
    t.insert('}', 2);
    t.insert('$', 3);
    t.insert('&', 4);
    t.insert('#', 6);
    t.insert('^', 7);
    t.insert('_', 8);
    t.insert(' ', 10);
    t.insert('\t', 10);
    t.insert('~', 13);
    t.insert('%', 14);
    t
}

pub fn cat_of(cats: &CatCodeTable, c: char) -> u8 {
    if c == '\n' {
        return 5;
    }
    cats.get(&c).copied().unwrap_or(12)
}

#[derive(Clone, Copy, PartialEq)]
enum LState {
    NewLine,
    MidLine,
    SkipBlanks,
}

/// Line-lazy TeX lexer. Holds the input text and converts characters to
/// tokens on demand; the catcode table is consulted at conversion time, so
/// catcode changes apply to not-yet-tokenized characters.
pub struct Lexer {
    src: Vec<char>,
    pos: usize,
    state: LState,
}

impl Lexer {
    pub fn new(input: &str) -> Lexer {
        let normalized: String = input.replace("\r\n", "\n").replace('\r', "\n");
        Lexer {
            src: normalized.chars().collect(),
            pos: 0,
            state: LState::NewLine,
        }
    }

    fn peek(&self) -> Option<char> {
        self.src.get(self.pos).copied()
    }

    fn cat(&self, cats: &CatCodeTable, c: char) -> u8 {
        cat_of(cats, c)
    }

    fn skip_comment(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            self.pos += 1;
        }
        if self.peek() == Some('\n') {
            self.pos += 1;
        }
        self.state = LState::NewLine;
    }

    fn read_cs(&mut self, cats: &CatCodeTable) -> Token {
        match self.peek() {
            None => Token::cs(" "),
            Some(c) => {
                if self.cat(cats, c) == 11 {
                    // TeX: any catcode-11 character is a letter (e.g. @ after
                    // \makeatletter), not just ASCII a-zA-Z.
                    let mut name = String::new();
                    while let Some(c) = self.peek() {
                        if self.cat(cats, c) == 11 {
                            name.push(c);
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    self.state = LState::SkipBlanks;
                    Token::CtrlSeq(name)
                } else {
                    self.pos += 1;
                    if c == '^' {
                        // ^^X or ^^ab
                        if let Some(d) = self.peek() {
                            if d == '^' {
                                self.pos += 1;
                                if let Some(e) = self.peek() {
                                    if e.is_ascii_hexdigit() {
                                        let h1 = e;
                                        self.pos += 1;
                                        if let Some(h2) = self.peek() {
                                            if h2.is_ascii_hexdigit() {
                                                self.pos += 1;
                                                let s: String = [h1, h2].iter().collect();
                                                let v = u32::from_str_radix(&s, 16).unwrap_or(0);
                                                let v = if v >= 0x80 { v - 0x80 } else { v };
                                                return Token::Char(
                                                    char::from_u32(v).unwrap_or('\0'),
                                                    CatCode::Other,
                                                );
                                            }
                                        }
                                        let v = (h1 as u32) & 0x7f;
                                        return Token::Char(
                                            char::from_u32(v).unwrap_or('\0'),
                                            CatCode::Other,
                                        );
                                    } else {
                                        self.pos += 1;
                                        let v = (e as u32) ^ 64;
                                        return Token::Char(
                                            char::from_u32(v).unwrap_or('\0'),
                                            CatCode::Other,
                                        );
                                    }
                                }
                                return Token::Char('^', CatCode::Other);
                            }
                        }
                        return Token::Char('^', CatCode::Other);
                    }
                    if self.cat(cats, c) == 10 {
                        self.state = LState::SkipBlanks;
                        return Token::Space;
                    }
                    self.state = LState::MidLine;
                    Token::CtrlSeq(c.to_string())
                }
            }
        }
    }

    /// Pull the next token, using `cats` looked up at conversion time.
    pub fn next_tok(&mut self, cats: &CatCodeTable) -> Option<Token> {
        loop {
            let c = self.peek()?;
            if c == '\n' {
                self.pos += 1;
                match self.state {
                    LState::NewLine => {
                        return Some(Token::cs("par"));
                    }
                    _ => {
                        self.state = LState::NewLine;
                        match self.peek() {
                            None => continue,
                            Some('\n') => continue,
                            Some('%') => continue,
                            Some(_) => return Some(Token::Space),
                        }
                    }
                }
            }
            let cat = self.cat(cats, c);
            match cat {
                10 => {
                    self.pos += 1;
                    match self.state {
                        LState::MidLine => {
                            self.state = LState::SkipBlanks;
                            return Some(Token::Space);
                        }
                        _ => continue,
                    }
                }
                14 => {
                    self.skip_comment();
                    continue;
                }
                0 => {
                    self.pos += 1;
                    return Some(self.read_cs(cats));
                }
                _ => {
                    self.pos += 1;
                    self.state = LState::MidLine;
                    return Some(Token::Char(
                        c,
                        CatCode::from_u8(cat).unwrap_or(CatCode::Other),
                    ));
                }
            }
        }
    }
    /// Read a raw line (chars through the next '\n', newline excluded),
    /// bypassing all tokenization. Returns None at end of input.
    pub fn read_raw_line(&mut self) -> Option<String> {
        if self.pos >= self.src.len() {
            return None;
        }
        let start = self.pos;
        while self.pos < self.src.len() && self.src[self.pos] != '\n' {
            self.pos += 1;
        }
        let line: String = self.src[start..self.pos].iter().collect();
        if self.pos < self.src.len() {
            self.pos += 1;
        }
        Some(line)
    }

    /// Read one raw character, bypassing tokenization.
    pub fn read_raw_char(&mut self) -> Option<char> {
        if self.pos >= self.src.len() {
            return None;
        }
        let c = self.src[self.pos];
        self.pos += 1;
        Some(c)
    }
}

/// Eagerly tokenize a whole input string with a FIXED catcode table.
/// Used by tests and for macro-definition snippets; the main document goes
/// through the lazy line lexer inside the expander instead.
pub fn lex(input: &str, cats: &CatCodeTable) -> Vec<Token> {
    let mut lx = Lexer::new(input);
    let mut out = Vec::new();
    while let Some(t) = lx.next_tok(cats) {
        out.push(t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_word_gobbles_space() {
        let cats = default_catcodes();
        let toks = lex("a \\foo b", &cats);
        assert_eq!(
            toks,
            vec![
                Token::Char('a', CatCode::Letter),
                Token::Space,
                Token::cs("foo"),
                Token::Char('b', CatCode::Letter),
            ]
        );
    }

    #[test]
    fn blank_line_is_par() {
        let cats = default_catcodes();
        let toks = lex("a\n\nb", &cats);
        assert!(toks.iter().any(|t| t.is_cs("par")));
    }

    #[test]
    fn comment_swallows_newline() {
        let cats = default_catcodes();
        let toks = lex("a % comment\nb", &cats);
        assert!(!toks.iter().any(|t| t.is_cs("par")));
    }

    #[test]
    fn control_symbol() {
        let cats = default_catcodes();
        let toks = lex("\\% done", &cats);
        assert_eq!(toks[0], Token::cs("%"));
    }

    #[test]
    fn catcode_change_applies_later_in_stream() {
        // Simulates: \makeatletter then \@foo in the same line, lazily.
        let mut cats = default_catcodes();
        let mut lx = Lexer::new("\\makeatletter \\@foo x");
        // tokens: \makeatletter
        let t1 = lx.next_tok(&cats).unwrap();
        assert_eq!(t1, Token::cs("makeatletter"));
        // execute it (as the engine would)
        cats.insert('@', 11);
        let t2 = lx.next_tok(&cats).unwrap();
        assert_eq!(t2, Token::cs("@foo"));
        let _ = cats;
    }
}
