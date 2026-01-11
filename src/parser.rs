//! Parser for the Intent programming language
//!
//! Transforms a stream of tokens into an Abstract Syntax Tree.

use crate::ast::*;
use crate::error::{IntentError, Result};
use crate::lexer::{Token, TokenKind, StringPart as LexerStringPart};

/// Parser for Intent source code
pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, current: 0 }
    }

    /// Parse a complete program
    pub fn parse(&mut self) -> Result<Program> {
        let mut statements = Vec::new();
        
        while !self.is_at_end() {
            statements.push(self.declaration()?);
        }
        
        Ok(Program { statements })
    }

    // Helper methods
    
    fn is_at_end(&self) -> bool {
        self.current >= self.tokens.len()
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.current)
    }

    fn previous(&self) -> Option<&Token> {
        if self.current > 0 {
            self.tokens.get(self.current - 1)
        } else {
            None
        }
    }

    fn advance(&mut self) -> Option<&Token> {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.previous()
    }

    fn check(&self, kind: &TokenKind) -> bool {
        if let Some(token) = self.peek() {
            std::mem::discriminant(&token.kind) == std::mem::discriminant(kind)
        } else {
            false
        }
    }
    
    fn check_identifier(&self) -> bool {
        if let Some(token) = self.peek() {
            matches!(token.kind, TokenKind::Identifier(_))
        } else {
            false
        }
    }

    fn match_token(&mut self, kinds: &[TokenKind]) -> bool {
        for kind in kinds {
            if self.check(kind) {
                self.advance();
                return true;
            }
        }
        false
    }

    fn consume(&mut self, kind: &TokenKind, message: &str) -> Result<Token> {
        if self.check(kind) {
            Ok(self.advance().unwrap().clone())
        } else {
            let line = self.peek().map(|t| t.line).unwrap_or(0);
            Err(IntentError::ParserError {
                line,
                message: message.to_string(),
            })
        }
    }

    fn current_line(&self) -> usize {
        self.peek().map(|t| t.line).unwrap_or(0)
    }

    // Parsing methods
    
    fn declaration(&mut self) -> Result<Statement> {
        // Check for attributes
        let attributes = self.parse_attributes()?;
        
        if self.match_token(&[TokenKind::Let]) {
            self.let_declaration()
        } else if self.match_token(&[TokenKind::Fn]) {
            self.function_declaration(attributes)
        } else if self.match_token(&[TokenKind::Type]) {
            self.type_alias_declaration()
        } else if self.match_token(&[TokenKind::Struct]) {
            self.struct_declaration(attributes)
        } else if self.match_token(&[TokenKind::Enum]) {
            self.enum_declaration(attributes)
        } else if self.match_token(&[TokenKind::Trait]) {
            self.trait_declaration()
        } else if self.match_token(&[TokenKind::Impl]) {
            self.impl_declaration()
        } else if self.match_token(&[TokenKind::Mod]) {
            self.module_declaration()
        } else if self.match_token(&[TokenKind::Use]) {
            self.use_declaration()
        } else if self.match_token(&[TokenKind::Import]) {
            self.import_declaration()
        } else if self.match_token(&[TokenKind::Export]) {
            self.export_declaration(attributes)
        } else if self.match_token(&[TokenKind::Pub]) {
            self.pub_declaration(attributes)
        } else if self.match_token(&[TokenKind::Protocol]) {
            self.protocol_declaration()
        } else {
            self.statement()
        }
    }

    fn parse_attributes(&mut self) -> Result<Vec<Attribute>> {
        let mut attributes = Vec::new();
        
        while self.match_token(&[TokenKind::Hash]) {
            self.consume(&TokenKind::LeftBracket, "Expected '[' after '#'")?;
            
            let name = self.consume_identifier("Expected attribute name")?;
            let mut args = Vec::new();
            
            if self.match_token(&[TokenKind::LeftParen]) {
                if !self.check(&TokenKind::RightParen) {
                    loop {
                        args.push(self.expression()?);
                        if !self.match_token(&[TokenKind::Comma]) {
                            break;
                        }
                    }
                }
                self.consume(&TokenKind::RightParen, "Expected ')' after attribute arguments")?;
            }
            
            self.consume(&TokenKind::RightBracket, "Expected ']' after attribute")?;
            
            attributes.push(Attribute { name, args });
        }
        
        Ok(attributes)
    }

    fn let_declaration(&mut self) -> Result<Statement> {
        let mutable = self.match_token(&[TokenKind::Mut]);
        
        // Check for pattern destructuring: let (a, b) = ...
        let (name, pattern) = if self.check(&TokenKind::LeftParen) || self.check(&TokenKind::LeftBracket) {
            let pat = self.parse_pattern()?;
            ("_destructure".to_string(), Some(pat))
        } else {
            let name = self.consume_identifier("Expected variable name")?;
            (name, None)
        };
        
        let type_annotation = if self.match_token(&[TokenKind::Colon]) {
            Some(self.parse_type()?)
        } else {
            None
        };
        
        let value = if self.match_token(&[TokenKind::Assign]) {
            Some(self.expression()?)
        } else {
            None
        };
        
        self.match_token(&[TokenKind::Semicolon]);
        
        Ok(Statement::Let {
            name,
            mutable,
            type_annotation,
            value,
            pattern,
        })
    }
    
    fn type_alias_declaration(&mut self) -> Result<Statement> {
        let name = self.consume_identifier("Expected type name")?;
        
        // Optional type parameters: type Foo<T, U>
        let type_params = if self.match_token(&[TokenKind::Less]) {
            let mut params = Vec::new();
            loop {
                params.push(self.consume_identifier("Expected type parameter")?);
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
            }
            self.consume(&TokenKind::Greater, "Expected '>' after type parameters")?;
            params
        } else {
            Vec::new()
        };
        
        self.consume(&TokenKind::Assign, "Expected '=' after type name")?;
        let target = self.parse_type()?;
        self.match_token(&[TokenKind::Semicolon]);
        
        Ok(Statement::TypeAlias {
            name,
            type_params,
            target,
        })
    }

    fn function_declaration(&mut self, attributes: Vec<Attribute>) -> Result<Statement> {
        let name = self.consume_identifier("Expected function name")?;
        
        // Parse optional generic type parameters: fn foo<T, U>()
        let type_params = if self.match_token(&[TokenKind::Less]) {
            let mut params = Vec::new();
            loop {
                params.push(self.consume_identifier("Expected type parameter")?);
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
            }
            self.consume(&TokenKind::Greater, "Expected '>' after type parameters")?;
            params
        } else {
            Vec::new()
        };
        
        self.consume(&TokenKind::LeftParen, "Expected '(' after function name")?;
        let params = self.parse_parameters()?;
        self.consume(&TokenKind::RightParen, "Expected ')' after parameters")?;
        
        let return_type = if self.match_token(&[TokenKind::Arrow]) {
            Some(self.parse_type()?)
        } else {
            None
        };
        
        // Parse optional effect annotation: `with io, async`
        let effects = if self.match_token(&[TokenKind::With]) {
            let mut effs = Vec::new();
            loop {
                effs.push(self.consume_identifier("Expected effect name")?);
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
            }
            effs
        } else if self.match_token(&[TokenKind::Pure]) {
            vec!["pure".to_string()]
        } else {
            Vec::new()
        };
        
        // Parse contract (requires/ensures)
        let contract = self.parse_contract()?;
        
        self.consume(&TokenKind::LeftBrace, "Expected '{' before function body")?;
        let body = self.block()?;
        
        Ok(Statement::Function {
            name,
            params,
            return_type,
            contract,
            body,
            attributes,
            type_params,
            effects,
        })
    }

    fn parse_parameters(&mut self) -> Result<Vec<Parameter>> {
        let mut params = Vec::new();
        
        if !self.check(&TokenKind::RightParen) {
            loop {
                let name = self.consume_identifier("Expected parameter name")?;
                
                let type_annotation = if self.match_token(&[TokenKind::Colon]) {
                    Some(self.parse_type()?)
                } else {
                    None
                };
                
                let default = if self.match_token(&[TokenKind::Assign]) {
                    Some(self.expression()?)
                } else {
                    None
                };
                
                params.push(Parameter {
                    name,
                    type_annotation,
                    default,
                });
                
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
            }
        }
        
        Ok(params)
    }

    fn parse_contract(&mut self) -> Result<Option<Contract>> {
        let mut requires = Vec::new();
        let mut ensures = Vec::new();
        
        while self.match_token(&[TokenKind::Requires]) {
            requires.push(self.expression()?);
        }
        
        while self.match_token(&[TokenKind::Ensures]) {
            ensures.push(self.expression()?);
        }
        
        if requires.is_empty() && ensures.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Contract { requires, ensures }))
        }
    }

    fn struct_declaration(&mut self, attributes: Vec<Attribute>) -> Result<Statement> {
        let name = self.consume_identifier("Expected struct name")?;
        
        // Parse optional generic type parameters: struct Foo<T, U>
        let type_params = if self.match_token(&[TokenKind::Less]) {
            let mut params = Vec::new();
            loop {
                params.push(self.consume_identifier("Expected type parameter")?);
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
            }
            self.consume(&TokenKind::Greater, "Expected '>' after type parameters")?;
            params
        } else {
            Vec::new()
        };
        
        self.consume(&TokenKind::LeftBrace, "Expected '{' after struct name")?;
        
        let mut fields = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let public = self.match_token(&[TokenKind::Pub]);
            let field_name = self.consume_identifier("Expected field name")?;
            self.consume(&TokenKind::Colon, "Expected ':' after field name")?;
            let type_annotation = self.parse_type()?;
            
            fields.push(Field {
                name: field_name,
                type_annotation,
                public,
            });
            
            if !self.match_token(&[TokenKind::Comma]) {
                break;
            }
        }
        
        self.consume(&TokenKind::RightBrace, "Expected '}' after struct fields")?;
        
        Ok(Statement::Struct {
            name,
            fields,
            attributes,
            type_params,
        })
    }

    fn enum_declaration(&mut self, attributes: Vec<Attribute>) -> Result<Statement> {
        let name = self.consume_identifier("Expected enum name")?;
        
        // Parse optional generic type parameters: enum Option<T>
        let type_params = if self.match_token(&[TokenKind::Less]) {
            let mut params = Vec::new();
            loop {
                params.push(self.consume_identifier("Expected type parameter")?);
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
            }
            self.consume(&TokenKind::Greater, "Expected '>' after type parameters")?;
            params
        } else {
            Vec::new()
        };
        
        self.consume(&TokenKind::LeftBrace, "Expected '{' after enum name")?;
        
        let mut variants = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let variant_name = self.consume_identifier("Expected variant name")?;
            
            let fields = if self.match_token(&[TokenKind::LeftParen]) {
                let mut variant_fields = Vec::new();
                if !self.check(&TokenKind::RightParen) {
                    loop {
                        variant_fields.push(self.parse_type()?);
                        if !self.match_token(&[TokenKind::Comma]) {
                            break;
                        }
                    }
                }
                self.consume(&TokenKind::RightParen, "Expected ')' after variant fields")?;
                Some(variant_fields)
            } else {
                None
            };
            
            variants.push(EnumVariant {
                name: variant_name,
                fields,
            });
            
            if !self.match_token(&[TokenKind::Comma]) {
                break;
            }
        }
        
        self.consume(&TokenKind::RightBrace, "Expected '}' after enum variants")?;
        
        Ok(Statement::Enum {
            name,
            variants,
            attributes,
            type_params,
        })
    }

    fn trait_declaration(&mut self) -> Result<Statement> {
        let name = self.consume_identifier("Expected trait name")?;
        
        // Parse optional type parameters: trait Foo<T>
        let type_params = if self.match_token(&[TokenKind::Less]) {
            let mut params = Vec::new();
            loop {
                params.push(self.consume_identifier("Expected type parameter")?);
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
            }
            self.consume(&TokenKind::Greater, "Expected '>' after type parameters")?;
            params
        } else {
            Vec::new()
        };
        
        // Parse optional supertraits: trait Foo: Bar + Baz
        let supertraits = if self.match_token(&[TokenKind::Colon]) {
            let mut traits = Vec::new();
            loop {
                traits.push(self.consume_identifier("Expected trait name")?);
                if !self.match_token(&[TokenKind::Plus]) {
                    break;
                }
            }
            traits
        } else {
            Vec::new()
        };
        
        self.consume(&TokenKind::LeftBrace, "Expected '{' after trait declaration")?;
        
        let mut methods = Vec::new();
        
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            if self.match_token(&[TokenKind::Fn]) {
                methods.push(self.trait_method()?);
            }
        }
        
        self.consume(&TokenKind::RightBrace, "Expected '}' after trait body")?;
        
        Ok(Statement::Trait {
            name,
            type_params,
            methods,
            supertraits,
        })
    }
    
    fn trait_method(&mut self) -> Result<TraitMethod> {
        let name = self.consume_identifier("Expected method name")?;
        
        self.consume(&TokenKind::LeftParen, "Expected '(' after method name")?;
        
        // Parse parameters
        let mut params = Vec::new();
        if !self.check(&TokenKind::RightParen) {
            loop {
                // Handle 'self' parameter
                if self.check_identifier() {
                    let param_name = self.consume_identifier("Expected parameter name")?;
                    
                    let type_annotation = if self.match_token(&[TokenKind::Colon]) {
                        Some(self.parse_type()?)
                    } else {
                        None
                    };
                    
                    params.push(Parameter {
                        name: param_name,
                        type_annotation,
                        default: None,
                    });
                }
                
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
            }
        }
        
        self.consume(&TokenKind::RightParen, "Expected ')' after parameters")?;
        
        // Parse return type
        let return_type = if self.match_token(&[TokenKind::Arrow]) {
            Some(self.parse_type()?)
        } else {
            None
        };
        
        // Parse optional contract
        let contract = self.parse_contract()?;
        
        // Parse optional default body or just semicolon
        let default_body = if self.match_token(&[TokenKind::LeftBrace]) {
            // Default implementation
            let mut statements = Vec::new();
            while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
                statements.push(self.declaration()?);
            }
            self.consume(&TokenKind::RightBrace, "Expected '}' after method body")?;
            Some(Block { statements })
        } else {
            self.match_token(&[TokenKind::Semicolon]);
            None
        };
        
        Ok(TraitMethod {
            name,
            params,
            return_type,
            contract,
            default_body,
        })
    }

    fn impl_declaration(&mut self) -> Result<Statement> {
        let first_name = self.consume_identifier("Expected type or trait name")?;
        
        // Check if this is `impl Trait for Type` or just `impl Type`
        let (trait_name, type_name) = if self.match_token(&[TokenKind::For]) {
            let type_name = self.consume_identifier("Expected type name after 'for'")?;
            (Some(first_name), type_name)
        } else {
            (None, first_name)
        };
        
        self.consume(&TokenKind::LeftBrace, "Expected '{' after type name")?;
        
        let mut methods = Vec::new();
        let mut invariants = Vec::new();
        
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            if self.match_token(&[TokenKind::Invariant]) {
                invariants.push(self.expression()?);
                self.match_token(&[TokenKind::Semicolon]);
            } else {
                let attrs = self.parse_attributes()?;
                if self.match_token(&[TokenKind::Fn]) {
                    methods.push(self.function_declaration(attrs)?);
                }
            }
        }
        
        self.consume(&TokenKind::RightBrace, "Expected '}' after impl block")?;
        
        Ok(Statement::Impl {
            type_name,
            trait_name,
            methods,
            invariants,
        })
    }

    fn module_declaration(&mut self) -> Result<Statement> {
        let name = self.consume_identifier("Expected module name")?;
        
        self.consume(&TokenKind::LeftBrace, "Expected '{' after module name")?;
        
        let mut body = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            body.push(self.declaration()?);
        }
        
        self.consume(&TokenKind::RightBrace, "Expected '}' after module body")?;
        
        Ok(Statement::Module { name, body })
    }

    fn use_declaration(&mut self) -> Result<Statement> {
        let mut path = Vec::new();
        
        loop {
            path.push(self.consume_identifier("Expected module path")?);
            if !self.match_token(&[TokenKind::Colon]) || !self.match_token(&[TokenKind::Colon]) {
                break;
            }
        }
        
        self.match_token(&[TokenKind::Semicolon]);
        
        Ok(Statement::Use { path })
    }
    
    /// Parse import declaration: `import { a, b as c } from "module"` or `import "module" as alias`
    fn import_declaration(&mut self) -> Result<Statement> {
        let mut items = Vec::new();
        let mut alias = None;
        
        // Check for selective imports: import { a, b, c }
        if self.match_token(&[TokenKind::LeftBrace]) {
            loop {
                let name = self.consume_identifier("Expected import name")?;
                let item_alias = if self.match_token(&[TokenKind::As]) {
                    Some(self.consume_identifier("Expected alias name")?)
                } else {
                    None
                };
                items.push(ImportItem { name, alias: item_alias });
                
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
            }
            self.consume(&TokenKind::RightBrace, "Expected '}' after import items")?;
            
            self.consume(&TokenKind::From, "Expected 'from' after import items")?;
            
            // Parse the module source (string literal)
            let source = self.parse_module_source()?;
            
            self.match_token(&[TokenKind::Semicolon]);
            
            Ok(Statement::Import {
                items,
                source,
                alias,
            })
        } else if let Some(token) = self.peek() {
            // Check for string literal: import "module" as alias
            if let TokenKind::String(ref s) = token.kind {
                let source = s.clone();
                self.advance();
                
                if self.match_token(&[TokenKind::As]) {
                    alias = Some(self.consume_identifier("Expected alias name")?);
                }
                
                self.match_token(&[TokenKind::Semicolon]);
                
                return Ok(Statement::Import {
                    items: vec![],
                    source,
                    alias,
                });
            }
            
            // Import entire module by name: import http as web from "module"
            let name = self.consume_identifier("Expected module name")?;
            if self.match_token(&[TokenKind::As]) {
                alias = Some(self.consume_identifier("Expected alias name")?);
            }
            
            // If there's no 'from', the name itself is the source
            if !self.check(&TokenKind::From) {
                self.match_token(&[TokenKind::Semicolon]);
                return Ok(Statement::Import {
                    items: vec![],
                    source: name,
                    alias,
                });
            }
            
            // With 'from', name becomes an item
            items.push(ImportItem { name: name.clone(), alias: None });
            
            self.consume(&TokenKind::From, "Expected 'from' after import items")?;
            
            // Parse the module source (string literal)
            let source = self.parse_module_source()?;
            
            self.match_token(&[TokenKind::Semicolon]);
            
            Ok(Statement::Import {
                items,
                source,
                alias: None,
            })
        } else {
            Err(IntentError::ParserError {
                line: self.current_line(),
                message: "Expected import specifier".to_string(),
            })
        }
    }
    
    fn parse_module_source(&mut self) -> Result<String> {
        if let Some(token) = self.peek() {
            if let TokenKind::String(ref s) = token.kind {
                let s = s.clone();
                self.advance();
                return Ok(s);
            } else if let TokenKind::Identifier(ref s) = token.kind {
                // Allow bare identifiers for std modules: from std/http
                let mut path = s.clone();
                self.advance();
                while self.match_token(&[TokenKind::Slash]) {
                    path.push('/');
                    path.push_str(&self.consume_identifier("Expected path segment")?);
                }
                return Ok(path);
            }
        }
        Err(IntentError::ParserError {
            line: self.current_line(),
            message: "Expected module path string".to_string(),
        })
    }
    
    /// Parse export declaration: `export fn foo()` or `export { a, b }`
    fn export_declaration(&mut self, attributes: Vec<Attribute>) -> Result<Statement> {
        // Check for list export: export { a, b }
        if self.match_token(&[TokenKind::LeftBrace]) {
            let mut items = Vec::new();
            loop {
                items.push(self.consume_identifier("Expected export name")?);
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
            }
            self.consume(&TokenKind::RightBrace, "Expected '}' after export items")?;
            self.match_token(&[TokenKind::Semicolon]);
            return Ok(Statement::Export { items, statement: None });
        }
        
        // Export a declaration: export fn foo() or export struct Bar
        let stmt = if self.match_token(&[TokenKind::Fn]) {
            self.function_declaration(attributes)?
        } else if self.match_token(&[TokenKind::Struct]) {
            self.struct_declaration(attributes)?
        } else if self.match_token(&[TokenKind::Enum]) {
            self.enum_declaration(attributes)?
        } else if self.match_token(&[TokenKind::Type]) {
            self.type_alias_declaration()?
        } else if self.match_token(&[TokenKind::Let]) {
            self.let_declaration()?
        } else {
            return Err(IntentError::ParserError {
                line: self.current_line(),
                message: "Expected declaration after 'export'".to_string(),
            });
        };
        
        Ok(Statement::Export {
            items: vec![],
            statement: Some(Box::new(stmt)),
        })
    }
    
    /// Parse pub declaration: `pub fn foo()` - shorthand for export
    fn pub_declaration(&mut self, attributes: Vec<Attribute>) -> Result<Statement> {
        // pub is just syntactic sugar for export
        self.export_declaration(attributes)
    }

    fn protocol_declaration(&mut self) -> Result<Statement> {
        let name = self.consume_identifier("Expected protocol name")?;
        
        self.consume(&TokenKind::LeftBrace, "Expected '{' after protocol name")?;
        
        // Simplified protocol parsing for now
        let steps = Vec::new();
        
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            self.advance(); // Skip for now
        }
        
        self.consume(&TokenKind::RightBrace, "Expected '}' after protocol body")?;
        
        Ok(Statement::Protocol { name, steps })
    }

    fn statement(&mut self) -> Result<Statement> {
        if self.match_token(&[TokenKind::Return]) {
            let value = if self.check(&TokenKind::Semicolon) || self.check(&TokenKind::RightBrace) {
                None
            } else {
                Some(self.expression()?)
            };
            self.match_token(&[TokenKind::Semicolon]);
            Ok(Statement::Return(value))
        } else if self.match_token(&[TokenKind::If]) {
            self.if_statement()
        } else if self.match_token(&[TokenKind::While]) {
            self.while_statement()
        } else if self.match_token(&[TokenKind::Loop]) {
            self.loop_statement()
        } else if self.match_token(&[TokenKind::For]) {
            self.for_in_statement()
        } else if self.match_token(&[TokenKind::Defer]) {
            self.defer_statement()
        } else if self.match_token(&[TokenKind::Break]) {
            self.match_token(&[TokenKind::Semicolon]);
            Ok(Statement::Break)
        } else if self.match_token(&[TokenKind::Continue]) {
            self.match_token(&[TokenKind::Semicolon]);
            Ok(Statement::Continue)
        } else {
            let expr = self.expression()?;
            self.match_token(&[TokenKind::Semicolon]);
            Ok(Statement::Expression(expr))
        }
    }
    
    fn for_in_statement(&mut self) -> Result<Statement> {
        let variable = self.consume_identifier("Expected variable name after 'for'")?;
        
        self.consume(&TokenKind::In, "Expected 'in' after for variable")?;
        
        let iterable = self.expression()?;
        
        self.consume(&TokenKind::LeftBrace, "Expected '{' after for iterable")?;
        let body = self.block()?;
        
        Ok(Statement::ForIn {
            variable,
            iterable,
            body,
        })
    }
    
    fn defer_statement(&mut self) -> Result<Statement> {
        let expr = self.expression()?;
        self.match_token(&[TokenKind::Semicolon]);
        Ok(Statement::Defer(expr))
    }

    fn if_statement(&mut self) -> Result<Statement> {
        let condition = self.expression()?;
        
        self.consume(&TokenKind::LeftBrace, "Expected '{' after if condition")?;
        let then_branch = self.block()?;
        
        let else_branch = if self.match_token(&[TokenKind::Else]) {
            if self.match_token(&[TokenKind::If]) {
                // else if
                let else_if = self.if_statement()?;
                Some(Block {
                    statements: vec![else_if],
                })
            } else {
                self.consume(&TokenKind::LeftBrace, "Expected '{' after else")?;
                Some(self.block()?)
            }
        } else {
            None
        };
        
        Ok(Statement::If {
            condition,
            then_branch,
            else_branch,
        })
    }

    fn while_statement(&mut self) -> Result<Statement> {
        let condition = self.expression()?;
        
        self.consume(&TokenKind::LeftBrace, "Expected '{' after while condition")?;
        let body = self.block()?;
        
        Ok(Statement::While { condition, body })
    }

    fn loop_statement(&mut self) -> Result<Statement> {
        self.consume(&TokenKind::LeftBrace, "Expected '{' after loop")?;
        let body = self.block()?;
        
        Ok(Statement::Loop { body })
    }

    fn block(&mut self) -> Result<Block> {
        let mut statements = Vec::new();
        
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            statements.push(self.declaration()?);
        }
        
        self.consume(&TokenKind::RightBrace, "Expected '}' after block")?;
        
        Ok(Block { statements })
    }

    // Expression parsing with precedence climbing

    fn expression(&mut self) -> Result<Expression> {
        self.assignment()
    }

    fn assignment(&mut self) -> Result<Expression> {
        let expr = self.or()?;
        
        if self.match_token(&[TokenKind::Assign]) {
            let value = self.assignment()?;
            return Ok(Expression::Assign {
                target: Box::new(expr),
                value: Box::new(value),
            });
        }
        
        Ok(expr)
    }

    fn or(&mut self) -> Result<Expression> {
        let mut expr = self.and()?;
        
        while self.match_token(&[TokenKind::Or]) {
            let right = self.and()?;
            expr = Expression::Binary {
                left: Box::new(expr),
                operator: BinaryOp::Or,
                right: Box::new(right),
            };
        }
        
        Ok(expr)
    }

    fn and(&mut self) -> Result<Expression> {
        let mut expr = self.equality()?;
        
        while self.match_token(&[TokenKind::And]) {
            let right = self.equality()?;
            expr = Expression::Binary {
                left: Box::new(expr),
                operator: BinaryOp::And,
                right: Box::new(right),
            };
        }
        
        Ok(expr)
    }

    fn equality(&mut self) -> Result<Expression> {
        let mut expr = self.comparison()?;
        
        while self.match_token(&[TokenKind::Equal, TokenKind::NotEqual]) {
            let operator = match self.previous().map(|t| &t.kind) {
                Some(TokenKind::Equal) => BinaryOp::Eq,
                Some(TokenKind::NotEqual) => BinaryOp::Ne,
                _ => unreachable!(),
            };
            let right = self.comparison()?;
            expr = Expression::Binary {
                left: Box::new(expr),
                operator,
                right: Box::new(right),
            };
        }
        
        Ok(expr)
    }

    fn comparison(&mut self) -> Result<Expression> {
        let mut expr = self.range()?;
        
        while self.match_token(&[
            TokenKind::Less,
            TokenKind::LessEqual,
            TokenKind::Greater,
            TokenKind::GreaterEqual,
        ]) {
            let operator = match self.previous().map(|t| &t.kind) {
                Some(TokenKind::Less) => BinaryOp::Lt,
                Some(TokenKind::LessEqual) => BinaryOp::Le,
                Some(TokenKind::Greater) => BinaryOp::Gt,
                Some(TokenKind::GreaterEqual) => BinaryOp::Ge,
                _ => unreachable!(),
            };
            let right = self.range()?;
            expr = Expression::Binary {
                left: Box::new(expr),
                operator,
                right: Box::new(right),
            };
        }
        
        Ok(expr)
    }
    
    fn range(&mut self) -> Result<Expression> {
        let expr = self.term()?;
        
        // Check for range operators: .. or ..=
        if self.match_token(&[TokenKind::DotDot]) {
            let end = self.term()?;
            return Ok(Expression::Range {
                start: Box::new(expr),
                end: Box::new(end),
                inclusive: false,
            });
        } else if self.match_token(&[TokenKind::DotDotEqual]) {
            let end = self.term()?;
            return Ok(Expression::Range {
                start: Box::new(expr),
                end: Box::new(end),
                inclusive: true,
            });
        }
        
        Ok(expr)
    }

    fn term(&mut self) -> Result<Expression> {
        let mut expr = self.factor()?;
        
        while self.match_token(&[TokenKind::Plus, TokenKind::Minus]) {
            let operator = match self.previous().map(|t| &t.kind) {
                Some(TokenKind::Plus) => BinaryOp::Add,
                Some(TokenKind::Minus) => BinaryOp::Sub,
                _ => unreachable!(),
            };
            let right = self.factor()?;
            expr = Expression::Binary {
                left: Box::new(expr),
                operator,
                right: Box::new(right),
            };
        }
        
        Ok(expr)
    }

    fn factor(&mut self) -> Result<Expression> {
        let mut expr = self.unary()?;
        
        while self.match_token(&[TokenKind::Star, TokenKind::Slash, TokenKind::Percent]) {
            let operator = match self.previous().map(|t| &t.kind) {
                Some(TokenKind::Star) => BinaryOp::Mul,
                Some(TokenKind::Slash) => BinaryOp::Div,
                Some(TokenKind::Percent) => BinaryOp::Mod,
                _ => unreachable!(),
            };
            let right = self.unary()?;
            expr = Expression::Binary {
                left: Box::new(expr),
                operator,
                right: Box::new(right),
            };
        }
        
        Ok(expr)
    }

    fn unary(&mut self) -> Result<Expression> {
        if self.match_token(&[TokenKind::Not, TokenKind::Minus]) {
            let operator = match self.previous().map(|t| &t.kind) {
                Some(TokenKind::Not) => UnaryOp::Not,
                Some(TokenKind::Minus) => UnaryOp::Neg,
                _ => unreachable!(),
            };
            let operand = self.unary()?;
            return Ok(Expression::Unary {
                operator,
                operand: Box::new(operand),
            });
        }
        
        self.call()
    }

    fn call(&mut self) -> Result<Expression> {
        let mut expr = self.primary()?;
        
        loop {
            if self.match_token(&[TokenKind::LeftParen]) {
                expr = self.finish_call(expr)?;
            } else if self.check(&TokenKind::LeftBrace) {
                // Check if this is a struct literal (Identifier followed by { name: })
                // Only treat as struct literal if it's an identifier and looks like struct syntax
                if let Expression::Identifier(name) = &expr {
                    if self.is_struct_literal() {
                        self.advance(); // consume the {
                        expr = self.finish_struct_literal(name.clone())?;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            } else if self.match_token(&[TokenKind::Dot]) {
                let name = self.consume_identifier("Expected property name after '.'")?;
                if self.match_token(&[TokenKind::LeftParen]) {
                    let arguments = self.arguments()?;
                    self.consume(&TokenKind::RightParen, "Expected ')' after arguments")?;
                    expr = Expression::MethodCall {
                        object: Box::new(expr),
                        method: name,
                        arguments,
                    };
                } else {
                    expr = Expression::FieldAccess {
                        object: Box::new(expr),
                        field: name,
                    };
                }
            } else if self.match_token(&[TokenKind::LeftBracket]) {
                let index = self.expression()?;
                self.consume(&TokenKind::RightBracket, "Expected ']' after index")?;
                expr = Expression::Index {
                    object: Box::new(expr),
                    index: Box::new(index),
                };
            } else {
                break;
            }
        }
        
        Ok(expr)
    }

    fn finish_call(&mut self, callee: Expression) -> Result<Expression> {
        let arguments = self.arguments()?;
        self.consume(&TokenKind::RightParen, "Expected ')' after arguments")?;
        
        Ok(Expression::Call {
            function: Box::new(callee),
            arguments,
        })
    }

    fn finish_struct_literal(&mut self, name: String) -> Result<Expression> {
        let mut fields = Vec::new();
        
        if !self.check(&TokenKind::RightBrace) {
            loop {
                let field_name = self.consume_identifier("Expected field name")?;
                self.consume(&TokenKind::Colon, "Expected ':' after field name")?;
                let value = self.expression()?;
                fields.push((field_name, value));
                
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
            }
        }
        
        self.consume(&TokenKind::RightBrace, "Expected '}' after struct fields")?;
        
        Ok(Expression::StructLiteral { name, fields })
    }
    
    /// Check if the upcoming tokens look like a struct literal ({ name: value })
    /// This looks ahead without consuming tokens
    fn is_struct_literal(&self) -> bool {
        // Look for pattern: { identifier :
        let mut pos = self.current;
        
        // Check for {
        if pos >= self.tokens.len() {
            return false;
        }
        if !matches!(self.tokens.get(pos).map(|t| &t.kind), Some(TokenKind::LeftBrace)) {
            return false;
        }
        pos += 1;
        
        // Check for } (empty struct literal)
        if matches!(self.tokens.get(pos).map(|t| &t.kind), Some(TokenKind::RightBrace)) {
            return true;
        }
        
        // Check for identifier
        if !matches!(self.tokens.get(pos).map(|t| &t.kind), Some(TokenKind::Identifier(_))) {
            return false;
        }
        pos += 1;
        
        // Check for :
        matches!(self.tokens.get(pos).map(|t| &t.kind), Some(TokenKind::Colon))
    }

    fn arguments(&mut self) -> Result<Vec<Expression>> {
        let mut args = Vec::new();
        
        if !self.check(&TokenKind::RightParen) {
            loop {
                args.push(self.expression()?);
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
            }
        }
        
        Ok(args)
    }

    fn primary(&mut self) -> Result<Expression> {
        // Integer literal
        if let Some(token) = self.peek() {
            if let TokenKind::Integer(n) = token.kind {
                self.advance();
                return Ok(Expression::Integer(n));
            }
        }

        // Float literal
        if let Some(token) = self.peek() {
            if let TokenKind::Float(n) = token.kind {
                self.advance();
                return Ok(Expression::Float(n));
            }
        }

        // String literal
        if let Some(token) = self.peek() {
            if let TokenKind::String(ref s) = token.kind {
                let s = s.clone();
                self.advance();
                return Ok(Expression::String(s));
            }
        }
        
        // Interpolated string literal
        if let Some(token) = self.peek() {
            if let TokenKind::InterpolatedString(ref parts) = token.kind {
                let parts = parts.clone();
                self.advance();
                return self.parse_interpolated_string(&parts);
            }
        }

        // Boolean literal
        if let Some(token) = self.peek() {
            if let TokenKind::Bool(b) = token.kind {
                self.advance();
                return Ok(Expression::Bool(b));
            }
        }

        // Identifier (or EnumName::Variant)
        if let Some(token) = self.peek() {
            if let TokenKind::Identifier(ref name) = token.kind {
                let name = name.clone();
                self.advance();
                
                // Check for enum variant access: EnumName::Variant
                if self.check(&TokenKind::ColonColon) {
                    self.advance(); // consume ::
                    if let Some(variant_token) = self.peek() {
                        if let TokenKind::Identifier(ref variant_name) = variant_token.kind {
                            let variant = variant_name.clone();
                            self.advance();
                            
                            // Check for arguments: EnumName::Variant(args)
                            let arguments = if self.check(&TokenKind::LeftParen) {
                                self.advance();
                                let mut args = Vec::new();
                                if !self.check(&TokenKind::RightParen) {
                                    loop {
                                        args.push(self.expression()?);
                                        if !self.match_token(&[TokenKind::Comma]) {
                                            break;
                                        }
                                    }
                                }
                                self.consume(&TokenKind::RightParen, "Expected ')' after arguments")?;
                                args
                            } else {
                                Vec::new()
                            };
                            
                            return Ok(Expression::EnumVariant {
                                enum_name: name,
                                variant,
                                arguments,
                            });
                        }
                    }
                    return Err(IntentError::ParserError {
                        line: self.current_line(),
                        message: "Expected variant name after '::'".to_string(),
                    });
                }
                
                return Ok(Expression::Identifier(name));
            }
        }

        // Parenthesized expression or tuple
        if self.match_token(&[TokenKind::LeftParen]) {
            if self.match_token(&[TokenKind::RightParen]) {
                return Ok(Expression::Unit);
            }
            let expr = self.expression()?;
            self.consume(&TokenKind::RightParen, "Expected ')' after expression")?;
            return Ok(expr);
        }

        // Array literal
        if self.match_token(&[TokenKind::LeftBracket]) {
            let mut elements = Vec::new();
            if !self.check(&TokenKind::RightBracket) {
                loop {
                    elements.push(self.expression()?);
                    if !self.match_token(&[TokenKind::Comma]) {
                        break;
                    }
                }
            }
            self.consume(&TokenKind::RightBracket, "Expected ']' after array elements")?;
            return Ok(Expression::Array(elements));
        }
        
        // Map literal: map { key: value, ... }
        if self.match_token(&[TokenKind::Map]) {
            self.consume(&TokenKind::LeftBrace, "Expected '{' after 'map'")?;
            let mut pairs = Vec::new();
            if !self.check(&TokenKind::RightBrace) {
                loop {
                    let key = self.expression()?;
                    self.consume(&TokenKind::Colon, "Expected ':' after map key")?;
                    let value = self.expression()?;
                    pairs.push((key, value));
                    if !self.match_token(&[TokenKind::Comma]) {
                        break;
                    }
                }
            }
            self.consume(&TokenKind::RightBrace, "Expected '}' after map entries")?;
            return Ok(Expression::MapLiteral(pairs));
        }

        // Block expression
        if self.match_token(&[TokenKind::LeftBrace]) {
            let block = self.block()?;
            return Ok(Expression::Block(block));
        }
        
        // Match expression
        if self.match_token(&[TokenKind::Match]) {
            return self.match_expression();
        }

        Err(IntentError::ParserError {
            line: self.current_line(),
            message: "Expected expression".to_string(),
        })
    }
    
    /// Parse an interpolated string into StringParts
    fn parse_interpolated_string(&mut self, parts: &[LexerStringPart]) -> Result<Expression> {
        let mut ast_parts = Vec::new();
        
        for part in parts {
            match part {
                LexerStringPart::Literal(s) => {
                    ast_parts.push(StringPart::Literal(s.clone()));
                }
                LexerStringPart::Interpolation(expr_str) => {
                    // Parse the expression string
                    let lexer = crate::lexer::Lexer::new(expr_str);
                    let tokens: Vec<_> = lexer.collect();
                    let mut parser = Parser::new(tokens);
                    let expr = parser.expression()?;
                    ast_parts.push(StringPart::Expr(expr));
                }
            }
        }
        
        Ok(Expression::InterpolatedString(ast_parts))
    }
    
    /// Parse a match expression: match expr { pattern => body, ... }
    fn match_expression(&mut self) -> Result<Expression> {
        let scrutinee = self.expression()?;
        
        self.consume(&TokenKind::LeftBrace, "Expected '{' after match expression")?;
        
        let mut arms = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let pattern = self.parse_pattern()?;
            
            // Optional guard: `if condition`
            let guard = if self.match_token(&[TokenKind::If]) {
                Some(self.expression()?)
            } else {
                None
            };
            
            self.consume(&TokenKind::FatArrow, "Expected '=>' after pattern")?;
            
            let body = self.expression()?;
            
            arms.push(MatchArm { pattern, guard, body });
            
            // Optional comma or newline between arms
            self.match_token(&[TokenKind::Comma]);
        }
        
        self.consume(&TokenKind::RightBrace, "Expected '}' after match arms")?;
        
        Ok(Expression::Match {
            scrutinee: Box::new(scrutinee),
            arms,
        })
    }
    
    /// Parse a pattern for match expressions
    fn parse_pattern(&mut self) -> Result<Pattern> {
        // Wildcard pattern: _
        if let Some(token) = self.peek() {
            if let TokenKind::Identifier(ref name) = token.kind {
                if name == "_" {
                    self.advance();
                    return Ok(Pattern::Wildcard);
                }
            }
        }
        
        // Literal patterns
        if let Some(token) = self.peek() {
            match &token.kind {
                TokenKind::Integer(n) => {
                    let n = *n;
                    self.advance();
                    return Ok(Pattern::Literal(Expression::Integer(n)));
                }
                TokenKind::Float(n) => {
                    let n = *n;
                    self.advance();
                    return Ok(Pattern::Literal(Expression::Float(n)));
                }
                TokenKind::String(s) => {
                    let s = s.clone();
                    self.advance();
                    return Ok(Pattern::Literal(Expression::String(s)));
                }
                TokenKind::Bool(b) => {
                    let b = *b;
                    self.advance();
                    return Ok(Pattern::Literal(Expression::Bool(b)));
                }
                _ => {}
            }
        }
        
        // Array pattern: [pat1, pat2, ...]
        if self.match_token(&[TokenKind::LeftBracket]) {
            let mut patterns = Vec::new();
            if !self.check(&TokenKind::RightBracket) {
                loop {
                    patterns.push(self.parse_pattern()?);
                    if !self.match_token(&[TokenKind::Comma]) {
                        break;
                    }
                }
            }
            self.consume(&TokenKind::RightBracket, "Expected ']' after array pattern")?;
            return Ok(Pattern::Array(patterns));
        }
        
        // Tuple pattern: (pat1, pat2, ...)
        if self.match_token(&[TokenKind::LeftParen]) {
            let mut patterns = Vec::new();
            if !self.check(&TokenKind::RightParen) {
                loop {
                    patterns.push(self.parse_pattern()?);
                    if !self.match_token(&[TokenKind::Comma]) {
                        break;
                    }
                }
            }
            self.consume(&TokenKind::RightParen, "Expected ')' after tuple pattern")?;
            return Ok(Pattern::Tuple(patterns));
        }
        
        // Identifier-based patterns (variable binding, struct, or variant)
        if let Some(token) = self.peek() {
            if let TokenKind::Identifier(ref name) = token.kind {
                let name = name.clone();
                self.advance();
                
                // Check for qualified variant: EnumName::Variant or EnumName::Variant(fields)
                if self.check(&TokenKind::ColonColon) {
                    self.advance(); // consume ::
                    let variant_name = self.consume_identifier("Expected variant name after '::'")?;
                    
                    // Check for fields
                    let fields = if self.check(&TokenKind::LeftParen) {
                        self.advance(); // consume (
                        let mut field_patterns = Vec::new();
                        if !self.check(&TokenKind::RightParen) {
                            loop {
                                field_patterns.push(self.parse_pattern()?);
                                if !self.match_token(&[TokenKind::Comma]) {
                                    break;
                                }
                            }
                        }
                        self.consume(&TokenKind::RightParen, "Expected ')' after variant fields")?;
                        Some(field_patterns)
                    } else {
                        None
                    };
                    
                    return Ok(Pattern::Variant {
                        name, // Enum name is the qualifier
                        variant: variant_name,
                        fields,
                    });
                }
                
                // Check for variant pattern: Name(pattern) or Name::Variant(pattern)
                if self.check(&TokenKind::LeftParen) {
                    // Variant with fields: Some(x) or Ok(value)
                    self.advance(); // consume (
                    let mut fields = Vec::new();
                    if !self.check(&TokenKind::RightParen) {
                        loop {
                            fields.push(self.parse_pattern()?);
                            if !self.match_token(&[TokenKind::Comma]) {
                                break;
                            }
                        }
                    }
                    self.consume(&TokenKind::RightParen, "Expected ')' after variant fields")?;
                    return Ok(Pattern::Variant {
                        name: String::new(), // No enum name qualifier
                        variant: name,
                        fields: Some(fields),
                    });
                }
                
                // Check for struct pattern: Name { field: pattern, ... }
                if self.check(&TokenKind::LeftBrace) {
                    self.advance(); // consume {
                    let mut fields = Vec::new();
                    if !self.check(&TokenKind::RightBrace) {
                        loop {
                            let field_name = self.consume_identifier("Expected field name")?;
                            self.consume(&TokenKind::Colon, "Expected ':' after field name")?;
                            let field_pattern = self.parse_pattern()?;
                            fields.push((field_name, field_pattern));
                            if !self.match_token(&[TokenKind::Comma]) {
                                break;
                            }
                        }
                    }
                    self.consume(&TokenKind::RightBrace, "Expected '}' after struct pattern")?;
                    return Ok(Pattern::Struct { name, fields });
                }
                
                // Simple identifier - check if it looks like a unit variant (capitalized) or a variable binding
                if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                    // Capitalized - could be a unit variant like None
                    return Ok(Pattern::Variant {
                        name: String::new(),
                        variant: name,
                        fields: None,
                    });
                }
                
                // Variable binding pattern
                return Ok(Pattern::Variable(name));
            }
        }
        
        Err(IntentError::ParserError {
            line: self.current_line(),
            message: "Expected pattern".to_string(),
        })
    }

    fn parse_type(&mut self) -> Result<TypeExpr> {
        let first_type = self.parse_single_type()?;
        
        // Check for union type: T | U | V ...
        if self.check(&TokenKind::Pipe) {
            let mut types = vec![first_type];
            while self.match_token(&[TokenKind::Pipe]) {
                types.push(self.parse_single_type()?);
            }
            return Ok(TypeExpr::Union(types));
        }
        
        Ok(first_type)
    }
    
    fn parse_single_type(&mut self) -> Result<TypeExpr> {
        let name = self.consume_identifier("Expected type name")?;
        
        // Check for generic parameters
        if self.match_token(&[TokenKind::Less]) {
            let mut args = Vec::new();
            loop {
                args.push(self.parse_type()?);
                if !self.match_token(&[TokenKind::Comma]) {
                    break;
                }
            }
            self.consume(&TokenKind::Greater, "Expected '>' after type parameters")?;
            
            // Check for optional after generic
            if self.match_token(&[TokenKind::Question]) {
                return Ok(TypeExpr::Optional(Box::new(TypeExpr::Generic { name, args })));
            }
            return Ok(TypeExpr::Generic { name, args });
        }
        
        // Check for array type
        if name == "Array" || self.match_token(&[TokenKind::LeftBracket]) {
            if name != "Array" {
                self.consume(&TokenKind::RightBracket, "Expected ']' for array type")?;
            }
            // Simplified: just return named type for now
        }
        
        // Check for optional type
        if self.match_token(&[TokenKind::Question]) {
            return Ok(TypeExpr::Optional(Box::new(TypeExpr::Named(name))));
        }
        
        Ok(TypeExpr::Named(name))
    }

    fn consume_identifier(&mut self, message: &str) -> Result<String> {
        if let Some(token) = self.peek() {
            if let TokenKind::Identifier(ref name) = token.kind {
                let name = name.clone();
                self.advance();
                return Ok(name);
            }
        }
        Err(IntentError::ParserError {
            line: self.current_line(),
            message: message.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(source: &str) -> Result<Program> {
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = Parser::new(tokens);
        parser.parse()
    }

    #[test]
    fn test_let_statement() {
        let program = parse("let x = 42;").unwrap();
        assert_eq!(program.statements.len(), 1);
    }

    #[test]
    fn test_function() {
        let program = parse("fn add(a, b) { return a + b; }").unwrap();
        assert_eq!(program.statements.len(), 1);
    }

    #[test]
    fn test_arithmetic() {
        let program = parse("1 + 2 * 3").unwrap();
        assert_eq!(program.statements.len(), 1);
    }
}
