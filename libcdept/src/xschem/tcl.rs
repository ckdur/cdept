//! Just enough Tcl syntax to build commands for xschem and read its list results.

/// Quote `text` as one Tcl word, with no substitution taking place inside it.
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' | '"' | '$' | '[' | ']' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Split a Tcl list into its elements. Handles braces, double quotes and backslashes,
/// which is what xschem's commands return.
pub fn parse_list(text: &str) -> Result<Vec<String>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut items = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let mut item = String::new();
        match chars[i] {
            '{' => {
                let mut depth = 1;
                i += 1;
                loop {
                    let Some(&c) = chars.get(i) else {
                        return Err("unmatched open brace in list".into());
                    };
                    i += 1;
                    match c {
                        '\\' => {
                            // Inside braces a backslash is kept, but it hides a brace.
                            item.push(c);
                            if let Some(&next) = chars.get(i) {
                                item.push(next);
                                i += 1;
                            }
                        }
                        '{' => {
                            depth += 1;
                            item.push(c);
                        }
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            item.push(c);
                        }
                        _ => item.push(c),
                    }
                }
            }
            '"' => {
                i += 1;
                loop {
                    let Some(&c) = chars.get(i) else {
                        return Err("unmatched quote in list".into());
                    };
                    i += 1;
                    match c {
                        '"' => break,
                        '\\' => i = unescape(&chars, i, &mut item),
                        _ => item.push(c),
                    }
                }
            }
            _ => {
                while let Some(&c) = chars.get(i) {
                    if c.is_whitespace() {
                        break;
                    }
                    i += 1;
                    if c == '\\' {
                        i = unescape(&chars, i, &mut item);
                    } else {
                        item.push(c);
                    }
                }
            }
        }
        items.push(item);
    }
    Ok(items)
}

/// Handle the character after a backslash at `chars[i]`; returns the next index.
fn unescape(chars: &[char], i: usize, out: &mut String) -> usize {
    match chars.get(i) {
        Some('n') => out.push('\n'),
        Some('t') => out.push('\t'),
        Some('r') => out.push('\r'),
        Some(&c) => out.push(c),
        None => out.push('\\'),
    }
    i + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_blocks_substitution() {
        assert_eq!(quote("plain"), r#""plain""#);
        assert_eq!(quote("a b"), r#""a b""#);
        assert_eq!(quote("$x[exit]"), r#""\$x\[exit\]""#);
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(quote("{x}"), r#""\{x\}""#);
    }

    #[test]
    fn quoted_words_read_back() {
        for text in [
            "plain",
            "a b",
            "$x[exit]",
            "a\"b\\c",
            "{x}",
            "line1\nline2",
            "",
        ] {
            assert_eq!(parse_list(&quote(text)).unwrap(), vec![text.to_string()]);
        }
    }

    #[test]
    fn lists_split_on_braces_and_space() {
        let list = "{x1} {sky130_tests/bandgap.sym} {subcircuit}\n{R 1} {devices/res.sym} {}";
        assert_eq!(
            parse_list(list).unwrap(),
            [
                "x1",
                "sky130_tests/bandgap.sym",
                "subcircuit",
                "R 1",
                "devices/res.sym",
                ""
            ]
        );
        assert_eq!(parse_list("a {b {c d}} e").unwrap(), ["a", "b {c d}", "e"]);
        assert_eq!(parse_list("  ").unwrap(), Vec::<String>::new());
        assert!(parse_list("{open").is_err());
    }
}
