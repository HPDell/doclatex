//! BibTeX engine: .bib parsing, citation label generation, bibliography
//! rendering in several styles (plain/unsrt/alpha/abbrv, IEEE-like numeric,
//! and author-year for natbib).

use crate::model::{BibItem, Inline};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BibStyle {
    Plain,
    Unsrt,
    Alpha,
    Abbrv,
    Ieeetr,
    Apalike,
    Authoryear,
}

impl BibStyle {
    pub fn from_name(s: &str) -> BibStyle {
        match s {
            "unsrt" => BibStyle::Unsrt,
            "alpha" => BibStyle::Alpha,
            "abbrv" => BibStyle::Abbrv,
            "ieeetr" | "IEEEtran" | "plainnat" | "unsrtnat" => BibStyle::Ieeetr,
            "apalike" | "agsm" | "dcu" | "kluwer" => BibStyle::Apalike,
            "authoryear" => BibStyle::Authoryear,
            _ => BibStyle::Plain,
        }
    }
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            BibStyle::Plain | BibStyle::Unsrt | BibStyle::Abbrv | BibStyle::Ieeetr
        )
    }
}

#[derive(Clone, Debug)]
pub struct BibEntry {
    pub etype: String,
    pub key: String,
    pub fields: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct BibItemFull {
    pub key: String,
    pub label: String,
    pub number: usize,
    pub content: Vec<Inline>,
    pub anchor: String,
}

#[derive(Debug)]
pub struct BibState {
    pub style: BibStyle,
    pub style_name: String,
    pub files: Vec<String>,
    pub cited: Vec<String>,
    pub nocite_all: bool,
    pub entries: HashMap<String, BibEntry>,
    pub items: Vec<BibItemFull>,
}

impl BibState {
    pub fn new() -> BibState {
        BibState {
            style: BibStyle::Plain,
            style_name: "plain".into(),
            files: vec![],
            cited: vec![],
            nocite_all: false,
            entries: HashMap::new(),
            items: vec![],
        }
    }

    pub fn cite(&mut self, keys: &[String]) {
        for k in keys {
            if k == "*" {
                self.nocite_all = true;
                continue;
            }
            if !self.cited.contains(k) {
                self.cited.push(k.clone());
            }
        }
    }

    pub fn style_name(&self) -> String {
        self.style_name.clone()
    }

    /// Parse .bib files and build bibliography items per the active style.
    pub fn resolve(&mut self, jobname_dir: &std::path::Path) {
        // Load entries from files (merge; later files override).
        for f in &self.files {
            let path = if std::path::Path::new(f).exists() {
                std::path::PathBuf::from(f)
            } else {
                let p = jobname_dir.join(f);
                if p.exists() {
                    p
                } else {
                    let p2 = jobname_dir.join(format!("{}.bib", f));
                    if p2.exists() {
                        p2
                    } else {
                        continue;
                    }
                }
            };
            if let Ok(text) = std::fs::read_to_string(&path) {
                for e in parse_bib(&text) {
                    self.entries.insert(e.key.clone(), e);
                }
            }
        }

        let mut keys: Vec<String> = vec![];
        if self.nocite_all {
            let mut all: Vec<String> = self.entries.keys().cloned().collect();
            all.sort();
            keys.extend(all);
        }
        for k in &self.cited {
            if self.entries.contains_key(k) && !keys.contains(k) {
                keys.push(k.clone());
            }
        }

        // ordering
        match self.style {
            BibStyle::Unsrt | BibStyle::Ieeetr => {
                // order of citation
                keys.sort_by_key(|k| self.cited.iter().position(|c| c == k).unwrap_or(usize::MAX));
            }
            _ => {
                let mut keys2 = keys.clone();
                keys2.sort_by(|a, b| {
                    let ea = self.entries.get(a).unwrap();
                    let eb = self.entries.get(b).unwrap();
                    let sa = (
                        author_sort(ea).to_lowercase(),
                        year_of(eb).cmp(&year_of(ea)),
                    );
                    let sb = (
                        author_sort(eb).to_lowercase(),
                        year_of(ea).cmp(&year_of(eb)),
                    );
                    // primary: author; tie: year; then title
                    let r = sa.0.cmp(&sb.0);
                    if r != std::cmp::Ordering::Equal {
                        return r;
                    }
                    year_of(ea).cmp(&year_of(eb)).then_with(|| {
                        clean_text(ea.fields.get("title").map(|s| s.as_str()).unwrap_or("")).cmp(
                            &clean_text(eb.fields.get("title").map(|s| s.as_str()).unwrap_or("")),
                        )
                    })
                });
                keys = keys2;
            }
        }

        // numbers & labels
        let mut number_of: HashMap<String, usize> = HashMap::new();
        let mut label_of: HashMap<String, String> = HashMap::new();
        for (i, k) in keys.iter().enumerate() {
            number_of.insert(k.clone(), i + 1);
            let e = self.entries.get(k).unwrap();
            let label = match self.style {
                BibStyle::Alpha => alpha_label(e),
                BibStyle::Apalike | BibStyle::Authoryear => author_year_label(e),
                _ => format!("[{}]", i + 1),
            };
            label_of.insert(k.clone(), label);
        }

        // citation compression table for numeric styles
        self.items = keys
            .iter()
            .map(|k| {
                let e = self.entries.get(k).unwrap();
                BibItemFull {
                    key: k.clone(),
                    label: label_of.get(k).unwrap().clone(),
                    number: *number_of.get(k).unwrap(),
                    content: format_entry(e, self.style),
                    anchor: format!("bibitem-{}", sanitize_anchor(k)),
                }
            })
            .collect();
    }

    pub fn number_of(&self, key: &str) -> Option<usize> {
        self.items.iter().find(|i| i.key == key).map(|i| i.number)
    }

    pub fn label_of(&self, key: &str) -> Option<String> {
        self.items
            .iter()
            .find(|i| i.key == key)
            .map(|i| i.label.clone())
    }

    /// Render a \cite for keys: "[1,2]" numeric / "[ABC04]" / "(Author, 2024)".
    /// Returns (text, links).
    pub fn render_cite(&self, keys: &[String], textual: bool) -> (String, Vec<(String, String)>) {
        let mut links = vec![];
        let valid: Vec<String> = keys
            .iter()
            .filter(|k| self.items.iter().any(|i| i.key == **k))
            .cloned()
            .collect();
        for k in &valid {
            if let Some(it) = self.items.iter().find(|i| i.key == *k) {
                links.push((format!("#{}", it.anchor), it.label.clone()));
            }
        }
        if valid.is_empty() {
            return ("[?]".to_string(), links);
        }
        if self.style.is_numeric() {
            let mut nums: Vec<usize> = valid.iter().filter_map(|k| self.number_of(k)).collect();
            nums.sort();
            let s = compress_ranges(&nums);
            (format!("[{}]", s), links)
        } else if textual {
            // \citet: Author (Year)
            let mut v = vec![];
            for k in &valid {
                let e = self.entries.get(k).unwrap();
                v.push(format!("{} ({})", short_authors(e), year_of(e)));
            }
            (v.join("; "), links)
        } else {
            // \citep: (Author, Year; Author2, Year2)
            let mut v = vec![];
            for k in &valid {
                let e = self.entries.get(k).unwrap();
                v.push(format!("{}, {}", short_authors(e), year_of(e)));
            }
            (format!("({})", v.join("; ")), links)
        }
    }

    pub fn to_model_items(&self) -> Vec<BibItem> {
        self.items
            .iter()
            .map(|i| BibItem {
                key: i.key.clone(),
                label: i.label.clone(),
                content: i.content.clone(),
            })
            .collect()
    }
}

pub fn sanitize_anchor(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.len() > 36 {
        out.truncate(36);
    }
    out
}

/// "[1,2,3]" -> "[1-3]"
fn compress_ranges(nums: &[usize]) -> String {
    let mut out = vec![];
    let mut i = 0;
    while i < nums.len() {
        let mut j = i;
        while j + 1 < nums.len() && nums[j + 1] == nums[j] + 1 {
            j += 1;
        }
        if j - i >= 2 {
            out.push(format!("{}-{}", nums[i], nums[j]));
        } else {
            for k in i..=j {
                out.push(nums[k].to_string());
            }
        }
        i = j + 1;
    }
    out.join(",")
}

// ---------------------------------------------------------------------------
// .bib parsing
// ---------------------------------------------------------------------------

pub fn parse_bib(text: &str) -> Vec<BibEntry> {
    let chars: Vec<char> = text.chars().collect();
    let mut pos = 0;
    let mut entries = vec![];
    let mut strings: HashMap<String, String> = HashMap::new();
    while pos < chars.len() {
        // find '@'
        while pos < chars.len() && chars[pos] != '@' {
            pos += 1;
        }
        if pos >= chars.len() {
            break;
        }
        pos += 1;
        let mut etype = String::new();
        while pos < chars.len() && (chars[pos].is_ascii_alphanumeric()) {
            etype.push(chars[pos]);
            pos += 1;
        }
        let etype_lc = etype.to_lowercase();
        // skip whitespace and comments
        while pos < chars.len() && (chars[pos].is_whitespace() || chars[pos] == '%') {
            if chars[pos] == '%' {
                while pos < chars.len() && chars[pos] != '\n' {
                    pos += 1;
                }
            } else {
                pos += 1;
            }
        }
        if etype_lc == "string" {
            if pos < chars.len() && chars[pos] == '{' {
                pos += 1;
                let (name, val, np) = parse_bib_field(&chars, pos, &strings);
                pos = np;
                if let (Some(n), Some(v)) = (name, val) {
                    strings.insert(n, v);
                }
            }
            continue;
        }
        if etype_lc == "preamble" || etype_lc == "comment" {
            if pos < chars.len() && chars[pos] == '{' {
                let mut depth = 0;
                while pos < chars.len() {
                    if chars[pos] == '{' {
                        depth += 1;
                    } else if chars[pos] == '}' {
                        depth -= 1;
                        if depth == 0 {
                            pos += 1;
                            break;
                        }
                    }
                    pos += 1;
                }
            }
            continue;
        }
        if pos >= chars.len() || chars[pos] != '{' {
            continue;
        }
        pos += 1;
        // entry key up to ','
        let mut key = String::new();
        while pos < chars.len() && chars[pos] != ',' {
            if !chars[pos].is_whitespace() {
                key.push(chars[pos]);
            }
            pos += 1;
        }
        if pos < chars.len() {
            pos += 1; // skip ','
        }
        let mut fields = HashMap::new();
        loop {
            // skip whitespace
            while pos < chars.len() && chars[pos].is_whitespace() {
                pos += 1;
            }
            if pos >= chars.len() || chars[pos] == '}' {
                if pos < chars.len() {
                    pos += 1;
                }
                break;
            }
            // read field name
            let mut fname = String::new();
            while pos < chars.len() && (chars[pos].is_ascii_alphanumeric() || chars[pos] == '_') {
                fname.push(chars[pos]);
                pos += 1;
            }
            while pos < chars.len() && chars[pos].is_whitespace() {
                pos += 1;
            }
            if pos < chars.len() && chars[pos] == '=' {
                pos += 1;
            }
            let (val, np) = parse_bib_value(&chars, pos, &strings);
            pos = np;
            if !fname.is_empty() {
                if let Some(v) = val {
                    fields.insert(fname.to_lowercase(), v);
                }
            }
            // skip whitespace, then ',' or '}'
            while pos < chars.len() && chars[pos].is_whitespace() {
                pos += 1;
            }
            if pos < chars.len() && chars[pos] == ',' {
                pos += 1;
            } else if pos < chars.len() && chars[pos] == '}' {
                pos += 1;
                break;
            } else if pos >= chars.len() {
                break;
            }
        }
        entries.push(BibEntry {
            etype: etype_lc,
            key,
            fields,
        });
    }
    entries
}

fn parse_bib_field(
    chars: &[char],
    mut pos: usize,
    strings: &HashMap<String, String>,
) -> (Option<String>, Option<String>, usize) {
    while pos < chars.len() && chars[pos].is_whitespace() {
        pos += 1;
    }
    let mut name = String::new();
    while pos < chars.len()
        && (chars[pos].is_ascii_alphanumeric() || matches!(chars[pos], '-' | '_' | ':' | '+'))
    {
        name.push(chars[pos]);
        pos += 1;
    }
    while pos < chars.len() && chars[pos].is_whitespace() {
        pos += 1;
    }
    if pos < chars.len() && chars[pos] == '=' {
        pos += 1;
    }
    let (val, pos) = parse_bib_value(chars, pos, strings);
    (Some(name), val, pos)
}

/// Parse a value: braced {...} (nested), "quoted", number, or macro name;
/// supports # concatenation.
fn parse_bib_value(
    chars: &[char],
    mut pos: usize,
    strings: &HashMap<String, String>,
) -> (Option<String>, usize) {
    while pos < chars.len() && chars[pos].is_whitespace() {
        pos += 1;
    }
    let mut out = String::new();
    let mut have = false;
    loop {
        while pos < chars.len() && chars[pos].is_whitespace() {
            pos += 1;
        }
        if pos >= chars.len() {
            break;
        }
        match chars[pos] {
            '{' => {
                pos += 1;
                let mut depth = 1;
                while pos < chars.len() && depth > 0 {
                    if chars[pos] == '{' {
                        depth += 1;
                        out.push('{');
                    } else if chars[pos] == '}' {
                        depth -= 1;
                        if depth > 0 {
                            out.push('}');
                        }
                    } else {
                        out.push(chars[pos]);
                    }
                    pos += 1;
                }
                have = true;
            }
            '"' => {
                pos += 1;
                while pos < chars.len() && chars[pos] != '"' {
                    out.push(chars[pos]);
                    pos += 1;
                }
                pos += 1;
                have = true;
            }
            '0'..='9' => {
                while pos < chars.len() && chars[pos].is_ascii_digit() {
                    out.push(chars[pos]);
                    pos += 1;
                }
                have = true;
            }
            c if c.is_ascii_alphabetic() => {
                let mut name = String::new();
                while pos < chars.len()
                    && (chars[pos].is_ascii_alphanumeric()
                        || matches!(chars[pos], '-' | '_' | ':' | '+'))
                {
                    name.push(chars[pos]);
                    pos += 1;
                }
                if let Some(v) = strings
                    .get(&name.to_lowercase())
                    .or_else(|| strings.get(&name))
                {
                    out.push_str(v);
                } else if let Some(m) = month_name(&name) {
                    out.push_str(m);
                }
                have = true;
            }
            _ => break,
        }
        // # concatenation?
        let mut look = pos;
        while look < chars.len() && chars[look].is_whitespace() {
            look += 1;
        }
        if look < chars.len() && chars[look] == '#' {
            pos = look + 1;
        } else {
            pos = look;
            break;
        }
    }
    if have {
        (Some(out), pos)
    } else {
        (None, pos)
    }
}

fn month_name(m: &str) -> Option<&'static str> {
    Some(match m.to_lowercase().as_str() {
        "jan" => "January",
        "feb" => "February",
        "mar" => "March",
        "apr" => "April",
        "may" => "May",
        "jun" => "June",
        "jul" => "July",
        "aug" => "August",
        "sep" => "September",
        "oct" => "October",
        "nov" => "November",
        "dec" => "December",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Strip LaTeX markup from a field value for display.
pub fn clean_text(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // control word: copy letters then one braced group content
                let mut name = String::new();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_alphabetic() {
                        name.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                match name.as_str() {
                    "emph" | "textit" | "textbf" | "texttt" | "textsc" | "url" | "href" => {
                        // href: skip first group (url), keep second
                        if name == "href" {
                            skip_braced(&mut chars);
                        }
                        if let Some(&d) = chars.peek() {
                            if d == '{' {
                                out.push_str(&braced_content(&mut chars));
                            }
                        }
                    }
                    "LaTeX" => out.push_str("LaTeX"),
                    "TeX" => out.push_str("TeX"),
                    "" => {
                        // control symbol
                        if let Some(&d) = chars.peek() {
                            if d == '{'
                                || d == '}'
                                || d == '$'
                                || d == '%'
                                || d == '&'
                                || d == '#'
                                || d == '_'
                            {
                                out.push(d);
                                chars.next();
                            }
                        }
                    }
                    _ => {}
                }
            }
            '{' | '}' => {}
            '~' => out.push(' '),
            '$' => {}
            '`' => {
                if chars.peek() == Some(&'`') {
                    chars.next();
                    out.push('“');
                } else {
                    out.push('‘');
                }
            }
            '\'' => {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    out.push('”');
                } else {
                    out.push('’');
                }
            }
            '-' => {
                if chars.peek() == Some(&'-') {
                    chars.next();
                    if chars.peek() == Some(&'-') {
                        chars.next();
                        out.push('—');
                    } else {
                        out.push('–');
                    }
                } else {
                    out.push('-');
                }
            }
            c => out.push(c),
        }
    }
    // collapse whitespace
    let mut collapsed = String::new();
    let mut last_ws = false;
    for c in out.chars() {
        if c.is_whitespace() {
            if !last_ws {
                collapsed.push(' ');
            }
            last_ws = true;
        } else {
            collapsed.push(c);
            last_ws = false;
        }
    }
    collapsed.trim().to_string()
}

fn skip_braced(chars: &mut std::iter::Peekable<std::str::Chars>) {
    if chars.peek() == Some(&'{') {
        let _ = braced_content(chars);
    }
}

fn braced_content(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut out = String::new();
    if chars.peek() != Some(&'{') {
        return out;
    }
    chars.next();
    let mut depth = 1;
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                depth += 1;
                out.push('{');
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                out.push('}');
            }
            c => out.push(c),
        }
    }
    clean_text(&out)
}

fn year_of(e: &BibEntry) -> String {
    e.fields.get("year").cloned().unwrap_or_default()
}

fn author_sort(e: &BibEntry) -> String {
    let a = e
        .fields
        .get("author")
        .or_else(|| e.fields.get("editor"))
        .cloned()
        .unwrap_or_default();
    let first = a.split(" and ").next().unwrap_or("");
    if let Some(idx) = first.find(',') {
        first[..idx].trim().to_string()
    } else {
        first.trim().to_string()
    }
}

#[derive(Clone)]
struct Name {
    first: String,
    last: String,
}

fn parse_names(s: &str) -> Vec<Name> {
    s.split(" and ")
        .map(|part| {
            let part = part.trim();
            if let Some(idx) = part.find(',') {
                Name {
                    last: part[..idx].trim().to_string(),
                    first: part[idx + 1..].trim().to_string(),
                }
            } else {
                // "First M. Last"
                let words: Vec<&str> = part.split_whitespace().collect();
                if words.len() <= 1 {
                    Name {
                        first: String::new(),
                        last: part.to_string(),
                    }
                } else {
                    Name {
                        first: words[..words.len() - 1].join(" "),
                        last: words[words.len() - 1].to_string(),
                    }
                }
            }
        })
        .collect()
}

fn initials(first: &str) -> String {
    first
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .map(|c| format!("{}.", c))
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_names(names: &[Name], abbrv: bool) -> String {
    let mut parts: Vec<String> = names
        .iter()
        .map(|n| {
            if abbrv {
                let init = initials(&n.first);
                if init.is_empty() {
                    n.last.clone()
                } else {
                    format!("{}, {}", n.last, init)
                }
            } else if n.first.is_empty() {
                n.last.clone()
            } else {
                format!("{} {}", n.first, n.last)
            }
        })
        .collect();
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        2 => format!("{} and {}", parts[0], parts[1]),
        _ => {
            let last = parts.pop().unwrap();
            format!("{}, and {}", parts.join(", "), last)
        }
    }
}

pub fn short_authors_pub(e: &BibEntry) -> String {
    short_authors(e)
}

pub fn year_of_pub(e: &BibEntry) -> String {
    year_of(e)
}

fn short_authors(e: &BibEntry) -> String {
    let names = parse_names(
        &e.fields
            .get("author")
            .or_else(|| e.fields.get("editor"))
            .cloned()
            .unwrap_or_default(),
    );
    match names.len() {
        0 => String::new(),
        1 => names[0].last.clone(),
        2 => format!("{} and {}", names[0].last, names[1].last),
        _ => format!("{} et al.", names[0].last),
    }
}

fn alpha_label(e: &BibEntry) -> String {
    let authors = e
        .fields
        .get("author")
        .map(|s| clean_text(s))
        .unwrap_or_default();
    let names = parse_names(&authors);
    let mut lab: String = if names.is_empty() {
        author_sort(e)
            .chars()
            .filter(|c| c.is_ascii_alphabetic())
            .take(3)
            .collect()
    } else {
        names
            .iter()
            .filter_map(|n| n.last.chars().find(|c| c.is_ascii_alphabetic()))
            .collect()
    };
    let y = year_of(e);
    let yy: String = y.chars().skip(y.len().saturating_sub(2)).collect();
    lab.push_str(&yy);
    if lab.is_empty() {
        "[?]".into()
    } else {
        format!("[{}]", lab)
    }
}

fn author_year_label(e: &BibEntry) -> String {
    format!("{} ({})", short_authors(e), year_of(e))
}

fn format_entry(e: &BibEntry, style: BibStyle) -> Vec<Inline> {
    let mut text = String::new();
    let abbrv = style == BibStyle::Abbrv;
    let author_year = matches!(style, BibStyle::Apalike | BibStyle::Authoryear);
    let f = |k: &str| clean_text(e.fields.get(k).map(|s| s.as_str()).unwrap_or(""));

    let authors = f("author");
    let title = f("title");
    if author_year {
        if !authors.is_empty() {
            text.push_str(&short_authors(e));
            text.push_str(" (");
            text.push_str(&year_of(e));
            text.push_str("). ");
        }
    } else {
        if !authors.is_empty() {
            let names = parse_names(&authors);
            text.push_str(&format_names(&names, abbrv));
            text.push_str(". ");
        }
    }
    match e.etype.as_str() {
        "article" => {
            if !title.is_empty() {
                text.push_str(&title);
                text.push('.');
            }
            let journal = f("journal");
            if !journal.is_empty() {
                text.push(' ');
                text.push_str(&journal);
            }
            let vol = f("volume");
            let num = f("number");
            if !vol.is_empty() {
                text.push_str(&format!(", vol. {}", vol));
                if !num.is_empty() {
                    text.push_str(&format!("({})", num));
                }
            } else if !num.is_empty() {
                text.push_str(&format!(", no. {}", num));
            }
            let pages = f("pages");
            if !pages.is_empty() {
                let p = pages.replace("--", "–");
                text.push_str(&format!(", pp. {}", p));
            }
            if !author_year {
                let y = year_of(e);
                if !y.is_empty() {
                    text.push_str(&format!(", {}", y));
                }
            }
            text.push('.');
        }
        "book" | "proceedings" | "manual" | "report" => {
            if !title.is_empty() {
                text.push_str(&title);
                text.push('.');
            }
            let publisher = f("publisher");
            let school = f("school");
            let inst = f("institution");
            let org = f("organization");
            let addr = f("address");
            if !publisher.is_empty() {
                text.push(' ');
                text.push_str(&publisher);
            } else if !school.is_empty() {
                text.push(' ');
                text.push_str(&school);
            } else if !inst.is_empty() {
                text.push(' ');
                text.push_str(&inst);
            } else if !org.is_empty() {
                text.push(' ');
                text.push_str(&org);
            }
            if !addr.is_empty() {
                if !publisher.is_empty()
                    || !school.is_empty()
                    || !inst.is_empty()
                    || !org.is_empty()
                {
                    text.push_str(", ");
                }
                text.push_str(&addr);
            }
            if !author_year {
                let y = year_of(e);
                if !y.is_empty() {
                    text.push_str(&format!(", {}", y));
                }
            }
            text.push('.');
        }
        "inproceedings" | "incollection" | "conference" => {
            if !title.is_empty() {
                text.push_str(&title);
                text.push('.');
            }
            let book = f("booktitle");
            if !book.is_empty() {
                text.push_str(" In ");
                text.push_str(&book);
            }
            let publisher = f("publisher");
            let addr = f("address");
            if !publisher.is_empty() {
                text.push_str(&format!(", {}", publisher));
            }
            if !addr.is_empty() {
                text.push_str(&format!(", {}", addr));
            }
            let pages = f("pages");
            if !pages.is_empty() {
                text.push_str(&format!(", pp. {}", pages.replace("--", "–")));
            }
            if !author_year {
                let y = year_of(e);
                if !y.is_empty() {
                    text.push_str(&format!(", {}", y));
                }
            }
            text.push('.');
        }
        "phdthesis" | "mastersthesis" => {
            if !title.is_empty() {
                text.push_str(&title);
                text.push('.');
            }
            let school = f("school");
            if !school.is_empty() {
                text.push(' ');
                text.push_str(&school);
            }
            if !author_year {
                let y = year_of(e);
                if !y.is_empty() {
                    text.push_str(&format!(", {}", y));
                }
            }
            text.push('.');
        }
        "misc" | "online" | "webpage" => {
            if !title.is_empty() {
                text.push_str(&title);
                text.push('.');
            }
            let how = f("howpublished");
            if !how.is_empty() {
                text.push(' ');
                text.push_str(&how);
            }
            let note = f("note");
            if !note.is_empty() {
                text.push(' ');
                text.push_str(&note);
            }
            let url = f("url");
            if !url.is_empty() {
                text.push(' ');
                text.push_str(&url);
            }
            if !author_year {
                let y = year_of(e);
                if !y.is_empty() {
                    text.push_str(&format!(", {}", y));
                }
            }
            text.push('.');
        }
        _ => {
            if !title.is_empty() {
                text.push_str(&title);
                text.push('.');
            }
            let note = f("note");
            if !note.is_empty() {
                text.push(' ');
                text.push_str(&note);
            }
            if !author_year {
                let y = year_of(e);
                if !y.is_empty() {
                    text.push_str(&format!(", {}", y));
                }
            }
            text.push('.');
        }
    }
    vec![Inline::Text(text)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_bib() {
        let src = r#"
@article{knuth1984,
  author  = "Knuth, Donald E. and others",
  title   = {Literate Programming},
  journal = {The Computer Journal},
  volume  = {27},
  number  = {2},
  pages   = {97--111},
  year    = 1984
}
@string{journal-physics = "Journal of Physics"}
@misc{x1, howpublished = journal-physics # " Online"}
"#;
        let entries = parse_bib(src);
        assert_eq!(entries.len(), 2);
        let e = &entries[0];
        assert_eq!(e.fields["title"], "Literate Programming");
        assert_eq!(e.fields["year"], "1984");
        let e2 = &entries[1];
        assert_eq!(e2.fields["howpublished"], "Journal of Physics Online");
    }

    #[test]
    fn style_labels_and_cite() {
        let mut bs = BibState::new();
        bs.entries = parse_bib(
            r#"@article{a, author={Alice A}, title={T1}, journal={J}, year={2020}}
               @article{b, author={Bob B}, title={T2}, journal={J}, year={2021}}
               @article{c, author={Carol C}, title={T3}, journal={J}, year={2022}}"#,
        )
        .into_iter()
        .map(|e| (e.key.clone(), e))
        .collect();
        bs.files = vec![];
        bs.cited = vec!["a".into(), "b".into(), "c".into()];
        bs.resolve(std::path::Path::new("."));
        let (txt, _) = bs.render_cite(&["a".into(), "b".into(), "c".into()], false);
        assert_eq!(txt, "[1-3]");
        let (txt, _) = bs.render_cite(&["c".into(), "a".into()], false);
        assert_eq!(txt, "[1,3]");
    }

    #[test]
    fn alpha_style() {
        let mut bs = BibState::new();
        bs.style = BibStyle::Alpha;
        bs.entries =
            parse_bib(r#"@article{a, author={Alice A}, title={T1}, journal={J}, year={2020}}"#)
                .into_iter()
                .map(|e| (e.key.clone(), e))
                .collect();
        bs.cited = vec!["a".into()];
        bs.resolve(std::path::Path::new("."));
        assert_eq!(bs.label_of("a").unwrap(), "[A20]");
    }
}
