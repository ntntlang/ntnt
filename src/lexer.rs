//! Lexer for the Intent programming language
//!
//! Transforms source code into a stream of tokens.

use std::iter::Peekable;
use std::str::Chars;

/// Part of an interpolated string
#[derive(Debug, Clone, PartialEq)]
pub enum StringPart {
    /// Literal string portion
    Literal(String),
    /// Expression to be interpolated (stored as string, parsed later)
    Interpolation(String),
}

/// Part of a template string (triple-quoted)
#[derive(Debug, Clone, PartialEq)]
pub enum TemplatePart {
    /// Literal string portion
    Literal(String),
    /// Expression to interpolate: {{expr}}
    Expr(String),
    /// For loop: {{#for x in items}}...{{/for}}
    ForLoop {
        var: String,
        iterable: String,
        body: Vec<TemplatePart>,
    },
    /// If conditional: {{#if condition}}...{{#else}}...{{/if}}
    IfBlock {
        condition: String,
        then_parts: Vec<TemplatePart>,
        else_parts: Vec<TemplatePart>,
    },
}

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
    Import,
    Export,
    From,
    As,
    Trait,
    For,
    In,
    Defer,
    Where,
    Map, // Map literal keyword

    // Contract keywords
    Contract,
    Requires,
    Ensures,
    Invariant,

    // Effect keywords
    Effect,
    With, // for effect annotations: fn foo() with io
    Pure, // pure function marker
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
    Plus,    // +
    Minus,   // -
    Star,    // *
    Slash,   // /
    Percent, // %
    Caret,   // ^

    // Comparison
    Equal,        // ==
    NotEqual,     // !=
    Less,         // <
    LessEqual,    // <=
    Greater,      // >
    GreaterEqual, // >=

    // Logical
    And, // &&
    Or,  // ||
    Not, // !

    // Assignment
    Assign,      // =
    PlusAssign,  // +=
    MinusAssign, // -=
    StarAssign,  // *=
    SlashAssign, // /=

    // Delimiters
    LeftParen,    // (
    RightParen,   // )
    LeftBrace,    // {
    RightBrace,   // }
    LeftBracket,  // [
    RightBracket, // ]

    // Punctuation
    Comma,            // ,
    Dot,              // .
    Colon,            // :
    ColonColon,       // ::
    Semicolon,        // ;
    Arrow,            // ->
    FatArrow,         // =>
    Question,         // ?
    QuestionQuestion, // ??
    At,               // @
    Hash,             // #
    Ampersand,        // &
    Pipe,             // |
    DotDot,           // ..
    DotDotEqual,      // ..=

    // Raw strings
    RawString(String),

    // Interpolated string parts
    InterpolatedString(Vec<StringPart>),

    // Template string (triple-quoted with {{}} interpolation)
    TemplateString(Vec<TemplatePart>),

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
        Token {
            kind,
            line,
            column,
            lexeme,
        }
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
        let mut has_interpolation = false;
        let mut parts: Vec<StringPart> = Vec::new();

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
                    Some('{') => value.push('{'), // Escape {
                    Some('}') => value.push('}'), // Escape }
                    Some(c) => value.push(c),
                    None => break,
                }
            } else if ch == '{' {
                // Start of interpolation
                has_interpolation = true;
                if !value.is_empty() {
                    parts.push(StringPart::Literal(value.clone()));
                    value.clear();
                }
                self.advance(); // consume '{'

                // Read until matching '}'
                let mut expr_str = String::new();
                let mut brace_count = 1;
                while let Some(&c) = self.peek() {
                    if c == '{' {
                        brace_count += 1;
                        expr_str.push(c);
                    } else if c == '}' {
                        brace_count -= 1;
                        if brace_count == 0 {
                            break;
                        }
                        expr_str.push(c);
                    } else {
                        expr_str.push(c);
                    }
                    self.advance();
                }
                self.advance(); // consume '}'
                parts.push(StringPart::Interpolation(expr_str));
            } else {
                value.push(ch);
                self.advance();
            }
        }

        if has_interpolation {
            if !value.is_empty() {
                parts.push(StringPart::Literal(value));
            }
            Token::new(
                TokenKind::InterpolatedString(parts),
                start_line,
                start_column,
                self.current_lexeme.clone(),
            )
        } else {
            Token::new(
                TokenKind::String(value.clone()),
                start_line,
                start_column,
                format!("{}{}{}", quote, value, quote),
            )
        }
    }

    /// Scan a raw string literal: r"..." or r#"..."# (with any number of #)
    fn scan_raw_string(&mut self, hash_count: usize) -> Token {
        let start_line = self.line;
        let start_column = self.column;
        self.current_lexeme.clear();

        let mut value = String::new();

        // Look for closing quote followed by the same number of #
        loop {
            match self.peek() {
                Some(&'"') => {
                    self.advance();
                    // Check if followed by correct number of #
                    let mut closing_hashes = 0;
                    while closing_hashes < hash_count && self.peek_is('#') {
                        self.advance();
                        closing_hashes += 1;
                    }
                    if closing_hashes == hash_count {
                        // Found the end
                        break;
                    } else {
                        // Not the end, add the quote and any hashes to the value
                        value.push('"');
                        for _ in 0..closing_hashes {
                            value.push('#');
                        }
                    }
                }
                Some(&ch) => {
                    value.push(ch);
                    self.advance();
                }
                None => break, // Unterminated raw string
            }
        }

        Token::new(
            TokenKind::RawString(value),
            start_line,
            start_column,
            self.current_lexeme.clone(),
        )
    }

    /// Scan a template string literal: """..."""
    /// Uses {{expr}} for interpolation (double braces, CSS-safe)
    /// Supports {{#for x in items}}...{{/for}} and {{#if cond}}...{{#else}}...{{/if}}
    fn scan_template_string(&mut self) -> Token {
        let start_line = self.line;
        let start_column = self.column;
        self.current_lexeme.clear();

        let mut content = String::new();

        // Read until closing """
        loop {
            match self.peek() {
                Some(&'"') => {
                    self.advance();
                    if self.peek() == Some(&'"') {
                        self.advance();
                        if self.peek() == Some(&'"') {
                            self.advance();
                            // Found closing """
                            break;
                        } else {
                            // Just two quotes, add them to content
                            content.push('"');
                            content.push('"');
                        }
                    } else {
                        // Just one quote
                        content.push('"');
                    }
                }
                Some(&ch) => {
                    content.push(ch);
                    self.advance();
                }
                None => break, // Unterminated template string
            }
        }

        // Parse the content into TemplateParts
        let parts = self.parse_template_content(&content);

        Token::new(
            TokenKind::TemplateString(parts),
            start_line,
            start_column,
            self.current_lexeme.clone(),
        )
    }

    /// Parse template string content into parts
    #[allow(clippy::only_used_in_recursion)]
    fn parse_template_content(&self, content: &str) -> Vec<TemplatePart> {
        let mut parts = Vec::new();
        let mut chars = content.chars().peekable();
        let mut literal = String::new();

        while let Some(ch) = chars.next() {
            if ch == '\\' {
                // Check for escaped {{ or }}
                if chars.peek() == Some(&'{') {
                    chars.next();
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        literal.push_str("{{");
                    } else {
                        literal.push('{');
                    }
                } else if chars.peek() == Some(&'}') {
                    chars.next();
                    if chars.peek() == Some(&'}') {
                        chars.next();
                        literal.push_str("}}");
                    } else {
                        literal.push('}');
                    }
                } else {
                    literal.push('\\');
                }
            } else if ch == '{' && chars.peek() == Some(&'{') {
                chars.next(); // consume second {

                // Save accumulated literal
                if !literal.is_empty() {
                    parts.push(TemplatePart::Literal(literal.clone()));
                    literal.clear();
                }

                // Read until }}
                let mut expr = String::new();
                let mut brace_depth = 0;

                while let Some(c) = chars.next() {
                    if c == '{' {
                        brace_depth += 1;
                        expr.push(c);
                    } else if c == '}' {
                        if chars.peek() == Some(&'}') && brace_depth == 0 {
                            chars.next(); // consume second }
                            break;
                        } else if brace_depth > 0 {
                            brace_depth -= 1;
                            expr.push(c);
                        } else {
                            expr.push(c);
                        }
                    } else {
                        expr.push(c);
                    }
                }

                // Parse the directive
                let expr = expr.trim();

                if let Some(stripped) = expr.strip_prefix("#for ") {
                    // Parse: #for x in items
                    if let Some((var_part, iter_part)) = stripped.split_once(" in ") {
                        let var = var_part.trim().to_string();
                        let iterable = iter_part.trim().to_string();

                        // Find the body until {{/for}}
                        let rest: String = chars.clone().collect();
                        if let Some(end_pos) = rest.find("{{/for}}") {
                            let body_content = &rest[..end_pos];
                            let body_parts = self.parse_template_content(body_content);

                            // Advance past the body and closing tag
                            for _ in 0..(end_pos + 8) {
                                chars.next();
                            }

                            parts.push(TemplatePart::ForLoop {
                                var,
                                iterable,
                                body: body_parts,
                            });
                        } else {
                            // No closing tag found, treat as literal
                            parts.push(TemplatePart::Literal(format!("{{{{#for {}}}}}", stripped)));
                        }
                    } else {
                        // Invalid for syntax, treat as literal
                        parts.push(TemplatePart::Literal(format!("{{{{#for {}}}}}", stripped)));
                    }
                } else if let Some(stripped) = expr.strip_prefix("#if ") {
                    // Parse: #if condition
                    let condition = stripped.trim().to_string();

                    // Find the body parts until {{/if}} (with optional {{#else}})
                    let rest: String = chars.clone().collect();

                    // Find {{#else}} and {{/if}} positions
                    let else_pos = rest.find("{{#else}}");
                    let endif_pos = rest.find("{{/if}}");

                    if let Some(endif) = endif_pos {
                        let (then_content, else_content) = if let Some(else_p) = else_pos {
                            if else_p < endif {
                                (&rest[..else_p], Some(&rest[(else_p + 9)..endif]))
                            } else {
                                (&rest[..endif], None)
                            }
                        } else {
                            (&rest[..endif], None)
                        };

                        let then_parts = self.parse_template_content(then_content);
                        let else_parts = if let Some(ec) = else_content {
                            self.parse_template_content(ec)
                        } else {
                            Vec::new()
                        };

                        // Advance past everything including closing tag
                        for _ in 0..(endif + 7) {
                            chars.next();
                        }

                        parts.push(TemplatePart::IfBlock {
                            condition,
                            then_parts,
                            else_parts,
                        });
                    } else {
                        // No closing tag found, treat as literal
                        parts.push(TemplatePart::Literal(format!("{{{{#if {}}}}}", stripped)));
                    }
                } else if expr.starts_with("/for")
                    || expr.starts_with("/if")
                    || expr.starts_with("#else")
                {
                    // Closing tags handled above, should not reach here
                    // If we do, it's unmatched - treat as literal
                    parts.push(TemplatePart::Literal(format!("{{{{{}}}}}", expr)));
                } else {
                    // Regular expression interpolation
                    parts.push(TemplatePart::Expr(expr.to_string()));
                }
            } else {
                literal.push(ch);
            }
        }

        // Don't forget remaining literal
        if !literal.is_empty() {
            parts.push(TemplatePart::Literal(literal));
        }

        parts
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

        // Check for raw string: r"..." or r#"..."#
        if first == 'r' {
            if self.peek_is('"') {
                // r"..." - raw string with no hashes
                self.advance(); // consume the opening "
                return self.scan_raw_string(0);
            } else if self.peek_is('#') {
                // Count hashes and check for quote
                let mut hash_count = 0;
                let mut chars_to_consume = Vec::new();

                // Peek ahead to count # and find "
                let mut temp_source = self.source.clone();
                while temp_source.peek() == Some(&'#') {
                    hash_count += 1;
                    chars_to_consume.push(temp_source.next().unwrap());
                }

                if temp_source.peek() == Some(&'"') {
                    // It's a raw string! Consume the hashes and quote
                    for _ in 0..hash_count {
                        self.advance(); // consume #
                    }
                    self.advance(); // consume "
                    return self.scan_raw_string(hash_count);
                }
                // Not a raw string, fall through to normal identifier
            }
        }

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
            "import" => TokenKind::Import,
            "export" => TokenKind::Export,
            "from" => TokenKind::From,
            "as" => TokenKind::As,
            "trait" => TokenKind::Trait,
            "for" => TokenKind::For,
            "in" => TokenKind::In,
            "defer" => TokenKind::Defer,
            "where" => TokenKind::Where,
            "map" => TokenKind::Map,

            // Contract keywords
            "contract" => TokenKind::Contract,
            "requires" => TokenKind::Requires,
            "ensures" => TokenKind::Ensures,
            "invariant" => TokenKind::Invariant,

            // Effect keywords
            "effect" => TokenKind::Effect,
            "with" => TokenKind::With,
            "pure" => TokenKind::Pure,
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
            '"' | '\'' => {
                // Check for triple-quote template string
                if ch == '"' && self.peek() == Some(&'"') {
                    self.advance(); // consume second "
                    if self.peek() == Some(&'"') {
                        self.advance(); // consume third "
                        self.scan_template_string()
                    } else {
                        // Empty string ""
                        Token::new(
                            TokenKind::String(String::new()),
                            start_line,
                            start_column,
                            "\"\"".into(),
                        )
                    }
                } else {
                    self.scan_string(ch)
                }
            }

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
                    Token::new(
                        TokenKind::MinusAssign,
                        start_line,
                        start_column,
                        "-=".into(),
                    )
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
                    Token::new(
                        TokenKind::SlashAssign,
                        start_line,
                        start_column,
                        "/=".into(),
                    )
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
                    Token::new(
                        TokenKind::GreaterEqual,
                        start_line,
                        start_column,
                        ">=".into(),
                    )
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
            ']' => Token::new(
                TokenKind::RightBracket,
                start_line,
                start_column,
                "]".into(),
            ),

            // Punctuation
            ',' => Token::new(TokenKind::Comma, start_line, start_column, ",".into()),
            '.' => {
                if self.match_char('.') {
                    if self.match_char('=') {
                        Token::new(
                            TokenKind::DotDotEqual,
                            start_line,
                            start_column,
                            "..=".into(),
                        )
                    } else {
                        Token::new(TokenKind::DotDot, start_line, start_column, "..".into())
                    }
                } else {
                    Token::new(TokenKind::Dot, start_line, start_column, ".".into())
                }
            }
            ':' => {
                if self.peek() == Some(&':') {
                    self.advance();
                    Token::new(TokenKind::ColonColon, start_line, start_column, "::".into())
                } else {
                    Token::new(TokenKind::Colon, start_line, start_column, ":".into())
                }
            }
            ';' => Token::new(TokenKind::Semicolon, start_line, start_column, ";".into()),
            '?' => {
                if self.peek() == Some(&'?') {
                    self.advance();
                    Token::new(
                        TokenKind::QuestionQuestion,
                        start_line,
                        start_column,
                        "??".into(),
                    )
                } else {
                    Token::new(TokenKind::Question, start_line, start_column, "?".into())
                }
            }
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

impl Iterator for Lexer<'_> {
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
