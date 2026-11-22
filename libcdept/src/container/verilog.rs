//! Verilog and SystemVerilog: the modules of a file with their ports, and the
//! user-defined primitives. Bodies are not read beyond the port declarations.

use serde::{Deserialize, Serialize};

/// What a Verilog file holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VerilogIndex {
    /// Modules in file order, each name once (a file may define a module several
    /// times under `` `ifdef `` branches; the first one counts).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<VerilogModule>,
    /// Names of the `primitive` definitions (UDPs).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub primitives: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VerilogModule {
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<VerilogPort>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VerilogPort {
    pub name: String,
    /// `input`, `output`, `inout`; empty when the file does not say.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub direction: String,
    /// The range as written, e.g. `[7:0]`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub range: String,
}

/// Words and punctuation of Verilog text, without comments, strings, attributes
/// `(* … *)` and compiler directives.
struct Lexer<'a> {
    bytes: &'a [u8],
    at: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token<'a> {
    Word(&'a [u8]),
    Punct(u8),
}

impl<'a> Lexer<'a> {
    fn next(&mut self) -> Option<Token<'a>> {
        let b = self.bytes;
        loop {
            let c = *b.get(self.at)?;
            match c {
                _ if c.is_ascii_whitespace() => self.at += 1,
                b'/' if b.get(self.at + 1) == Some(&b'/') => {
                    self.at = memchr::memchr(b'\n', &b[self.at..]).map_or(b.len(), |n| self.at + n);
                }
                b'/' if b.get(self.at + 1) == Some(&b'*') => {
                    self.at = memchr::memmem::find(&b[self.at + 2..], b"*/")
                        .map_or(b.len(), |n| self.at + 2 + n + 2);
                }
                b'(' if b.get(self.at + 1) == Some(&b'*') && b.get(self.at + 2) != Some(&b')') => {
                    self.at = memchr::memmem::find(&b[self.at + 2..], b"*)")
                        .map_or(b.len(), |n| self.at + 2 + n + 2);
                }
                b'"' => {
                    let mut j = self.at + 1;
                    while let Some(n) = memchr::memchr2(b'"', b'\\', &b[j..]) {
                        j += n + 1;
                        if b[j - 1] == b'"' {
                            break;
                        }
                        j += 1;
                    }
                    self.at = j.min(b.len());
                }
                // A compiler directive. Those that take the rest of the line go with
                // it; `ifdef and the like go with their macro name, so that what they
                // guard is read (both branches: `module m (a `ifdef P , vdd `endif);`).
                b'`' => {
                    let start = self.at + 1;
                    let mut end = start;
                    while end < b.len() && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
                        end += 1;
                    }
                    self.at = end;
                    match &b[start..end] {
                        b"define"
                        | b"include"
                        | b"timescale"
                        | b"default_nettype"
                        | b"undef"
                        | b"line"
                        | b"pragma"
                        | b"begin_keywords"
                        | b"unconnected_drive"
                        | b"default_decay_time"
                        | b"default_trireg_strength"
                        | b"delay_mode_path" => {
                            // To the end of the line, following backslash-newlines.
                            loop {
                                let line_end = memchr::memchr(b'\n', &b[self.at..])
                                    .map_or(b.len(), |n| self.at + n);
                                let continued = line_end > 0
                                    && line_end < b.len()
                                    && b[..line_end].trim_ascii_end().ends_with(b"\\");
                                self.at = (line_end + 1).min(b.len());
                                if !continued {
                                    break;
                                }
                            }
                        }
                        b"ifdef" | b"ifndef" | b"elsif" => {
                            while self.at < b.len() && matches!(b[self.at], b' ' | b'\t') {
                                self.at += 1;
                            }
                            while self.at < b.len()
                                && (b[self.at].is_ascii_alphanumeric() || b[self.at] == b'_')
                            {
                                self.at += 1;
                            }
                        }
                        _ => {}
                    }
                }
                b'\\' => {
                    // An escaped identifier runs to the next white space.
                    let start = self.at;
                    while self.at < b.len() && !b[self.at].is_ascii_whitespace() {
                        self.at += 1;
                    }
                    return Some(Token::Word(&b[start..self.at]));
                }
                _ if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' => {
                    let start = self.at;
                    while self.at < b.len()
                        && (b[self.at].is_ascii_alphanumeric() || matches!(b[self.at], b'_' | b'$'))
                    {
                        self.at += 1;
                    }
                    return Some(Token::Word(&b[start..self.at]));
                }
                _ => {
                    self.at += 1;
                    return Some(Token::Punct(c));
                }
            }
        }
    }

    /// The text of a `[ … ]` range whose opening bracket was just read.
    fn range(&mut self) -> String {
        let start = self.at - 1;
        let end = memchr::memchr(b']', &self.bytes[self.at..])
            .map_or(self.bytes.len(), |n| self.at + n + 1);
        self.at = end;
        String::from_utf8_lossy(&self.bytes[start..end])
            .split_whitespace()
            .collect()
    }
}

const DIRECTIONS: [&[u8]; 3] = [b"input", b"output", b"inout"];
/// Words between a direction and the port name.
const TYPES: [&[u8]; 14] = [
    b"wire",
    b"reg",
    b"logic",
    b"tri",
    b"signed",
    b"unsigned",
    b"var",
    b"supply0",
    b"supply1",
    b"wand",
    b"wor",
    b"bit",
    b"integer",
    b"real",
];

fn text(word: &[u8]) -> String {
    String::from_utf8_lossy(word).into_owned()
}

/// Read a Verilog file's text. Never fails: what is not understood is skipped.
pub fn parse(bytes: &[u8]) -> VerilogIndex {
    let mut index = VerilogIndex::default();
    let mut lexer = Lexer { bytes, at: 0 };
    while let Some(token) = lexer.next() {
        match token {
            Token::Word(b"module" | b"macromodule") => {
                let module = module(&mut lexer);
                if !module.name.is_empty() && !index.modules.iter().any(|m| m.name == module.name) {
                    index.modules.push(module);
                }
            }
            Token::Word(b"primitive") => {
                if let Some(Token::Word(name)) = lexer.next() {
                    let name = text(name);
                    if !index.primitives.contains(&name) {
                        index.primitives.push(name);
                    }
                }
                skip_to(&mut lexer, b"endprimitive");
            }
            _ => {}
        }
    }
    index
}

fn skip_to(lexer: &mut Lexer, word: &[u8]) {
    while let Some(token) = lexer.next() {
        if token == Token::Word(word) {
            break;
        }
    }
}

/// After `module`: the name, the port list, then the declarations of the body.
fn module(lexer: &mut Lexer) -> VerilogModule {
    let mut module = VerilogModule::default();
    // `module automatic name`, `module static name`.
    loop {
        match lexer.next() {
            Some(Token::Word(b"automatic" | b"static")) => {}
            Some(Token::Word(name)) => {
                module.name = text(name);
                break;
            }
            _ => return module,
        }
    }
    // Up to the header's semicolon: `#( parameters )` and `( ports )`.
    let mut depth = 0usize;
    let mut in_parameters = false;
    let mut direction = String::new();
    let mut range = String::new();
    // In a port list `a = 1` gives a default: skip to the comma.
    let mut skipping = false;
    loop {
        match lexer.next() {
            None => return module,
            Some(Token::Punct(b'#')) if depth == 0 => in_parameters = true,
            Some(Token::Punct(b'(')) => depth += 1,
            Some(Token::Punct(b')')) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    in_parameters = false;
                }
            }
            Some(Token::Punct(b';')) if depth == 0 => break,
            Some(_) if in_parameters || depth != 1 => {}
            Some(Token::Punct(b',')) => skipping = false,
            Some(Token::Punct(b'=')) => skipping = true,
            Some(_) if skipping => {}
            Some(Token::Punct(b'[')) => {
                let text = lexer.range();
                // A range after the name is an unpacked dimension, not the width.
                if module
                    .ports
                    .last()
                    .is_none_or(|p: &VerilogPort| !p.name.is_empty())
                {
                    range = text;
                }
            }
            Some(Token::Punct(b'.')) => {
                // `.name(expression)`: the name is the port.
                if let Some(Token::Word(name)) = lexer.next() {
                    module.ports.push(VerilogPort {
                        name: text(name),
                        direction: direction.clone(),
                        range: String::new(),
                    });
                }
                skipping = true;
            }
            Some(Token::Word(word)) if DIRECTIONS.contains(&word) => {
                direction = text(word);
                range.clear();
            }
            Some(Token::Word(word)) if TYPES.contains(&word) => {}
            Some(Token::Word(word)) => module.ports.push(VerilogPort {
                name: text(word),
                direction: direction.clone(),
                range: range.clone(),
            }),
            Some(Token::Punct(_)) => {}
        }
    }
    // The body: `input [3:0] a, b;` declarations give the directions of a plain list.
    let mut declaring: Option<String> = None;
    let mut range = String::new();
    loop {
        match lexer.next() {
            None | Some(Token::Word(b"endmodule")) => break,
            Some(Token::Word(word)) if DIRECTIONS.contains(&word) => {
                declaring = Some(text(word));
                range.clear();
            }
            Some(Token::Punct(b';')) => declaring = None,
            Some(token) => {
                let Some(direction) = &declaring else {
                    continue;
                };
                match token {
                    Token::Punct(b'[') => range = lexer.range(),
                    Token::Punct(b'=') => declaring = None,
                    Token::Word(word) if !TYPES.contains(&word) => {
                        let name = text(word);
                        if let Some(port) = module.ports.iter_mut().find(|p| p.name == name)
                            && port.direction.is_empty()
                        {
                            port.direction = direction.clone();
                            port.range = range.clone();
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    module
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports(module: &VerilogModule) -> Vec<(&str, &str, &str)> {
        module
            .ports
            .iter()
            .map(|p| (p.name.as_str(), p.direction.as_str(), p.range.as_str()))
            .collect()
    }

    #[test]
    fn plain_and_ansi_port_lists() {
        let text = r#"
`timescale 1ns / 1ps
`default_nettype none
// module commented_out (a);
/* module also_out (a); endmodule */
(* blackbox *)
module inv_1 (
    Y   ,
    A   ,
    VPWR
);
    output Y   ;
    input  A   ;
    input  VPWR;
    wire not0_out_Y;
    not not0 (not0_out_Y, A);
endmodule
`ifdef FUNCTIONAL
module inv_1 (Y, A); output Y; input A; endmodule
`endif
module ram #(parameter W = 8, parameter D = (1 << 4)) (
    input  wire        clk,
    input  wire [W-1:0] din, addr,
    output reg  [ W-1 : 0 ] dout,
    inout vdd
);
    initial $display("module fake (x);");
endmodule
primitive udp_dff (Q, D, CLK);
    output Q; input D, CLK; reg Q;
    table 0 r : ? : 0; endtable
endprimitive
module empty; endmodule
module top(); endmodule
`define WIDE(x) \\
    module not_this (x); endmodule
module gated (Y, A
        `ifdef USE_POWER_PINS , VDD, VSS `endif);
    output Y; input A;
    `ifdef USE_POWER_PINS inout VDD, VSS; `endif
    wire ok = (VDD === 1'b1) && `SOME_MACRO;
endmodule
"#;
        let index = parse(text.as_bytes());
        let names: Vec<_> = index.modules.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["inv_1", "ram", "empty", "top", "gated"]);
        assert_eq!(
            ports(&index.modules[4]),
            [
                ("Y", "output", ""),
                ("A", "input", ""),
                ("VDD", "inout", ""),
                ("VSS", "inout", "")
            ]
        );
        assert_eq!(
            ports(&index.modules[0]),
            [
                ("Y", "output", ""),
                ("A", "input", ""),
                ("VPWR", "input", "")
            ]
        );
        assert_eq!(
            ports(&index.modules[1]),
            [
                ("clk", "input", ""),
                ("din", "input", "[W-1:0]"),
                ("addr", "input", "[W-1:0]"),
                ("dout", "output", "[W-1:0]"),
                ("vdd", "inout", ""),
            ]
        );
        assert!(index.modules[2].ports.is_empty());
        assert_eq!(index.primitives, ["udp_dff"]);
    }

    #[test]
    fn ranges_in_body_declarations() {
        let index = parse(b"module m (a, b);\n input [3:0] a;\n output b;\nendmodule\n");
        assert_eq!(
            ports(&index.modules[0]),
            [("a", "input", "[3:0]"), ("b", "output", "")]
        );
    }
}
