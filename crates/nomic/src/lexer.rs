//! Tokenizer for the Nomic authoring syntax.
//!
//! The surface syntax borrows C/TypeScript-family expression tokens and adds a
//! handful of declaration keywords. `///` doc comments are preserved as tokens
//! so descriptions become part of the IR; `//` comments are dropped.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(i64),
    Str(String),
    Doc(String),
    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Semi,
    Dot,
    DotDot,
    Question,
    QQ,       // ??
    Assign,   // =
    FatArrow, // =>
    Pipe,     // |
    // operators
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    AndAnd,
    OrOr,
    Eof,
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tok::Ident(s) => write!(f, "identifier `{s}`"),
            Tok::Int(n) => write!(f, "integer `{n}`"),
            Tok::Str(s) => write!(f, "string {s:?}"),
            Tok::Doc(_) => write!(f, "doc comment"),
            Tok::LParen => write!(f, "`(`"),
            Tok::RParen => write!(f, "`)`"),
            Tok::LBrace => write!(f, "`{{`"),
            Tok::RBrace => write!(f, "`}}`"),
            Tok::Comma => write!(f, "`,`"),
            Tok::Colon => write!(f, "`:`"),
            Tok::Semi => write!(f, "`;`"),
            Tok::Dot => write!(f, "`.`"),
            Tok::DotDot => write!(f, "`..`"),
            Tok::Question => write!(f, "`?`"),
            Tok::QQ => write!(f, "`??`"),
            Tok::Assign => write!(f, "`=`"),
            Tok::FatArrow => write!(f, "`=>`"),
            Tok::Pipe => write!(f, "`|`"),
            Tok::Eq => write!(f, "`==`"),
            Tok::Ne => write!(f, "`!=`"),
            Tok::Lt => write!(f, "`<`"),
            Tok::Le => write!(f, "`<=`"),
            Tok::Gt => write!(f, "`>`"),
            Tok::Ge => write!(f, "`>=`"),
            Tok::Plus => write!(f, "`+`"),
            Tok::Minus => write!(f, "`-`"),
            Tok::Star => write!(f, "`*`"),
            Tok::Slash => write!(f, "`/`"),
            Tok::Percent => write!(f, "`%`"),
            Tok::Bang => write!(f, "`!`"),
            Tok::AndAnd => write!(f, "`&&`"),
            Tok::OrOr => write!(f, "`||`"),
            Tok::Eof => write!(f, "end of input"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
}

impl fmt::Display for Pos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub pos: Pos,
}

#[derive(Debug, Clone)]
pub struct LexError {
    pub pos: Pos,
    pub message: String,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.pos, self.message)
    }
}

/// A plain `//` comment, kept out of the token stream but available to
/// tools that must preserve it (the formatter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub pos: Pos,
    pub text: String,
}

pub fn lex(src: &str) -> Result<Vec<Token>, LexError> {
    lex_full(src).map(|(t, _)| t)
}

/// Tokenize, also returning every plain `//` comment with its position.
pub fn lex_full(src: &str) -> Result<(Vec<Token>, Vec<Comment>), LexError> {
    let mut comments = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut col = 1u32;
    let mut out = Vec::new();

    macro_rules! push {
        ($tok:expr, $pos:expr) => {
            out.push(Token { tok: $tok, pos: $pos })
        };
    }

    while i < chars.len() {
        let c = chars[i];
        let pos = Pos { line, col };
        // whitespace
        if c == '\n' {
            line += 1;
            col = 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            col += 1;
            i += 1;
            continue;
        }
        // comments
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            let is_doc = i + 2 < chars.len() && chars[i + 2] == '/';
            let mut j = i + if is_doc { 3 } else { 2 };
            let start = j;
            while j < chars.len() && chars[j] != '\n' {
                j += 1;
            }
            let text: String = chars[start..j].iter().collect();
            if is_doc {
                push!(Tok::Doc(text.trim().to_string()), pos);
            } else {
                comments.push(Comment { pos, text: text.trim().to_string() });
            }
            col += (j - i) as u32;
            i = j;
            continue;
        }
        // strings
        if c == '"' {
            let mut j = i + 1;
            let mut s = String::new();
            let mut closed = false;
            while j < chars.len() {
                let d = chars[j];
                if d == '\\' && j + 1 < chars.len() {
                    let e = chars[j + 1];
                    s.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        '"' => '"',
                        '\\' => '\\',
                        other => other,
                    });
                    j += 2;
                    continue;
                }
                if d == '"' {
                    closed = true;
                    j += 1;
                    break;
                }
                if d == '\n' {
                    return Err(LexError { pos, message: "unterminated string literal".into() });
                }
                s.push(d);
                j += 1;
            }
            if !closed {
                return Err(LexError { pos, message: "unterminated string literal".into() });
            }
            col += (j - i) as u32;
            i = j;
            push!(Tok::Str(s), pos);
            continue;
        }
        // numbers
        if c.is_ascii_digit() {
            let mut j = i;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            let text: String = chars[i..j].iter().collect();
            let n: i64 = text
                .parse()
                .map_err(|_| LexError { pos, message: format!("integer literal out of range: {text}") })?;
            col += (j - i) as u32;
            i = j;
            push!(Tok::Int(n), pos);
            continue;
        }
        // identifiers
        if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let text: String = chars[i..j].iter().collect();
            col += (j - i) as u32;
            i = j;
            push!(Tok::Ident(text), pos);
            continue;
        }
        // two-char operators
        let two: Option<Tok> = if i + 1 < chars.len() {
            match (c, chars[i + 1]) {
                ('=', '=') => Some(Tok::Eq),
                ('!', '=') => Some(Tok::Ne),
                ('<', '=') => Some(Tok::Le),
                ('>', '=') => Some(Tok::Ge),
                ('&', '&') => Some(Tok::AndAnd),
                ('|', '|') => Some(Tok::OrOr),
                ('=', '>') => Some(Tok::FatArrow),
                ('.', '.') => Some(Tok::DotDot),
                ('?', '?') => Some(Tok::QQ),
                _ => None,
            }
        } else {
            None
        };
        if let Some(t) = two {
            push!(t, pos);
            i += 2;
            col += 2;
            continue;
        }
        let one = match c {
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            '{' => Tok::LBrace,
            '}' => Tok::RBrace,
            ',' => Tok::Comma,
            ':' => Tok::Colon,
            ';' => Tok::Semi,
            '.' => Tok::Dot,
            '?' => Tok::Question,
            '=' => Tok::Assign,
            '|' => Tok::Pipe,
            '<' => Tok::Lt,
            '>' => Tok::Gt,
            '+' => Tok::Plus,
            '-' => Tok::Minus,
            '*' => Tok::Star,
            '/' => Tok::Slash,
            '%' => Tok::Percent,
            '!' => Tok::Bang,
            other => {
                return Err(LexError { pos, message: format!("unexpected character `{other}`") });
            }
        };
        push!(one, pos);
        i += 1;
        col += 1;
    }
    out.push(Token { tok: Tok::Eof, pos: Pos { line, col } });
    Ok((out, comments))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_basic_tokens() {
        let toks = lex("rule \"x\" on Drop(p, c) { require Turn == p \"no\" }").unwrap();
        let kinds: Vec<&Tok> = toks.iter().map(|t| &t.tok).collect();
        assert!(matches!(kinds[0], Tok::Ident(s) if s == "rule"));
        assert!(kinds.contains(&&Tok::Eq));
        assert!(matches!(kinds.last().unwrap(), Tok::Eof));
    }

    #[test]
    fn doc_comments_are_tokens_and_plain_comments_are_not() {
        let toks = lex("/// hello\n// nope\nfact X").unwrap();
        assert!(matches!(&toks[0].tok, Tok::Doc(s) if s == "hello"));
        assert!(matches!(&toks[1].tok, Tok::Ident(s) if s == "fact"));
    }

    #[test]
    fn ranges_lex_as_dotdot() {
        let toks = lex("0..6").unwrap();
        assert_eq!(toks[1].tok, Tok::DotDot);
    }
}
