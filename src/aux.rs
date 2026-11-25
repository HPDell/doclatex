//! .aux file: cross-pass data for labels, citations, bibliography.

use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct AuxLabel {
    pub number: String,
    pub page: String,
    pub counter: String,
    pub title: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct AuxData {
    pub labels: HashMap<String, AuxLabel>,
    pub citations: Vec<String>,
    pub bibfiles: Vec<String>,
    pub bibstyle: String,
}

impl AuxData {
    pub fn read(path: &Path) -> AuxData {
        let mut d = AuxData::default();
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => return d,
        };
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("\\newlabel") {
                if let Some((key, val)) = parse_braced_pair(rest.trim_start()) {
                    let fields = parse_aux_label_value(&val);
                    d.labels.insert(
                        key,
                        AuxLabel {
                            number: fields.0,
                            page: fields.1,
                            counter: fields.2,
                            title: fields.3,
                        },
                    );
                }
            } else if let Some(rest) = line.strip_prefix("\\citation") {
                if let Some((key, _)) = parse_braced_pair(rest.trim_start()) {
                    if !d.citations.contains(&key) {
                        d.citations.push(key);
                    }
                }
            } else if let Some(rest) = line.strip_prefix("\\bibdata") {
                if let Some((key, _)) = parse_braced_pair(rest.trim_start()) {
                    d.bibfiles.push(key);
                }
            } else if let Some(rest) = line.strip_prefix("\\bibstyle") {
                if let Some((key, _)) = parse_braced_pair(rest.trim_start()) {
                    d.bibstyle = key;
                }
            }
        }
        d
    }

    pub fn write(
        path: &Path,
        labels: &[(String, String, Option<String>, String)],
        citations: &[String],
        bibfiles: &[String],
        bibstyle: &str,
    ) {
        let mut s = String::new();
        s.push_str("% doclatex aux file\n");
        for (key, number, title, counter) in labels {
            let title = title.clone().unwrap_or_default();
            s.push_str(&format!(
                "\\newlabel{{{}}}{{{{{}}}{{1}}{{{}}}{{{}}}}}\n",
                key, number, counter, title
            ));
        }
        for c in citations {
            s.push_str(&format!("\\citation{{{}}}\n", c));
        }
        for b in bibfiles {
            s.push_str(&format!("\\bibdata{{{}}}\n", b));
        }
        s.push_str(&format!("\\bibstyle{{{}}}\n", bibstyle));
        let _ = std::fs::write(path, s);
    }
}

/// Parse {key}{rest...}: returns (key, rest-after-first-group).
fn parse_braced_pair(s: &str) -> Option<(String, String)> {
    let mut chars = s.chars();
    if chars.next() != Some('{') {
        return None;
    }
    let mut depth = 1;
    let mut key = String::new();
    for c in chars.by_ref() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((key, chars.collect()));
                }
            }
            _ => key.push(c),
        }
    }
    None
}

/// Parse the value {{number}{page}{counter}{title}}.
fn parse_aux_label_value(s: &str) -> (String, String, String, Option<String>) {
    let mut parts = vec![];
    let trimmed = s.trim();
    let mut rest = trimmed
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .unwrap_or(trimmed)
        .to_string();
    for _ in 0..4 {
        match parse_braced_pair(&rest) {
            Some((p, r)) => {
                parts.push(p);
                rest = r.trim().to_string();
            }
            None => break,
        }
    }
    let get = |i: usize| parts.get(i).cloned().unwrap_or_default();
    let title = get(3);
    (
        get(0),
        get(1),
        get(2),
        if title.is_empty() { None } else { Some(title) },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nested_newlabel_fields() {
        let value = parse_aux_label_value("{{2}{1}{equation}{Euler equation}}");
        assert_eq!(
            value,
            (
                "2".into(),
                "1".into(),
                "equation".into(),
                Some("Euler equation".into())
            )
        );
    }
}
