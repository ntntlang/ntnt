//! Lexer for the Intent programming language
//!
//! Transforms source code into a stream of tokens.

use std::iter::Peekable;
use std::str::Chars;

/// Token types for the Intent language
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Integer(i64),
    Float(f64),
    String(String),
    Bool(bool),

    // Identifiers and keywords
    Identifier(String),
    
    // Keywords
    Let,
    Mut,
    Fn,
    Return,
    If,
    Else,
    While,
    Loop,
    Break,
    Continue,
    Match,
    Struct,
    Enum,
    Impl,
    Type,
    Mod,
    Use,
    Pub,
    
    // Contract keywords
    Contract,
    Requires,
    Ensures,
    Invariant,
    
    // Effect keywords
    Effect,
    Try,
    Catch,
    
    // AI/Collaboration keywords
    Intent,
    Approve,
    Observe,
    Protocol,
    Async,
    Await,

    // Operators
    Plus,           // +
    Minus,          // -
    Star,           // *
    Slash,          // /
    Percent,        // %
    Caret,          // ^
    
    // Comparison
    Equal,          // ==
    NotEqual,       // !=
    Less,           // <
    LessEqual,      // <=
    Greater,        // >
    GreaterEqual,   // >=
    
    // Logical
    And,            // &&
    Or,             // ||
    Not,            // !
    
    // Assignment
    Assign,         // =
    PlusAssign,     // +=
    MinusAssign,    // -=
    StarAssign,     // *=
    SlashAssign,    // /=
    
    // Delimiters
    LeftParen,      // (
    RightParen,     // )
    LeftBrace,      // {
    RightBrace,     // }
    LeftBracket,    // [
    RightBracket,   // ]
    
    // Punctuation
    Comma,          // ,
    Dot,            // .
    Colon,          // :
    Semicolon,      // ;
    Arrow,          // ->
    FatArrow,       // =>
    Question,       // ?
    At,             // @
    Hash,           // #
    Ampersand,      // &
    Pipe,           // |
    
    // Special
    Eof,
    Newline,
}

/// A token with position information
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub column: usize,
    pub lexeme: String,
}

impl Token {
    pub fn new(kind: TokenKind, line: usize, column: usize, lexeme: String) -> Self {
        Token { kind, line, column, lexeme }
    }
}

/// Lexer for tokenizing Intent source code
pub struct Lexer<'a> {
    source: Peekable<Chars<'a>>,
    line: usize,
    column: usize,
    current_lexeme: String,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Lexer {
            source: source.chars().peekable(),
            line: 1,
            column: 1,
            current_lexeme: String::new(),
        }
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.source.next()?;
        self.current_lexeme.push(ch);
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn peek(&mut self) -> Option<&char> {
        self.source.peek()
    }

    fn peek_is(&mut self, expected: char) -> bool {
        self.peek() == Some(&expected)
    }

    fn match_char(&mut self, expected: char) -> bool {
        if self.peek_is(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(&ch) = self.peek() {
            match ch {
                ' ' | '\t' | '\r' => {
                    self.advance();
                }
                '\n' => {
                    self.advance();
                }
                '/' => {
                    // Check for comments
                    let mut chars = self.source.clone();
                    chars.next();
                    if chars.peek() == Some(&'/') {
                        // Line comment
                        self.advance(); // consume first /
                        self.advance(); // consume second /
                        while let Some(&ch) = self.peek() {
                            if ch == '\n' {
                                break;
                            }
                            self.advance();
                        }
                    } else if chars.peek() == Some(&'*') {
                        // Block comment
                        self.advance(); // consume /
                        self.advance(); // consume *
                        loop {
                            match self.advance() {
                                Some('*') if self.peek_is('/') => {
                                    self.advance();
                                    break;
                                }
                                None => break,
                                _ => {}
                            }
                        }
                    } else {
                        return;
                    }
                }
                _ => return,
            }
            self.current_lexeme.clear();
        }
    }

    fn scan_string(&mut self, quote: char) -> Token {
        let start_line = self.line;
        let start_column = self.column;
        self.current_lexeme.clear();
        
        let mut value = String::new();
        
        while let Some(&ch) = self.peek() {
            if ch == quote {
                self.advance();
                break;
            }
            if ch == '\\' {
                self.advance();
                match self.advance() {
                    Some('n') => value.push('\n'),
                    Some('t') => value.push('\t'),
                    Some('r') => value.push('\r'),
                    Some('\\') => value.push('\\'),
                    Some('"') => value.push('"'),
                    Some('\'') => value.push('\''),
                    Some(c) => value.push(c),
                    None => break,
                }
            } else {
                value.push(ch);
                self.advance();
            }
        }
        
        Token::new(
            TokenKind::String(value.clone()),
            start_line,
            start_column,
            format!("{}{}{}", quote, value, quote),
        )
    }

    fn scan_number(&mut self, first: char) -> Token {
        let start_line = self.line;
        let start_column = self.column - 1;
        
        let mut num_str = String::from(first);
        let mut is_float = false;
        
        // Check for hex, binary, octal
        if first == '0' {
            if let Some(&ch) = self.peek() {
                match ch {
                    'x' | 'X' => {
                        num_str.push(self.advance().unwrap());
                        while let Some(&ch) = self.peek() {
                            if ch.is_ascii_hexdigit() || ch == '_' {
                                if ch != '_' {
                                    num_str.push(ch);
                                }
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        let value = i64::from_str_radix(&num_str[2..], 16).unwrap_or(0);
                        return Token::new(
                            TokenKind::Integer(value),
                            start_line,
                            start_column,
                            self.current_lexeme.clone(),
                        );
                    }
                    'b' | 'B' => {
                        num_str.push(self.advance().unwrap());
                        while let Some(&ch) = self.peek() {
                            if ch == '0' || ch == '1' || ch == '_' {
                                if ch != '_' {
                                    num_str.push(ch);
                                }
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        let value = i64::from_str_radix(&num_str[2..], 2).unwrap_or(0);
                        return Token::new(
                            TokenKind::Integer(value),
                            start_line,
                            start_column,
                            self.current_lexeme.clone(),
                        );
                    }
                    _ => {}
                }
            }
        }
        
        // Regular decimal number
        while let Some(&ch) = self.peek() {
            if ch.is_ascii_digit() || ch == '_' {
                if ch != '_' {
                    num_str.push(ch);
                }
                self.advance();
            } else {
                break;
            }
        }
        
        // Check for decimal point
        if self.peek_is('.') {
            // Look ahead to see if it's a method call or float
            let mut lookahead = self.source.clone();
            lookahead.next();
            if let Some(ch) = lookahead.peek() {
                if ch.is_ascii_digit() {
                    is_float = true;
                    num_str.push(self.advance().unwrap()); // consume .
                    while let Some(&ch) = self.peek() {
                        if ch.is_ascii_digit() || ch == '_' {
                            if ch != '_' {
                                num_str.push(ch);
                            }
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
            }
        }
        
        // Check for exponent
        if let Some(&ch) = self.peek() {
            if ch == 'e' || ch == 'E' {
                is_float = true;
                num_str.push(self.advance().unwrap());
                if let Some(&sign) = self.peek() {
                    if sign == '+' || sign == '-' {
                        num_str.push(self.advance().unwrap());
                    }
                }
                while let Some(&ch) = self.peek() {
                    if ch.is_ascii_digit() {
                        num_str.push(ch);
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
        }
        
        if is_float {
            let value: f64 = num_str.parse().unwrap_or(0.0);
            Token::new(
                TokenKind::Float(value),
                start_line,
                start_column,
                self.current_lexeme.clone(),
            )
        } else {
            let value: i64 = num_str.parse().unwrap_or(0);
            Token::new(
                TokenKind::Integer(value),
                start_line,
                start_column,
                self.current_lexeme.clone(),
            )
        }
    }

    fn scan_identifier(&mut self, first: char) -> Token {
        let start_line = self.line;
        let start_column = self.column - 1;
        
        let mut ident = String::from(first);
        
        while let Some(&ch) = self.peek() {
            if ch.is_alphanumeric() || ch == '_' {
                ident.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        
        let kind = match ident.as_str() {
            // Keywords
            "let" => TokenKind::Let,
            "mut" => TokenKind::Mut,
            "fn" => TokenKind::Fn,
            "return" => TokenKind::Return,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "loop" => TokenKind::Loop,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "match" => TokenKind::Match,
            "struct" => TokenKind::Struct,
            "enum" => TokenKind::Enum,
            "impl" => TokenKind::Impl,
            "type" => TokenKind::Type,
            "mod" => TokenKind::Mod,
            "use" => TokenKind::Use,
            "pub" => TokenKind::Pub,
            
            // Contract keywords
            "contract" => TokenKind::Contract,
            "requires" => TokenKind::Requires,
            "ensures" => TokenKind::Ensures,
            "invariant" => TokenKind::Invariant,
            
            // Effect keywords
            "effect" => TokenKind::Effect,
            "try" => TokenKind::Try,
            "catch" => TokenKind::Catch,
            
            // AI/Collaboration keywords
            "intent" => TokenKind::Intent,
            "approve" => TokenKind::Approve,
            "observe" => TokenKind::Observe,
            "protocol" => TokenKind::Protocol,
            "async" => TokenKind::Async,
            "await" => TokenKind::Await,
            
            // Literals
            "true" => TokenKind::Bool(true),
            "false" => TokenKind::Bool(false),
            
            // Identifier
            _ => TokenKind::Identifier(ident.clone()),
        };
        
        Token::new(kind, start_line, start_column, ident)
    }

    fn next_token(&mut self) -> Option<Token> {
        self.skip_whitespace();
        self.current_lexeme.clear();
        
        let start_line = self.line;
        let start_column = self.column;
        
        let ch = self.advance()?;
        
        let token = match ch {
            // String literals
            '"' | '\'' => self.scan_string(ch),
            
            // Numbers
            '0'..='9' => self.scan_number(ch),
            
            // Identifiers and keywords
            'a'..='z' | 'A'..='Z' | '_' => self.scan_identifier(ch),
            
            // Operators and punctuation
            '+' => {
                if self.match_char('=') {
                    Token::new(TokenKind::PlusAssign, start_line, start_column, "+=".into())
                } else {
                    Token::new(TokenKind::Plus, start_line, start_column, "+".into())
                }
            }
            '-' => {
                if self.match_char('>') {
                    Token::new(TokenKind::Arrow, start_line, start_column, "->".into())
                } else if self.match_char('=') {
                    Token::new(TokenKind::MinusAssign, start_line, start_column, "-=".into())
                } else {
                    Token::new(TokenKind::Minus, start_line, start_column, "-".into())
                }
            }
            '*' => {
                if self.match_char('=') {
                    Token::new(TokenKind::StarAssign, start_line, start_column, "*=".into())
                } else {
                    Token::new(TokenKind::Star, start_line, start_column, "*".into())
                }
            }
            '/' => {
                if self.match_char('=') {
                    Token::new(TokenKind::SlashAssign, start_line, start_column, "/=".into())
                } else {
                    Token::new(TokenKind::Slash, start_line, start_column, "/".into())
                }
            }
            '%' => Token::new(TokenKind::Percent, start_line, start_column, "%".into()),
            '^' => Token::new(TokenKind::Caret, start_line, start_column, "^".into()),
            
            '=' => {
                if self.match_char('=') {
                    Token::new(TokenKind::Equal, start_line, start_column, "==".into())
                } else if self.match_char('>') {
                    Token::new(TokenKind::FatArrow, start_line, start_column, "=>".into())
                } else {
                    Token::new(TokenKind::Assign, start_line, start_column, "=".into())
                }
            }
            '!' => {
                if self.match_char('=') {
                    Token::new(TokenKind::NotEqual, start_line, start_column, "!=".into())
                } else {
                    Token::new(TokenKind::Not, start_line, start_column, "!".into())
                }
            }
            '<' => {
                if self.match_char('=') {
                    Token::new(TokenKind::LessEqual, start_line, start_column, "<=".into())
                } else {
                    Token::new(TokenKind::Less, start_line, start_column, "<".into())
                }
            }
            '>' => {
                if self.match_char('=') {
                    Token::new(TokenKind::GreaterEqual, start_line, start_column, ">=".into())
                } else {
                    Token::new(TokenKind::Greater, start_line, start_column, ">".into())
                }
            }
            '&' => {
                if self.match_char('&') {
                    Token::new(TokenKind::And, start_line, start_column, "&&".into())
                } else {
                    Token::new(TokenKind::Ampersand, start_line, start_column, "&".into())
                }
            }
            '|' => {
                if self.match_char('|') {
                    Token::new(TokenKind::Or, start_line, start_column, "||".into())
                } else {
                    Token::new(TokenKind::Pipe, start_line, start_column, "|".into())
                }
            }
            
            // Delimiters
            '(' => Token::new(TokenKind::LeftParen, start_line, start_column, "(".into()),
            ')' => Token::new(TokenKind::RightParen, start_line, start_column, ")".into()),
            '{' => Token::new(TokenKind::LeftBrace, start_line, start_column, "{".into()),
            '}' => Token::new(TokenKind::RightBrace, start_line, start_column, "}".into()),
            '[' => Token::new(TokenKind::LeftBracket, start_line, start_column, "[".into()),
            ']' => Token::new(TokenKind::RightBracket, start_line, start_column, "]".into()),
            
            // Punctuation
            ',' => Token::new(TokenKind::Comma, start_line, start_column, ",".into()),
            '.' => Token::new(TokenKind::Dot, start_line, start_column, ".".into()),
            ':' => Token::new(TokenKind::Colon, start_line, start_column, ":".into()),
            ';' => Token::new(TokenKind::Semicolon, start_line, start_column, ";".into()),
            '?' => Token::new(TokenKind::Question, start_line, start_column, "?".into()),
            '@' => Token::new(TokenKind::At, start_line, start_column, "@".into()),
            '#' => Token::new(TokenKind::Hash, start_line, start_column, "#".into()),
            
            _ => Token::new(
                TokenKind::Identifier(ch.to_string()),
                start_line,
                start_column,
                ch.to_string(),
            ),
        };
        
        Some(token)
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_token()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_tokens() {
        let source = "let x = 42;";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        
        assert!(matches!(tokens[0].kind, TokenKind::Let));
        assert!(matches!(tokens[1].kind, TokenKind::Identifier(_)));
        assert!(matches!(tokens[2].kind, TokenKind::Assign));
        assert!(matches!(tokens[3].kind, TokenKind::Integer(42)));
        assert!(matches!(tokens[4].kind, TokenKind::Semicolon));
    }

    #[test]
    fn test_string_literal() {
        let source = r#""hello world""#;
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        
        assert!(matches!(&tokens[0].kind, TokenKind::String(s) if s == "hello world"));
    }

    #[test]
    fn test_function() {
        let source = "fn add(x, y) { return x + y; }";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        
        assert!(matches!(tokens[0].kind, TokenKind::Fn));
    }
}
