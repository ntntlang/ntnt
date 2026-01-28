//! Static type checker for Intent
//!
//! Performs type checking as a separate pass between parsing and interpretation.
//! Produces diagnostics (errors/warnings) without blocking execution.
//! Uses gradual typing: untyped code defaults to `Any`, which is compatible with everything.

use std::collections::HashMap;

use crate::ast::*;
use crate::types::Type;

/// Severity of a type diagnostic
#[derive(Debug, Clone, PartialEq)]
pub enum Severity {
    Error,
    Warning,
}

/// A diagnostic produced by the type checker
#[derive(Debug, Clone)]
pub struct TypeDiagnostic {
    pub severity: Severity,
    pub message: String,
    pub line: usize,
    pub column: usize,
    pub hint: Option<String>,
}

/// Signature of a function (builtin, stdlib, or user-defined)
#[derive(Debug, Clone)]
pub struct FunctionSig {
    pub params: Vec<(String, Type)>,
    pub return_type: Type,
    pub variadic: bool,
}

/// Type checking context with scoped variable bindings
pub struct TypeContext {
    /// Stack of variable scopes (innermost last)
    scopes: Vec<HashMap<String, Type>>,
    /// User-defined function signatures
    functions: HashMap<String, FunctionSig>,
    /// Struct field types
    structs: HashMap<String, Vec<(String, Type)>>,
    /// Enum variants: enum_name -> [(variant_name, Option<field_types>)]
    enums: HashMap<String, Vec<(String, Option<Vec<Type>>)>>,
    /// Type aliases
    type_aliases: HashMap<String, Type>,
    /// Builtin and stdlib function signatures
    builtin_sigs: HashMap<String, FunctionSig>,
    /// Return type of current function being checked
    current_return_type: Option<Type>,
    /// Collected diagnostics
    diagnostics: Vec<TypeDiagnostic>,
    /// Source lines for line number lookup
    source_lines: Vec<String>,
    /// When true, warn about untyped function parameters and missing return types
    strict_lint: bool,
    /// File path of the current file being checked (for resolving relative imports)
    current_file: Option<String>,
    /// Cache of already-parsed module signatures (to avoid re-parsing)
    module_cache: HashMap<String, HashMap<String, FunctionSig>>,
    /// Set of files currently being resolved (for circular import detection)
    resolving_files: Vec<String>,
}

/// Returns true if NTNT_STRICT mode is enabled
pub fn is_strict_mode() -> bool {
    std::env::var("NTNT_STRICT").map_or(false, |v| v == "1" || v == "true")
}

/// Run the type checker in strict mode. Returns `Some(errors)` if strict mode is
/// enabled and type errors were found, `None` otherwise (either not strict, or no errors).
pub fn strict_check(ast: &Program, source: &str) -> Option<Vec<TypeDiagnostic>> {
    strict_check_with_file(ast, source, None)
}

/// Strict check with file path for cross-file import resolution
pub fn strict_check_with_file(
    ast: &Program,
    source: &str,
    file_path: Option<&str>,
) -> Option<Vec<TypeDiagnostic>> {
    if !is_strict_mode() {
        return None;
    }
    let errors: Vec<_> = check_program_with_options(ast, source, false, file_path)
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Entry point: check a parsed program and return diagnostics
pub fn check_program(ast: &Program, source: &str) -> Vec<TypeDiagnostic> {
    check_program_with_options(ast, source, false, None)
}

/// Entry point with strict lint mode: also warns about untyped function signatures
pub fn check_program_strict(ast: &Program, source: &str) -> Vec<TypeDiagnostic> {
    check_program_with_options(ast, source, true, None)
}

/// Entry point with file path for cross-file import resolution
pub fn check_program_with_file(
    ast: &Program,
    source: &str,
    file_path: &str,
) -> Vec<TypeDiagnostic> {
    check_program_with_options(ast, source, false, Some(file_path))
}

/// Entry point with file path and strict lint mode
pub fn check_program_strict_with_file(
    ast: &Program,
    source: &str,
    file_path: &str,
) -> Vec<TypeDiagnostic> {
    check_program_with_options(ast, source, true, Some(file_path))
}

fn check_program_with_options(
    ast: &Program,
    source: &str,
    strict_lint: bool,
    file_path: Option<&str>,
) -> Vec<TypeDiagnostic> {
    let mut ctx = TypeContext::new(source);
    ctx.strict_lint = strict_lint;
    ctx.current_file = file_path.map(|s| s.to_string());
    ctx.register_builtins();

    // Pass 1: collect top-level declarations (enables forward references)
    for stmt in &ast.statements {
        ctx.collect_declaration(stmt);
    }

    // Pass 2: type-check all statements
    for stmt in &ast.statements {
        ctx.check_statement(stmt);
    }

    ctx.diagnostics
}

impl TypeContext {
    fn new(source: &str) -> Self {
        TypeContext {
            scopes: vec![HashMap::new()],
            functions: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            type_aliases: HashMap::new(),
            builtin_sigs: HashMap::new(),
            current_return_type: None,
            diagnostics: Vec::new(),
            source_lines: source.lines().map(|l| l.to_string()).collect(),
            strict_lint: false,
            current_file: None,
            module_cache: HashMap::new(),
            resolving_files: Vec::new(),
        }
    }

    // ── Scope management ──────────────────────────────────────────────

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn bind(&mut self, name: &str, typ: Type) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), typ);
        }
    }

    fn lookup(&self, name: &str) -> Option<&Type> {
        for scope in self.scopes.iter().rev() {
            if let Some(typ) = scope.get(name) {
                return Some(typ);
            }
        }
        None
    }

    // ── Diagnostics ───────────────────────────────────────────────────

    fn emit(&mut self, severity: Severity, message: String, line: usize, hint: Option<String>) {
        self.diagnostics.push(TypeDiagnostic {
            severity,
            message,
            line,
            column: 0,
            hint,
        });
    }

    fn error(&mut self, message: String, line: usize, hint: Option<String>) {
        self.emit(Severity::Error, message, line, hint);
    }

    fn warning(&mut self, message: String, line: usize, hint: Option<String>) {
        self.emit(Severity::Warning, message, line, hint);
    }

    // ── Line number lookup ────────────────────────────────────────────

    fn find_line(&self, needle: &str) -> usize {
        for (i, line) in self.source_lines.iter().enumerate() {
            if line.contains(needle) {
                return i + 1; // 1-indexed
            }
        }
        0
    }

    #[allow(dead_code)]
    fn find_line_after(&self, needle: &str, after: usize) -> usize {
        for (i, line) in self.source_lines.iter().enumerate() {
            if i + 1 > after && line.contains(needle) {
                return i + 1;
            }
        }
        // Fall back to searching from beginning
        self.find_line(needle)
    }

    // ── Type resolution ───────────────────────────────────────────────

    /// Convert AST TypeExpr to internal Type
    fn resolve_type_expr(&self, te: &TypeExpr) -> Type {
        match te {
            TypeExpr::Named(name) => match name.as_str() {
                "Int" => Type::Int,
                "Float" => Type::Float,
                "String" => Type::String,
                "Bool" => Type::Bool,
                "Unit" | "()" => Type::Unit,
                "Any" => Type::Any,
                "Never" => Type::Never,
                "Array" => Type::Array(Box::new(Type::Any)),
                _ => {
                    // Check type aliases
                    if let Some(resolved) = self.type_aliases.get(name) {
                        return resolved.clone();
                    }
                    // Check structs/enums
                    if self.structs.contains_key(name) || self.enums.contains_key(name) {
                        return Type::Named(name.clone());
                    }
                    // Treat unresolved type names as Any (likely type parameters like T, U)
                    Type::Any
                }
            },
            TypeExpr::Array(inner) => Type::Array(Box::new(self.resolve_type_expr(inner))),
            TypeExpr::Map {
                key_type,
                value_type,
            } => Type::Map {
                key_type: Box::new(self.resolve_type_expr(key_type)),
                value_type: Box::new(self.resolve_type_expr(value_type)),
            },
            TypeExpr::Tuple(types) => {
                Type::Tuple(types.iter().map(|t| self.resolve_type_expr(t)).collect())
            }
            TypeExpr::Function {
                params,
                return_type,
            } => Type::Function {
                params: params.iter().map(|t| self.resolve_type_expr(t)).collect(),
                return_type: Box::new(self.resolve_type_expr(return_type)),
            },
            TypeExpr::Generic { name, args } => {
                let resolved_args: Vec<Type> =
                    args.iter().map(|t| self.resolve_type_expr(t)).collect();
                if name == "Array" && resolved_args.len() == 1 {
                    Type::Array(Box::new(resolved_args[0].clone()))
                } else if name == "Option" && resolved_args.len() == 1 {
                    Type::Optional(Box::new(resolved_args[0].clone()))
                } else if name == "Map" && resolved_args.len() == 2 {
                    Type::Map {
                        key_type: Box::new(resolved_args[0].clone()),
                        value_type: Box::new(resolved_args[1].clone()),
                    }
                } else {
                    Type::Generic {
                        name: name.clone(),
                        args: resolved_args,
                    }
                }
            }
            TypeExpr::Optional(inner) => Type::Optional(Box::new(self.resolve_type_expr(inner))),
            TypeExpr::Union(types) => {
                Type::Union(types.iter().map(|t| self.resolve_type_expr(t)).collect())
            }
        }
    }

    /// Check if two types are compatible (delegates to Type::is_compatible)
    fn compatible(&self, actual: &Type, expected: &Type) -> bool {
        actual.is_compatible(expected)
    }

    /// Compute the union of two types
    fn union_type(&self, a: &Type, b: &Type) -> Type {
        if a.is_compatible(b) {
            // If compatible, prefer the more specific one
            if matches!(a, Type::Any) {
                b.clone()
            } else {
                a.clone()
            }
        } else {
            Type::Union(vec![a.clone(), b.clone()])
        }
    }

    /// Try to determine the return type of a callback argument.
    /// Checks: (1) Lambda expression type, (2) Named function lookup.
    fn resolve_callback_return_type(
        &self,
        expr: &Expression,
        inferred_type: &Type,
    ) -> Option<Type> {
        // Case 1: Lambda — inferred_type is Type::Function { return_type, .. }
        if let Type::Function { return_type, .. } = inferred_type {
            return Some((**return_type).clone());
        }
        // Case 2: Named function identifier
        if let Expression::Identifier(name) = expr {
            if let Some(sig) = self
                .functions
                .get(name)
                .or_else(|| self.builtin_sigs.get(name))
            {
                return Some(sig.return_type.clone());
            }
        }
        None
    }

    // ── Pass 1: Declaration collection ────────────────────────────────

    fn collect_declaration(&mut self, stmt: &Statement) {
        match stmt {
            Statement::Function {
                name,
                params,
                return_type,
                type_params: _,
                ..
            } => {
                let param_types: Vec<(String, Type)> = params
                    .iter()
                    .map(|p| {
                        let typ = p
                            .type_annotation
                            .as_ref()
                            .map(|t| self.resolve_type_expr(t))
                            .unwrap_or(Type::Any);
                        (p.name.clone(), typ)
                    })
                    .collect();

                let ret = return_type
                    .as_ref()
                    .map(|t| self.resolve_type_expr(t))
                    .unwrap_or(Type::Any);

                self.functions.insert(
                    name.clone(),
                    FunctionSig {
                        params: param_types,
                        return_type: ret,
                        variadic: false,
                    },
                );
            }
            Statement::Struct {
                name,
                fields,
                type_params: _,
                ..
            } => {
                let field_types: Vec<(String, Type)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), self.resolve_type_expr(&f.type_annotation)))
                    .collect();
                self.structs.insert(name.clone(), field_types);
            }
            Statement::Enum {
                name,
                variants,
                type_params: _,
                ..
            } => {
                let variant_types: Vec<(String, Option<Vec<Type>>)> = variants
                    .iter()
                    .map(|v| {
                        let fields = v
                            .fields
                            .as_ref()
                            .map(|fs| fs.iter().map(|t| self.resolve_type_expr(t)).collect());
                        (v.name.clone(), fields)
                    })
                    .collect();
                self.enums.insert(name.clone(), variant_types);
            }
            Statement::TypeAlias {
                name,
                target,
                type_params: _,
            } => {
                let resolved = self.resolve_type_expr(target);
                self.type_aliases.insert(name.clone(), resolved);
            }
            Statement::Impl { methods, .. } => {
                for method in methods {
                    self.collect_declaration(method);
                }
            }
            _ => {}
        }
    }

    // ── Pass 2: Statement checking ────────────────────────────────────

    fn check_statement(&mut self, stmt: &Statement) {
        match stmt {
            Statement::Let {
                name,
                type_annotation,
                value,
                pattern,
                ..
            } => {
                let inferred = value
                    .as_ref()
                    .map(|v| self.infer_expression(v))
                    .unwrap_or(Type::Any);

                if let Some(ann) = type_annotation {
                    let expected = self.resolve_type_expr(ann);
                    if !self.compatible(&inferred, &expected) {
                        let line = self.find_line(&format!("let {}", name));
                        self.error(
                            format!(
                                "Type mismatch: variable '{}' declared as {} but initialized with {}",
                                name,
                                expected.name(),
                                inferred.name()
                            ),
                            line,
                            Some(format!("Expected {}", expected.name())),
                        );
                    }
                    self.bind(name, expected);
                } else if let Some(Pattern::Tuple(patterns)) = pattern {
                    // Destructuring: bind each pattern variable
                    for p in patterns {
                        if let Pattern::Variable(var_name) = p {
                            self.bind(var_name, Type::Any);
                        }
                    }
                } else {
                    self.bind(name, inferred);
                }
            }

            Statement::Function {
                name,
                params,
                return_type,
                contract,
                body,
                type_params: _,
                ..
            } => {
                self.push_scope();

                // Strict lint: warn about untyped parameters and missing return type
                if self.strict_lint {
                    let fn_line = self.find_line(&format!("fn {}", name));
                    for param in params {
                        if param.type_annotation.is_none() {
                            self.warning(
                                format!(
                                    "Parameter '{}' in function '{}' has no type annotation",
                                    param.name, name
                                ),
                                fn_line,
                                Some(format!("Add a type: {}: Type", param.name)),
                            );
                        }
                    }
                    if return_type.is_none() {
                        self.warning(
                            format!("Function '{}' has no return type annotation", name),
                            fn_line,
                            Some(format!("Add a return type: fn {}(...) -> Type", name)),
                        );
                    }
                }

                // Bind parameters
                for param in params {
                    let typ = param
                        .type_annotation
                        .as_ref()
                        .map(|t| self.resolve_type_expr(t))
                        .unwrap_or(Type::Any);
                    self.bind(&param.name, typ);
                }

                // Set expected return type
                let prev_return = self.current_return_type.take();
                let resolved_return = return_type.as_ref().map(|t| self.resolve_type_expr(t));
                self.current_return_type = resolved_return.clone();

                // Type-check contract expressions (requires/ensures)
                if let Some(contract) = contract {
                    let fn_line = self.find_line(&format!("fn {}", name));

                    // requires: check each expression evaluates to Bool
                    for req_expr in &contract.requires {
                        let req_type = self.infer_expression(req_expr);
                        if !self.compatible(&req_type, &Type::Bool)
                            && !matches!(req_type, Type::Any)
                        {
                            let line = self.find_line_after("requires", fn_line.saturating_sub(1));
                            self.error(
                                format!(
                                    "Contract 'requires' in '{}' should be Bool, got {}",
                                    name,
                                    req_type.name()
                                ),
                                if line > 0 { line } else { fn_line },
                                Some("requires clauses must evaluate to Bool".to_string()),
                            );
                        }
                    }

                    // ensures: bind `result` to return type, then check each expression
                    if !contract.ensures.is_empty() {
                        let result_type = resolved_return.clone().unwrap_or(Type::Any);
                        self.bind("result", result_type);

                        for ens_expr in &contract.ensures {
                            let ens_type = self.infer_expression(ens_expr);
                            if !self.compatible(&ens_type, &Type::Bool)
                                && !matches!(ens_type, Type::Any)
                            {
                                let line =
                                    self.find_line_after("ensures", fn_line.saturating_sub(1));
                                self.error(
                                    format!(
                                        "Contract 'ensures' in '{}' should be Bool, got {}",
                                        name,
                                        ens_type.name()
                                    ),
                                    if line > 0 { line } else { fn_line },
                                    Some("ensures clauses must evaluate to Bool".to_string()),
                                );
                            }
                        }
                    }
                }

                // Check body
                let body_type = self.check_block(body);

                // Verify return type if annotated
                if let Some(expected_ret) = &self.current_return_type {
                    if !self.compatible(&body_type, expected_ret) && !matches!(body_type, Type::Any)
                    {
                        let line = self.find_line(&format!("fn {}", name));
                        self.error(
                            format!(
                                "Return type mismatch in '{}': expected {} but body returns {}",
                                name,
                                expected_ret.name(),
                                body_type.name()
                            ),
                            line,
                            Some(format!("Expected return type {}", expected_ret.name())),
                        );
                    }
                }

                self.current_return_type = prev_return;
                self.pop_scope();
            }

            Statement::Return(expr) => {
                if let Some(expr) = expr {
                    let actual = self.infer_expression(expr);
                    if let Some(expected) = &self.current_return_type {
                        if !self.compatible(&actual, expected) && !matches!(actual, Type::Any) {
                            let line = self.find_line("return");
                            self.error(
                                format!(
                                    "Return type mismatch: expected {} but returning {}",
                                    expected.name(),
                                    actual.name()
                                ),
                                line,
                                None,
                            );
                        }
                    }
                }
            }

            Statement::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_type = self.infer_expression(condition);
                if !self.compatible(&cond_type, &Type::Bool) && !matches!(cond_type, Type::Any) {
                    let line = self.find_line("if ");
                    self.warning(
                        format!("Condition has type {} instead of Bool", cond_type.name()),
                        line,
                        None,
                    );
                }
                self.push_scope();
                self.check_block(then_branch);
                self.pop_scope();
                if let Some(else_b) = else_branch {
                    self.push_scope();
                    self.check_block(else_b);
                    self.pop_scope();
                }
            }

            Statement::While { condition, body } => {
                let cond_type = self.infer_expression(condition);
                if !self.compatible(&cond_type, &Type::Bool) && !matches!(cond_type, Type::Any) {
                    let line = self.find_line("while ");
                    self.warning(
                        format!(
                            "While condition has type {} instead of Bool",
                            cond_type.name()
                        ),
                        line,
                        None,
                    );
                }
                self.push_scope();
                self.check_block(body);
                self.pop_scope();
            }

            Statement::ForIn {
                variable,
                iterable,
                body,
            } => {
                let iter_type = self.infer_expression(iterable);
                let elem_type = match &iter_type {
                    Type::Array(inner) => (**inner).clone(),
                    Type::String => Type::String,
                    Type::Map { key_type, .. } => (**key_type).clone(),
                    _ => Type::Any,
                };
                self.push_scope();
                self.bind(variable, elem_type);
                self.check_block(body);
                self.pop_scope();
            }

            Statement::Loop { body } => {
                self.push_scope();
                self.check_block(body);
                self.pop_scope();
            }

            Statement::Expression(expr) => {
                self.infer_expression(expr);
            }

            Statement::Import {
                items,
                source,
                alias,
            } => {
                self.register_import(items, source, alias.as_deref());
            }

            // Already handled in Pass 1
            Statement::Struct { .. }
            | Statement::Enum { .. }
            | Statement::TypeAlias { .. }
            | Statement::Trait { .. } => {}

            Statement::Impl {
                type_name,
                methods,
                invariants,
                ..
            } => {
                // Type-check invariant expressions
                if !invariants.is_empty() {
                    self.push_scope();

                    // Bind struct fields so invariant expressions can reference them
                    if let Some(fields) = self.structs.get(type_name).cloned() {
                        for (field_name, field_type) in &fields {
                            self.bind(field_name, field_type.clone());
                        }
                    }

                    for inv_expr in invariants {
                        let inv_type = self.infer_expression(inv_expr);
                        if !self.compatible(&inv_type, &Type::Bool)
                            && !matches!(inv_type, Type::Any)
                        {
                            let line = self.find_line("invariant");
                            self.error(
                                format!(
                                    "Invariant in '{}' should be Bool, got {}",
                                    type_name,
                                    inv_type.name()
                                ),
                                line,
                                Some("invariant clauses must evaluate to Bool".to_string()),
                            );
                        }
                    }

                    self.pop_scope();
                }

                for method in methods {
                    self.check_statement(method);
                }
            }

            // Statements that don't need type checking
            Statement::Break
            | Statement::Continue
            | Statement::Use { .. }
            | Statement::Export { .. }
            | Statement::Module { .. }
            | Statement::Protocol { .. }
            | Statement::Intent { .. }
            | Statement::Defer(_) => {}
        }
    }

    /// Check a block and return the type of the last expression
    fn check_block(&mut self, block: &Block) -> Type {
        let mut last_type = Type::Unit;
        for stmt in &block.statements {
            self.check_statement(stmt);
            // Track the type of expression statements (for implicit return)
            if let Statement::Expression(expr) = stmt {
                last_type = self.infer_expression(expr);
            } else if let Statement::Return(Some(expr)) = stmt {
                last_type = self.infer_expression(expr);
            } else {
                last_type = Type::Unit;
            }
        }
        last_type
    }

    // ── Expression type inference ─────────────────────────────────────

    fn infer_expression(&mut self, expr: &Expression) -> Type {
        match expr {
            Expression::Integer(_) => Type::Int,
            Expression::Float(_) => Type::Float,
            Expression::String(_) => Type::String,
            Expression::Bool(_) => Type::Bool,
            Expression::Unit => Type::Unit,

            Expression::Identifier(name) => {
                // Check special names
                match name.as_str() {
                    "None" => return Type::Optional(Box::new(Type::Any)),
                    "true" | "false" => return Type::Bool,
                    _ => {}
                }

                if let Some(typ) = self.lookup(name) {
                    typ.clone()
                } else if self.functions.contains_key(name) || self.builtin_sigs.contains_key(name)
                {
                    // It's a function name used as a value
                    Type::Any
                } else {
                    // Don't emit error here — interpreter handles undefined vars
                    Type::Any
                }
            }

            Expression::Binary {
                left,
                operator,
                right,
            } => {
                let left_type = self.infer_expression(left);
                let right_type = self.infer_expression(right);
                self.infer_binary_op(operator, &left_type, &right_type)
            }

            Expression::Unary { operator, operand } => {
                let operand_type = self.infer_expression(operand);
                match operator {
                    UnaryOp::Neg => match &operand_type {
                        Type::Int => Type::Int,
                        Type::Float => Type::Float,
                        _ => Type::Any,
                    },
                    UnaryOp::Not => Type::Bool,
                }
            }

            Expression::Call {
                function,
                arguments,
            } => self.infer_call(function, arguments),

            Expression::MethodCall {
                object,
                method,
                arguments,
            } => {
                let obj_type = self.infer_expression(object);
                let method_arg_types: Vec<Type> =
                    arguments.iter().map(|a| self.infer_expression(a)).collect();
                // Method calls: infer return type from known methods
                match method.as_str() {
                    "unwrap" | "unwrap_or" => match &obj_type {
                        Type::Optional(inner) => (**inner).clone(),
                        Type::Generic { name, args } if name == "Result" && !args.is_empty() => {
                            args[0].clone()
                        }
                        _ => Type::Any,
                    },
                    "is_some" | "is_none" | "is_ok" | "is_err" => Type::Bool,
                    "filter" | "sort" | "reverse" | "slice" | "concat" => match &obj_type {
                        Type::Array(_) => obj_type.clone(),
                        _ => Type::Any,
                    },
                    "flatten" => match &obj_type {
                        Type::Array(inner) if matches!(inner.as_ref(), Type::Array(_)) => {
                            (**inner).clone()
                        }
                        Type::Array(_) => obj_type.clone(),
                        _ => Type::Any,
                    },
                    "push" => match &obj_type {
                        Type::Array(_) => obj_type.clone(),
                        _ => Type::Any,
                    },
                    "first" | "last" | "pop" => match &obj_type {
                        Type::Array(inner) => (**inner).clone(),
                        _ => Type::Any,
                    },
                    "map" | "transform" => {
                        if let Some((arg_expr, arg_type)) =
                            arguments.first().zip(method_arg_types.first())
                        {
                            if let Some(ret) = self.resolve_callback_return_type(arg_expr, arg_type)
                            {
                                Type::Array(Box::new(ret))
                            } else {
                                Type::Array(Box::new(Type::Any))
                            }
                        } else {
                            Type::Array(Box::new(Type::Any))
                        }
                    }
                    "len" | "length" => Type::Int,
                    "to_string" | "to_str" => Type::String,
                    "abs" => match &obj_type {
                        Type::Int | Type::Float => obj_type.clone(),
                        _ => Type::Any,
                    },
                    "min" | "max" => match &obj_type {
                        Type::Int | Type::Float => obj_type.clone(),
                        _ => Type::Any,
                    },
                    "clamp" => match &obj_type {
                        Type::Int | Type::Float => obj_type.clone(),
                        _ => Type::Any,
                    },
                    "keys" => match &obj_type {
                        Type::Map { key_type, .. } => Type::Array(key_type.clone()),
                        _ => Type::Array(Box::new(Type::String)),
                    },
                    "values" => match &obj_type {
                        Type::Map { value_type, .. } => Type::Array(value_type.clone()),
                        _ => Type::Any,
                    },
                    "entries" => match &obj_type {
                        Type::Map { .. } => Type::Array(Box::new(Type::Array(Box::new(Type::Any)))),
                        _ => Type::Any,
                    },
                    "get_key" | "get" => match &obj_type {
                        Type::Map { value_type, .. } => (**value_type).clone(),
                        _ => Type::Any,
                    },
                    _ => Type::Any,
                }
            }

            Expression::FieldAccess { object, field } => {
                let obj_type = self.infer_expression(object);
                match &obj_type {
                    Type::Named(name) => {
                        if let Some(fields) = self.structs.get(name) {
                            for (fname, ftype) in fields {
                                if fname == field {
                                    return ftype.clone();
                                }
                            }
                        }
                        Type::Any
                    }
                    _ => Type::Any,
                }
            }

            Expression::Index { object, index } => {
                let obj_type = self.infer_expression(object);
                let _idx_type = self.infer_expression(index);
                match &obj_type {
                    Type::Array(inner) => (**inner).clone(),
                    Type::Map { value_type, .. } => (**value_type).clone(),
                    Type::String => Type::String,
                    _ => Type::Any,
                }
            }

            Expression::Array(elements) => {
                if elements.is_empty() {
                    return Type::Array(Box::new(Type::Any));
                }
                let mut elem_type = self.infer_expression(&elements[0]);
                for elem in &elements[1..] {
                    let t = self.infer_expression(elem);
                    elem_type = self.union_type(&elem_type, &t);
                }
                Type::Array(Box::new(elem_type))
            }

            Expression::MapLiteral(pairs) => {
                if pairs.is_empty() {
                    return Type::Map {
                        key_type: Box::new(Type::Any),
                        value_type: Box::new(Type::Any),
                    };
                }
                let mut key_type = self.infer_expression(&pairs[0].0);
                let mut val_type = self.infer_expression(&pairs[0].1);
                for (k, v) in &pairs[1..] {
                    let kt = self.infer_expression(k);
                    let vt = self.infer_expression(v);
                    key_type = self.union_type(&key_type, &kt);
                    val_type = self.union_type(&val_type, &vt);
                }
                Type::Map {
                    key_type: Box::new(key_type),
                    value_type: Box::new(val_type),
                }
            }

            Expression::Range { .. } => {
                // Range produces an iterable of Int
                Type::Array(Box::new(Type::Int))
            }

            Expression::InterpolatedString(_) => Type::String,
            Expression::TemplateString(_) => Type::String,

            Expression::StructLiteral { name, fields } => {
                // Check field types match struct definition
                if let Some(struct_fields) = self.structs.get(name).cloned() {
                    for (fname, fexpr) in fields {
                        let actual = self.infer_expression(fexpr);
                        if let Some((_, expected)) = struct_fields.iter().find(|(n, _)| n == fname)
                        {
                            if !self.compatible(&actual, expected) && !matches!(actual, Type::Any) {
                                let line = self.find_line(name);
                                self.error(
                                    format!(
                                        "Field '{}' of struct '{}': expected {} but got {}",
                                        fname,
                                        name,
                                        expected.name(),
                                        actual.name()
                                    ),
                                    line,
                                    None,
                                );
                            }
                        }
                    }
                }
                Type::Named(name.clone())
            }

            Expression::EnumVariant {
                enum_name,
                variant,
                arguments,
            } => {
                // Check variant argument types
                if let Some(variants) = self.enums.get(enum_name).cloned() {
                    if let Some((_, expected_fields)) = variants.iter().find(|(v, _)| v == variant)
                    {
                        if let Some(expected) = expected_fields {
                            if arguments.len() != expected.len() {
                                let line = self.find_line(&format!("{}::{}", enum_name, variant));
                                self.error(
                                    format!(
                                        "Enum variant {}::{} expects {} argument(s), got {}",
                                        enum_name,
                                        variant,
                                        expected.len(),
                                        arguments.len()
                                    ),
                                    line,
                                    None,
                                );
                            } else {
                                for (arg, exp_type) in arguments.iter().zip(expected.iter()) {
                                    let actual = self.infer_expression(arg);
                                    if !self.compatible(&actual, exp_type)
                                        && !matches!(actual, Type::Any)
                                    {
                                        let line =
                                            self.find_line(&format!("{}::{}", enum_name, variant));
                                        self.error(
                                            format!(
                                                "Enum variant {}::{}: expected {} but got {}",
                                                enum_name,
                                                variant,
                                                exp_type.name(),
                                                actual.name()
                                            ),
                                            line,
                                            None,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                // Special handling for Option/Result constructors
                match (enum_name.as_str(), variant.as_str()) {
                    ("Option", "Some") | (_, "Some") => {
                        if let Some(first) = arguments.first() {
                            let inner = self.infer_expression(first);
                            Type::Optional(Box::new(inner))
                        } else {
                            Type::Optional(Box::new(Type::Any))
                        }
                    }
                    ("Option", "None") | (_, "None") => Type::Optional(Box::new(Type::Any)),
                    ("Result", "Ok") | (_, "Ok") => {
                        if let Some(first) = arguments.first() {
                            let inner = self.infer_expression(first);
                            Type::Generic {
                                name: "Result".to_string(),
                                args: vec![inner, Type::Any],
                            }
                        } else {
                            Type::Generic {
                                name: "Result".to_string(),
                                args: vec![Type::Any, Type::Any],
                            }
                        }
                    }
                    ("Result", "Err") | (_, "Err") => {
                        if let Some(first) = arguments.first() {
                            let inner = self.infer_expression(first);
                            Type::Generic {
                                name: "Result".to_string(),
                                args: vec![Type::Any, inner],
                            }
                        } else {
                            Type::Generic {
                                name: "Result".to_string(),
                                args: vec![Type::Any, Type::Any],
                            }
                        }
                    }
                    _ => Type::Named(enum_name.clone()),
                }
            }

            Expression::Lambda { params, body } => {
                self.push_scope();
                let param_types: Vec<Type> = params
                    .iter()
                    .map(|p| {
                        let typ = p
                            .type_annotation
                            .as_ref()
                            .map(|t| self.resolve_type_expr(t))
                            .unwrap_or(Type::Any);
                        self.bind(&p.name, typ.clone());
                        typ
                    })
                    .collect();
                let ret = self.infer_expression(body);
                self.pop_scope();
                Type::Function {
                    params: param_types,
                    return_type: Box::new(ret),
                }
            }

            Expression::Block(block) => {
                self.push_scope();
                let typ = self.check_block(block);
                self.pop_scope();
                typ
            }

            Expression::IfExpr {
                condition,
                then_branch,
                else_branch,
            } => {
                self.infer_expression(condition);
                let then_type = self.infer_expression(then_branch);
                let else_type = self.infer_expression(else_branch);
                self.union_type(&then_type, &else_type)
            }

            Expression::Match { scrutinee, arms } => {
                let scrutinee_type = self.infer_expression(scrutinee);
                let mut result_type: Option<Type> = None;

                for arm in arms {
                    self.push_scope();
                    self.bind_pattern(&arm.pattern, &scrutinee_type);
                    if let Some(guard) = &arm.guard {
                        self.infer_expression(guard);
                    }
                    let arm_type = self.infer_expression(&arm.body);
                    self.pop_scope();

                    result_type = Some(match result_type {
                        Some(prev) => self.union_type(&prev, &arm_type),
                        None => arm_type,
                    });
                }

                result_type.unwrap_or(Type::Any)
            }

            Expression::Assign { target, value } => {
                let _target_type = self.infer_expression(target);
                self.infer_expression(value);
                Type::Unit
            }

            Expression::Await(inner) => self.infer_expression(inner),
            Expression::Try(inner) => self.infer_expression(inner),
        }
    }

    /// Infer the result type of a binary operation
    fn infer_binary_op(&self, op: &BinaryOp, left: &Type, right: &Type) -> Type {
        match op {
            // Arithmetic operators
            BinaryOp::Add => match (left, right) {
                (Type::Int, Type::Int) => Type::Int,
                (Type::Float, _) | (_, Type::Float) => Type::Float,
                (Type::String, _) | (_, Type::String) => Type::String,
                (Type::Any, _) | (_, Type::Any) => Type::Any,
                _ => Type::Any,
            },
            BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Pow => {
                match (left, right) {
                    (Type::Int, Type::Int) => Type::Int,
                    (Type::Float, _) | (_, Type::Float) => Type::Float,
                    (Type::Any, _) | (_, Type::Any) => Type::Any,
                    _ => Type::Any,
                }
            }

            // Comparison operators
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => Type::Bool,

            // Logical operators
            BinaryOp::And | BinaryOp::Or => Type::Bool,

            // Null coalescing
            BinaryOp::NullCoalesce => {
                // a ?? b: if a is Optional(T), result is T, else Any
                match left {
                    Type::Optional(inner) => (**inner).clone(),
                    _ => right.clone(),
                }
            }
        }
    }

    /// Infer the return type of a function call
    fn infer_call(&mut self, function: &Expression, arguments: &[Expression]) -> Type {
        // Infer argument types
        let arg_types: Vec<Type> = arguments.iter().map(|a| self.infer_expression(a)).collect();

        // Get function name for lookup
        let fn_name = match function {
            Expression::Identifier(name) => Some(name.clone()),
            _ => None,
        };

        if let Some(name) = &fn_name {
            // Special built-in constructors and contract functions
            match name.as_str() {
                // old(expr) in ensures clauses — returns the same type as expr
                "old" if arguments.len() == 1 => {
                    return arg_types[0].clone();
                }
                // unwrap(Optional<T>) -> T, unwrap(Result<T, E>) -> T
                "unwrap" if arguments.len() == 1 => {
                    return match &arg_types[0] {
                        Type::Optional(inner) => (**inner).clone(),
                        Type::Generic { name, args } if name == "Result" && !args.is_empty() => {
                            args[0].clone()
                        }
                        _ => Type::Any,
                    };
                }
                // filter(Array<T>, pred) -> Array<T>
                "filter" if arguments.len() == 2 => {
                    if let Type::Array(_) = &arg_types[0] {
                        return arg_types[0].clone();
                    }
                }
                // Collection functions that preserve Array<T> element type
                "sort" | "reverse" if arguments.len() == 1 => {
                    if let Type::Array(_) = &arg_types[0] {
                        return arg_types[0].clone();
                    }
                }
                // flatten(Array<Array<T>>) -> Array<T> (unwraps one nesting level)
                "flatten" if arguments.len() == 1 => {
                    if let Type::Array(inner) = &arg_types[0] {
                        if let Type::Array(_) = inner.as_ref() {
                            return (**inner).clone();
                        }
                        return arg_types[0].clone();
                    }
                }
                "slice" if !arguments.is_empty() => {
                    if let Type::Array(_) = &arg_types[0] {
                        return arg_types[0].clone();
                    }
                }
                "concat" if arguments.len() == 2 => {
                    if let Type::Array(_) = &arg_types[0] {
                        return arg_types[0].clone();
                    }
                }
                // push(Array<T>, T) -> Array<T>
                "push" if arguments.len() == 2 => {
                    if let Type::Array(_) = &arg_types[0] {
                        return arg_types[0].clone();
                    }
                }
                // first(Array<T>) -> T, last(Array<T>) -> T, pop(Array<T>) -> T
                "first" | "last" | "pop" if !arguments.is_empty() => {
                    if let Type::Array(inner) = &arg_types[0] {
                        return (**inner).clone();
                    }
                }
                // Math functions that preserve numeric type
                "abs" if arguments.len() == 1 => match &arg_types[0] {
                    Type::Int | Type::Float => return arg_types[0].clone(),
                    _ => {}
                },
                "min" | "max" if arguments.len() == 2 => match (&arg_types[0], &arg_types[1]) {
                    (Type::Int, Type::Int) => return Type::Int,
                    (Type::Float, _) | (_, Type::Float) => return Type::Float,
                    _ => {}
                },
                "clamp" if arguments.len() == 3 => match &arg_types[0] {
                    Type::Int | Type::Float => return arg_types[0].clone(),
                    _ => {}
                },
                "keys" if arguments.len() == 1 => {
                    if let Type::Map { key_type, .. } = &arg_types[0] {
                        return Type::Array(key_type.clone());
                    }
                }
                "values" if arguments.len() == 1 => {
                    if let Type::Map { value_type, .. } = &arg_types[0] {
                        return Type::Array(value_type.clone());
                    }
                }
                "entries" if arguments.len() == 1 => {
                    if let Type::Map { .. } = &arg_types[0] {
                        return Type::Array(Box::new(Type::Array(Box::new(Type::Any))));
                    }
                }
                "get_key" if arguments.len() >= 2 => {
                    if let Type::Map { value_type, .. } = &arg_types[0] {
                        return (**value_type).clone();
                    }
                }
                // transform(array, callback) -> Array<callback_return_type>
                "transform" if arguments.len() == 2 => {
                    if let Some(ret) =
                        self.resolve_callback_return_type(&arguments[1], &arg_types[1])
                    {
                        return Type::Array(Box::new(ret));
                    }
                    // Fall through to default Array<Any> from sig lookup
                }
                "Some" => {
                    return if let Some(first) = arg_types.first() {
                        Type::Optional(Box::new(first.clone()))
                    } else {
                        Type::Optional(Box::new(Type::Any))
                    };
                }
                "Ok" => {
                    return if let Some(first) = arg_types.first() {
                        Type::Generic {
                            name: "Result".to_string(),
                            args: vec![first.clone(), Type::Any],
                        }
                    } else {
                        Type::Generic {
                            name: "Result".to_string(),
                            args: vec![Type::Any, Type::Any],
                        }
                    };
                }
                "Err" => {
                    return if let Some(first) = arg_types.first() {
                        Type::Generic {
                            name: "Result".to_string(),
                            args: vec![Type::Any, first.clone()],
                        }
                    } else {
                        Type::Generic {
                            name: "Result".to_string(),
                            args: vec![Type::Any, Type::Any],
                        }
                    };
                }
                _ => {}
            }

            // Look up in user-defined functions, then builtins
            let sig = self
                .functions
                .get(name)
                .cloned()
                .or_else(|| self.builtin_sigs.get(name).cloned());

            if let Some(sig) = sig {
                // Check argument count
                if !sig.variadic && arg_types.len() != sig.params.len() {
                    let line = self.find_line(&format!("{}(", name));
                    self.error(
                        format!(
                            "Function '{}' expects {} argument(s), got {}",
                            name,
                            sig.params.len(),
                            arg_types.len()
                        ),
                        line,
                        None,
                    );
                    return sig.return_type;
                }

                // Check argument types (skip for variadic)
                if !sig.variadic {
                    for (i, (arg_type, (param_name, param_type))) in
                        arg_types.iter().zip(sig.params.iter()).enumerate()
                    {
                        if !self.compatible(arg_type, param_type)
                            && !matches!(arg_type, Type::Any)
                            && !matches!(param_type, Type::Any)
                        {
                            let line = self.find_line(&format!("{}(", name));
                            self.error(
                                format!(
                                    "Argument {} ('{}') of '{}': expected {} but got {}",
                                    i + 1,
                                    param_name,
                                    name,
                                    param_type.name(),
                                    arg_type.name()
                                ),
                                line,
                                Some(format!("Expected {}", param_type.name())),
                            );
                        }
                    }
                }

                return sig.return_type;
            }
        }

        // Unknown function or dynamic call
        Type::Any
    }

    /// Bind pattern variables with their inferred types
    fn bind_pattern(&mut self, pattern: &Pattern, scrutinee_type: &Type) {
        match pattern {
            Pattern::Variable(name) => {
                self.bind(name, scrutinee_type.clone());
            }
            Pattern::Wildcard => {}
            Pattern::Literal(_) => {}
            Pattern::Tuple(patterns) => {
                if let Type::Tuple(types) = scrutinee_type {
                    for (p, t) in patterns.iter().zip(types.iter()) {
                        self.bind_pattern(p, t);
                    }
                } else {
                    for p in patterns {
                        self.bind_pattern(p, &Type::Any);
                    }
                }
            }
            Pattern::Array(patterns) => {
                let elem_type = match scrutinee_type {
                    Type::Array(inner) => (**inner).clone(),
                    _ => Type::Any,
                };
                for p in patterns {
                    self.bind_pattern(p, &elem_type);
                }
            }
            Pattern::Struct { name, fields } => {
                // Look up struct definition for field types
                let struct_fields = self.structs.get(name).cloned().or_else(|| {
                    // Also try scrutinee type if it's a Named type
                    if let Type::Named(type_name) = scrutinee_type {
                        self.structs.get(type_name).cloned()
                    } else {
                        None
                    }
                });
                for (fname, p) in fields {
                    let field_type = struct_fields
                        .as_ref()
                        .and_then(|sf| sf.iter().find(|(n, _)| n == fname).map(|(_, t)| t.clone()))
                        .unwrap_or(Type::Any);
                    self.bind_pattern(p, &field_type);
                }
            }
            Pattern::Variant {
                variant, fields, ..
            } => {
                // Special handling for Option/Result patterns
                match variant.as_str() {
                    "Some" => {
                        let inner = match scrutinee_type {
                            Type::Optional(inner) => (**inner).clone(),
                            _ => Type::Any,
                        };
                        if let Some(fields) = fields {
                            for p in fields {
                                self.bind_pattern(p, &inner);
                            }
                        }
                    }
                    "Ok" => {
                        let inner = match scrutinee_type {
                            Type::Generic { name, args }
                                if name == "Result" && !args.is_empty() =>
                            {
                                args[0].clone()
                            }
                            _ => Type::Any,
                        };
                        if let Some(fields) = fields {
                            for p in fields {
                                self.bind_pattern(p, &inner);
                            }
                        }
                    }
                    "Err" => {
                        let inner = match scrutinee_type {
                            Type::Generic { name, args } if name == "Result" && args.len() > 1 => {
                                args[1].clone()
                            }
                            _ => Type::Any,
                        };
                        if let Some(fields) = fields {
                            for p in fields {
                                self.bind_pattern(p, &inner);
                            }
                        }
                    }
                    "None" => {}
                    _ => {
                        if let Some(fields) = fields {
                            for p in fields {
                                self.bind_pattern(p, &Type::Any);
                            }
                        }
                    }
                }
            }
        }
    }

    // ── Import resolution ─────────────────────────────────────────────

    /// Resolve an import source path to an absolute file path
    fn resolve_import_path(&self, source: &str) -> Option<std::path::PathBuf> {
        if source.starts_with("std/") {
            return None; // Standard library, not a file
        }

        if !source.starts_with("./") && !source.starts_with("../") {
            return None; // Not a relative import
        }

        let current = self.current_file.as_ref()?;
        let current_dir = std::path::Path::new(current)
            .parent()
            .unwrap_or(std::path::Path::new("."));

        let mut path = current_dir.join(source);
        if path.extension().is_none() {
            path = path.with_extension("tnt");
        }

        Some(path)
    }

    /// Parse a file and extract function signatures (Pass 1 only)
    fn extract_file_signatures(
        &mut self,
        file_path: &std::path::Path,
    ) -> HashMap<String, FunctionSig> {
        use crate::lexer::Lexer;
        use crate::parser::Parser;

        let path_str = file_path.to_string_lossy().to_string();

        // Check cache
        if let Some(cached) = self.module_cache.get(&path_str) {
            return cached.clone();
        }

        // Check for circular imports
        if self.resolving_files.contains(&path_str) {
            return HashMap::new();
        }

        // Read and parse
        let source_code = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(_) => return HashMap::new(),
        };

        let lexer = Lexer::new(&source_code);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = Parser::new(tokens);
        let ast = match parser.parse() {
            Ok(ast) => ast,
            Err(_) => return HashMap::new(),
        };

        // Mark as resolving (circular import protection)
        self.resolving_files.push(path_str.clone());

        // Create a temporary context for Pass 1 only
        let mut temp_ctx = TypeContext::new(&source_code);
        temp_ctx.current_file = Some(path_str.clone());
        temp_ctx.register_builtins();

        // Run Pass 1 on the imported file to collect declarations
        for stmt in &ast.statements {
            temp_ctx.collect_declaration(stmt);
        }

        // Extract the function signatures
        let sigs = temp_ctx.functions;

        // Cache and unmark
        self.module_cache.insert(path_str.clone(), sigs.clone());
        self.resolving_files.retain(|f| f != &path_str);

        sigs
    }

    fn register_import(&mut self, items: &[ImportItem], source: &str, alias: Option<&str>) {
        // If it's a module alias import, bind the module name
        if let Some(alias_name) = alias {
            self.bind(alias_name, Type::Any);
            return;
        }

        // Try standard library first
        let module_sigs = get_module_signatures(source);
        if !module_sigs.is_empty() {
            for item in items {
                let local_name = item.alias.as_ref().unwrap_or(&item.name);
                if let Some(sig) = module_sigs.get(&item.name) {
                    self.builtin_sigs.insert(local_name.clone(), sig.clone());
                } else {
                    self.bind(local_name, Type::Any);
                }
            }
            return;
        }

        // Try user file import
        if let Some(file_path) = self.resolve_import_path(source) {
            let file_sigs = self.extract_file_signatures(&file_path);
            for item in items {
                let local_name = item.alias.as_ref().unwrap_or(&item.name);
                if let Some(sig) = file_sigs.get(&item.name) {
                    self.builtin_sigs.insert(local_name.clone(), sig.clone());
                } else {
                    // Function not found in the imported file
                    self.bind(local_name, Type::Any);
                }
            }
            return;
        }

        // Unknown module — bind all as Any
        for item in items {
            let local_name = item.alias.as_ref().unwrap_or(&item.name);
            self.bind(local_name, Type::Any);
        }
    }

    // ── Builtin registration ──────────────────────────────────────────

    fn register_builtins(&mut self) {
        let b = &mut self.builtin_sigs;

        // Helper macro for concise registration
        macro_rules! sig {
            ($name:expr, [$($pname:expr => $ptype:expr),*], $ret:expr) => {
                b.insert($name.to_string(), FunctionSig {
                    params: vec![$(($pname.to_string(), $ptype)),*],
                    return_type: $ret,
                    variadic: false,
                });
            };
            ($name:expr, [$($pname:expr => $ptype:expr),*], $ret:expr, variadic) => {
                b.insert($name.to_string(), FunctionSig {
                    params: vec![$(($pname.to_string(), $ptype)),*],
                    return_type: $ret,
                    variadic: true,
                });
            };
        }

        // I/O
        sig!("print", ["value" => Type::Any], Type::Unit, variadic);
        sig!("input", ["prompt" => Type::String], Type::String);

        // Conversion
        sig!("str", ["value" => Type::Any], Type::String);
        sig!("int", ["value" => Type::Any], Type::Int);
        sig!("float", ["value" => Type::Any], Type::Float);
        sig!("bool", ["value" => Type::Any], Type::Bool);
        sig!("type", ["value" => Type::Any], Type::String);

        // Collections
        sig!("len", ["value" => Type::Any], Type::Int);
        sig!("push", ["array" => Type::Array(Box::new(Type::Any)), "item" => Type::Any], Type::Array(Box::new(Type::Any)));
        sig!("pop", ["array" => Type::Array(Box::new(Type::Any))], Type::Any);
        sig!("keys", ["map" => Type::Any], Type::Array(Box::new(Type::String)));
        sig!("values", ["map" => Type::Any], Type::Array(Box::new(Type::Any)));
        sig!("entries", ["map" => Type::Any], Type::Array(Box::new(Type::Any)));
        sig!("has_key", ["map" => Type::Any, "key" => Type::String], Type::Bool);
        sig!("get_key", ["map" => Type::Any, "key" => Type::String], Type::Any);
        sig!("sort", ["array" => Type::Array(Box::new(Type::Any))], Type::Array(Box::new(Type::Any)));
        sig!("reverse", ["value" => Type::Any], Type::Any);
        sig!("contains", ["haystack" => Type::Any, "needle" => Type::Any], Type::Bool);
        sig!("filter", ["array" => Type::Array(Box::new(Type::Any)), "predicate" => Type::Any], Type::Array(Box::new(Type::Any)));
        sig!("transform", ["array" => Type::Array(Box::new(Type::Any)), "mapper" => Type::Any], Type::Array(Box::new(Type::Any)));
        sig!("first", ["array" => Type::Array(Box::new(Type::Any))], Type::Any, variadic);
        sig!("last", ["array" => Type::Array(Box::new(Type::Any))], Type::Any, variadic);
        sig!("concat", ["a" => Type::Any, "b" => Type::Any], Type::Any);
        sig!("slice", ["array" => Type::Array(Box::new(Type::Any)), "start" => Type::Int], Type::Array(Box::new(Type::Any)), variadic);
        sig!("is_empty", ["value" => Type::Any], Type::Bool);
        sig!("flatten", ["array" => Type::Array(Box::new(Type::Any))], Type::Array(Box::new(Type::Any)));

        // Math
        sig!("abs", ["n" => Type::Any], Type::Any);
        sig!("min", ["a" => Type::Any, "b" => Type::Any], Type::Any);
        sig!("max", ["a" => Type::Any, "b" => Type::Any], Type::Any);
        sig!("round", ["n" => Type::Float], Type::Int, variadic);
        sig!("floor", ["n" => Type::Float], Type::Int);
        sig!("ceil", ["n" => Type::Float], Type::Int);
        sig!("sqrt", ["n" => Type::Any], Type::Float);
        sig!("sign", ["n" => Type::Any], Type::Int);
        sig!("clamp", ["n" => Type::Any, "min" => Type::Any, "max" => Type::Any], Type::Any);

        // Assertions
        sig!("assert", ["condition" => Type::Bool], Type::Unit, variadic);

        // HTTP server builtins (global)
        sig!("get", ["pattern" => Type::String, "handler" => Type::Any], Type::Unit);
        sig!("post", ["pattern" => Type::String, "handler" => Type::Any], Type::Unit);
        sig!("put", ["pattern" => Type::String, "handler" => Type::Any], Type::Unit);
        sig!("patch", ["pattern" => Type::String, "handler" => Type::Any], Type::Unit);
        sig!("delete", ["pattern" => Type::String, "handler" => Type::Any], Type::Unit);
        sig!("listen", ["port" => Type::Int], Type::Unit);
        sig!("serve_static", ["prefix" => Type::String, "dir" => Type::String], Type::Unit);
        sig!("use_middleware", ["handler" => Type::Any], Type::Unit);
        sig!("on_shutdown", ["handler" => Type::Any], Type::Unit);
        sig!("routes", ["dir" => Type::String], Type::Unit);
        sig!("template", ["path" => Type::String, "vars" => Type::Any], Type::String);

        // Utility
        sig!("unwrap", ["value" => Type::Any], Type::Any);

        // Register synthetic struct types for HTTP
        let map_string_string = Type::Map {
            key_type: Box::new(Type::String),
            value_type: Box::new(Type::String),
        };

        // Request — matches BridgeRequest fields from http_bridge.rs
        self.structs.insert(
            "Request".to_string(),
            vec![
                ("method".to_string(), Type::String),
                ("path".to_string(), Type::String),
                ("url".to_string(), Type::String),
                ("query".to_string(), Type::String),
                ("body".to_string(), Type::String),
                ("id".to_string(), Type::String),
                ("ip".to_string(), Type::String),
                ("protocol".to_string(), Type::String),
                ("query_params".to_string(), map_string_string.clone()),
                ("params".to_string(), map_string_string.clone()),
                ("headers".to_string(), map_string_string.clone()),
            ],
        );

        // Response — for html(), json(), etc. return values
        self.structs.insert(
            "Response".to_string(),
            vec![
                ("status".to_string(), Type::Int),
                ("body".to_string(), Type::String),
                ("headers".to_string(), map_string_string),
            ],
        );
    }
}

// ── Stdlib module signature registry ──────────────────────────────────

fn get_module_signatures(module: &str) -> HashMap<String, FunctionSig> {
    let mut sigs = HashMap::new();

    macro_rules! sig {
        ($name:expr, [$($pname:expr => $ptype:expr),*], $ret:expr) => {
            sigs.insert($name.to_string(), FunctionSig {
                params: vec![$(($pname.to_string(), $ptype)),*],
                return_type: $ret,
                variadic: false,
            });
        };
        ($name:expr, [$($pname:expr => $ptype:expr),*], $ret:expr, variadic) => {
            sigs.insert($name.to_string(), FunctionSig {
                params: vec![$(($pname.to_string(), $ptype)),*],
                return_type: $ret,
                variadic: true,
            });
        };
    }

    match module {
        "std/string" => {
            sig!("split", ["s" => Type::String, "delim" => Type::String], Type::Array(Box::new(Type::String)));
            sig!("join", ["arr" => Type::Array(Box::new(Type::String)), "delim" => Type::String], Type::String);
            sig!("trim", ["s" => Type::String], Type::String);
            sig!("trim_left", ["s" => Type::String], Type::String);
            sig!("trim_right", ["s" => Type::String], Type::String);
            sig!("trim_chars", ["s" => Type::String, "chars" => Type::String], Type::String);
            sig!("to_lower", ["s" => Type::String], Type::String);
            sig!("to_upper", ["s" => Type::String], Type::String);
            sig!("replace", ["s" => Type::String, "from" => Type::String, "to" => Type::String], Type::String);
            sig!("replace_first", ["s" => Type::String, "from" => Type::String, "to" => Type::String], Type::String);
            sig!("replace_all", ["s" => Type::String, "from" => Type::String, "to" => Type::String], Type::String);
            sig!("replace_chars", ["s" => Type::String, "chars" => Type::String, "replacement" => Type::String], Type::String);
            sig!("remove_chars", ["s" => Type::String, "chars" => Type::String], Type::String);
            sig!("keep_chars", ["s" => Type::String, "chars" => Type::String], Type::String);
            sig!("contains", ["s" => Type::String, "sub" => Type::String], Type::Bool);
            sig!("starts_with", ["s" => Type::String, "prefix" => Type::String], Type::Bool);
            sig!("ends_with", ["s" => Type::String, "suffix" => Type::String], Type::Bool);
            sig!("index_of", ["s" => Type::String, "sub" => Type::String], Type::Int);
            sig!("last_index_of", ["s" => Type::String, "sub" => Type::String], Type::Int);
            sig!("substring", ["s" => Type::String, "start" => Type::Int, "end" => Type::Int], Type::String);
            sig!("char_at", ["s" => Type::String, "idx" => Type::Int], Type::String);
            sig!("chars", ["s" => Type::String], Type::Array(Box::new(Type::String)));
            sig!("repeat", ["s" => Type::String, "n" => Type::Int], Type::String);
            sig!("pad_left", ["s" => Type::String, "len" => Type::Int, "char" => Type::String], Type::String);
            sig!("pad_right", ["s" => Type::String, "len" => Type::Int, "char" => Type::String], Type::String);
            sig!("reverse", ["s" => Type::String], Type::String);
            sig!("capitalize", ["s" => Type::String], Type::String);
            sig!("title", ["s" => Type::String], Type::String);
            sig!("is_uppercase", ["s" => Type::String], Type::Bool);
            sig!("is_lowercase", ["s" => Type::String], Type::Bool);
            sig!("is_numeric", ["s" => Type::String], Type::Bool);
            sig!("is_alphanumeric", ["s" => Type::String], Type::Bool);
            sig!("count", ["s" => Type::String, "sub" => Type::String], Type::Int);
            sig!("lines", ["s" => Type::String], Type::Array(Box::new(Type::String)));
            sig!("words", ["s" => Type::String], Type::Array(Box::new(Type::String)));
            sig!("truncate", ["s" => Type::String, "len" => Type::Int], Type::String, variadic);
            sig!("slugify", ["s" => Type::String], Type::String);
            sig!("matches_pattern", ["s" => Type::String, "pattern" => Type::String], Type::Bool);
            sig!("replace_pattern", ["s" => Type::String, "pattern" => Type::String, "replacement" => Type::String], Type::String);
            sig!("find_pattern", ["s" => Type::String, "pattern" => Type::String], Type::Optional(Box::new(Type::String)));
            sig!("find_all_pattern", ["s" => Type::String, "pattern" => Type::String], Type::Array(Box::new(Type::String)));
            sig!("split_pattern", ["s" => Type::String, "pattern" => Type::String], Type::Array(Box::new(Type::String)));
        }
        "std/math" => {
            sig!("sin", ["x" => Type::Float], Type::Float);
            sig!("cos", ["x" => Type::Float], Type::Float);
            sig!("tan", ["x" => Type::Float], Type::Float);
            sig!("asin", ["x" => Type::Float], Type::Float);
            sig!("acos", ["x" => Type::Float], Type::Float);
            sig!("atan", ["x" => Type::Float], Type::Float);
            sig!("atan2", ["y" => Type::Float, "x" => Type::Float], Type::Float);
            sig!("log", ["x" => Type::Float], Type::Float);
            sig!("log2", ["x" => Type::Float], Type::Float);
            sig!("log10", ["x" => Type::Float], Type::Float);
            sig!("exp", ["x" => Type::Float], Type::Float);
            sig!("pow", ["base" => Type::Float, "exp" => Type::Float], Type::Float);
            sig!("random", [], Type::Float);
            sig!("random_int", ["min" => Type::Int, "max" => Type::Int], Type::Int);
            sig!("PI", [], Type::Float);
            sig!("E", [], Type::Float);
        }
        "std/collections" => {
            sig!("push", ["array" => Type::Array(Box::new(Type::Any)), "item" => Type::Any], Type::Array(Box::new(Type::Any)));
            sig!("pop", ["array" => Type::Array(Box::new(Type::Any))], Type::Any);
            sig!("keys", ["map" => Type::Any], Type::Array(Box::new(Type::String)));
            sig!("values", ["map" => Type::Any], Type::Array(Box::new(Type::Any)));
            sig!("entries", ["map" => Type::Any], Type::Array(Box::new(Type::Any)));
            sig!("has_key", ["map" => Type::Any, "key" => Type::String], Type::Bool);
            sig!("get_key", ["map" => Type::Any, "key" => Type::String], Type::Any, variadic);
            sig!("first", ["array" => Type::Array(Box::new(Type::Any))], Type::Any, variadic);
            sig!("last", ["array" => Type::Array(Box::new(Type::Any))], Type::Any, variadic);
            sig!("concat", ["a" => Type::Any, "b" => Type::Any], Type::Any);
            sig!("slice", ["array" => Type::Array(Box::new(Type::Any)), "start" => Type::Int], Type::Array(Box::new(Type::Any)), variadic);
            sig!("sort", ["array" => Type::Array(Box::new(Type::Any))], Type::Array(Box::new(Type::Any)));
            sig!("reverse", ["array" => Type::Array(Box::new(Type::Any))], Type::Array(Box::new(Type::Any)));
            sig!("is_empty", ["value" => Type::Any], Type::Bool);
            sig!("flatten", ["array" => Type::Array(Box::new(Type::Any))], Type::Array(Box::new(Type::Any)));
            sig!("filter", ["array" => Type::Array(Box::new(Type::Any)), "predicate" => Type::Any], Type::Array(Box::new(Type::Any)));
            sig!("transform", ["array" => Type::Array(Box::new(Type::Any)), "mapper" => Type::Any], Type::Array(Box::new(Type::Any)));
        }
        "std/json" => {
            sig!("parse", ["s" => Type::String], Type::Any);
            sig!("stringify", ["value" => Type::Any], Type::String);
            sig!("stringify_pretty", ["value" => Type::Any], Type::String);
        }
        "std/fs" => {
            sig!("read_file", ["path" => Type::String], Type::String);
            sig!("write_file", ["path" => Type::String, "content" => Type::String], Type::Unit);
            sig!("exists", ["path" => Type::String], Type::Bool);
            sig!("is_file", ["path" => Type::String], Type::Bool);
            sig!("is_dir", ["path" => Type::String], Type::Bool);
            sig!("mkdir", ["path" => Type::String], Type::Unit);
            sig!("readdir", ["path" => Type::String], Type::Array(Box::new(Type::String)));
            sig!("remove", ["path" => Type::String], Type::Unit);
            sig!("copy", ["src" => Type::String, "dst" => Type::String], Type::Unit);
            sig!("rename", ["src" => Type::String, "dst" => Type::String], Type::Unit);
        }
        "std/env" => {
            sig!("get_env", ["name" => Type::String], Type::Optional(Box::new(Type::String)));
            sig!("set_env", ["name" => Type::String, "value" => Type::String], Type::Unit);
            sig!("all_env", [], Type::Any);
            sig!("load_env", ["path" => Type::String], Type::Unit);
            sig!("args", [], Type::Array(Box::new(Type::String)));
            sig!("cwd", [], Type::String);
        }
        "std/http" => {
            sig!("fetch", ["url" => Type::String], Type::Generic {
                name: "Result".to_string(),
                args: vec![Type::Named("Response".to_string()), Type::String],
            }, variadic);
            sig!("download", ["url" => Type::String, "path" => Type::String], Type::Any);
            sig!("Cache", ["ttl" => Type::Int], Type::Any);
            sig!("cache_fetch", ["cache" => Type::Any, "url" => Type::String], Type::Any, variadic);
        }
        "std/http/server" => {
            sig!("json", ["data" => Type::Any], Type::Named("Response".to_string()), variadic);
            sig!("html", ["content" => Type::String], Type::Named("Response".to_string()), variadic);
            sig!("text", ["content" => Type::String], Type::Named("Response".to_string()));
            sig!("redirect", ["url" => Type::String], Type::Named("Response".to_string()));
            sig!("status", ["code" => Type::Int, "body" => Type::Any], Type::Named("Response".to_string()));
            sig!("parse_json", ["req" => Type::Any], Type::Generic {
                name: "Result".to_string(),
                args: vec![
                    Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    },
                    Type::String,
                ],
            });
            sig!("parse_form", ["req" => Type::Any], Type::Map {
                key_type: Box::new(Type::String),
                value_type: Box::new(Type::String),
            });
        }
        "std/db/postgres" => {
            sig!("connect", ["url" => Type::String], Type::Any);
            sig!("query", ["conn" => Type::Any, "sql" => Type::String], Type::Any, variadic);
            sig!("execute", ["conn" => Type::Any, "sql" => Type::String], Type::Any, variadic);
            sig!("close", ["conn" => Type::Any], Type::Unit);
        }
        "std/db/sqlite" => {
            sig!("connect", ["path" => Type::String], Type::Any);
            sig!("query", ["conn" => Type::Any, "sql" => Type::String], Type::Any, variadic);
            sig!("query_one", ["conn" => Type::Any, "sql" => Type::String], Type::Any, variadic);
            sig!("execute", ["conn" => Type::Any, "sql" => Type::String], Type::Any, variadic);
            sig!("transaction", ["conn" => Type::Any, "callback" => Type::Any], Type::Any);
            sig!("close", ["conn" => Type::Any], Type::Unit);
        }
        "std/url" => {
            sig!("encode", ["s" => Type::String], Type::String);
            sig!("decode", ["s" => Type::String], Type::String);
            sig!("parse", ["url" => Type::String], Type::Any);
            sig!("parse_query", ["query" => Type::String], Type::Any);
            sig!("build_query", ["params" => Type::Any], Type::String);
            sig!("join", ["base" => Type::String, "path" => Type::String], Type::String);
        }
        "std/path" => {
            sig!("join", ["parts" => Type::String], Type::String, variadic);
            sig!("dirname", ["path" => Type::String], Type::String);
            sig!("basename", ["path" => Type::String], Type::String);
            sig!("extname", ["path" => Type::String], Type::String);
            sig!("is_absolute", ["path" => Type::String], Type::Bool);
        }
        "std/time" => {
            sig!("now", [], Type::Any);
            sig!("now_millis", [], Type::Int);
            sig!("format", ["time" => Type::Any, "fmt" => Type::String], Type::String);
            sig!("elapsed", ["start" => Type::Any], Type::Any);
            sig!("duration", ["ms" => Type::Int], Type::Any);
            sig!("parse_datetime", ["date_str" => Type::String, "format" => Type::String], Type::Generic {
                name: "Result".to_string(),
                args: vec![Type::Int, Type::String],
            });
        }
        "std/concurrent" => {
            sig!("channel", [], Type::Any);
            sig!("send", ["ch" => Type::Any, "value" => Type::Any], Type::Unit);
            sig!("recv", ["ch" => Type::Any], Type::Any);
            sig!("sleep_ms", ["ms" => Type::Int], Type::Unit);
        }
        "std/csv" => {
            sig!("parse", ["s" => Type::String], Type::Array(Box::new(Type::Array(Box::new(Type::String)))));
            sig!("parse_csv", ["s" => Type::String], Type::Array(Box::new(Type::Array(Box::new(Type::String)))));
            sig!("parse_with_headers", ["s" => Type::String], Type::Array(Box::new(Type::Any)));
            sig!("stringify", ["data" => Type::Array(Box::new(Type::Any))], Type::String);
            sig!("stringify_with_headers", ["data" => Type::Array(Box::new(Type::Any)), "headers" => Type::Array(Box::new(Type::String))], Type::String);
        }
        "std/crypto" => {
            sig!("sha256", ["data" => Type::String], Type::String);
            sig!("sha256_bytes", ["data" => Type::String], Type::Array(Box::new(Type::Int)));
            sig!("hmac", ["key" => Type::String, "data" => Type::String], Type::String, variadic);
            sig!("random_bytes", ["n" => Type::Int], Type::Array(Box::new(Type::Int)));
            sig!("random_hex", ["n" => Type::Int], Type::String);
            sig!("hex_encode", ["data" => Type::Any], Type::String);
            sig!("hex_decode", ["s" => Type::String], Type::Array(Box::new(Type::Int)));
            sig!("uuid", [], Type::String);
        }
        _ => {
            // Unknown module — imports will be bound as Any
        }
    }

    sigs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn check(source: &str) -> Vec<TypeDiagnostic> {
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        check_program(&ast, source)
    }

    fn check_errors(source: &str) -> Vec<TypeDiagnostic> {
        check(source)
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .collect()
    }

    fn check_warnings(source: &str) -> Vec<TypeDiagnostic> {
        check(source)
            .into_iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect()
    }

    // ── Literal type inference ──────────────────────────────────

    #[test]
    fn test_infer_int_literal() {
        let diags = check("let x: Int = 42");
        assert!(diags.is_empty(), "No errors for correct Int assignment");
    }

    #[test]
    fn test_infer_float_literal() {
        let diags = check("let x: Float = 3.14");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_infer_string_literal() {
        let diags = check(r#"let x: String = "hello""#);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_infer_bool_literal() {
        let diags = check("let x: Bool = true");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_infer_unit_literal() {
        let diags = check("let x: Unit = ()");
        assert!(diags.is_empty());
    }

    // ── Type mismatch errors ────────────────────────────────────

    #[test]
    fn test_type_mismatch_int_to_string() {
        let errs = check_errors(r#"let x: String = 42"#);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("Type mismatch"));
        assert!(errs[0].message.contains("String"));
        assert!(errs[0].message.contains("Int"));
    }

    #[test]
    fn test_type_mismatch_string_to_int() {
        let errs = check_errors(r#"let x: Int = "hello""#);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("Type mismatch"));
    }

    #[test]
    fn test_type_mismatch_bool_to_float() {
        let errs = check_errors("let x: Float = true");
        assert_eq!(errs.len(), 1);
    }

    // ── Numeric coercion ────────────────────────────────────────

    #[test]
    fn test_int_float_coercion() {
        let diags = check("let x: Float = 42");
        assert!(diags.is_empty(), "Int should coerce to Float");
    }

    // ── Binary operator type rules ──────────────────────────────

    #[test]
    fn test_add_int_int() {
        let diags = check("let x: Int = 1 + 2");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_add_int_float() {
        let diags = check("let x: Float = 1 + 2.0");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_add_string_concat() {
        let diags = check(r#"let x: String = "a" + "b""#);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_comparison_returns_bool() {
        let diags = check("let x: Bool = 1 < 2");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_logical_returns_bool() {
        let diags = check("let x: Bool = true && false");
        assert!(diags.is_empty());
    }

    // ── Variable binding and lookup ─────────────────────────────

    #[test]
    fn test_variable_type_propagation() {
        let diags = check(
            r#"
            let x = 42
            let y: Int = x
            "#,
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_variable_type_mismatch_propagation() {
        let errs = check_errors(
            r#"
            let x = "hello"
            let y: Int = x
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("Type mismatch"));
    }

    // ── Function call checking ──────────────────────────────────

    #[test]
    fn test_function_correct_args() {
        let diags = check(
            r#"
            fn add(a: Int, b: Int) -> Int {
                return a + b
            }
            let result: Int = add(1, 2)
            "#,
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_function_wrong_arg_count() {
        let errs = check_errors(
            r#"
            fn add(a: Int, b: Int) -> Int {
                return a + b
            }
            add(1)
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("expects 2"));
        assert!(errs[0].message.contains("got 1"));
    }

    #[test]
    fn test_function_wrong_arg_type() {
        let errs = check_errors(
            r#"
            fn greet(name: String) -> String {
                return "hello"
            }
            greet(42)
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("expected String"));
        assert!(errs[0].message.contains("got Int"));
    }

    // ── Return type checking ────────────────────────────────────

    #[test]
    fn test_return_type_mismatch() {
        let errs = check_errors(
            r#"
            fn get_name() -> String {
                return 42
            }
            "#,
        );
        // Should catch return type mismatch
        assert!(!errs.is_empty());
        assert!(errs
            .iter()
            .any(|e| e.message.contains("Return type mismatch")
                || e.message.contains("type mismatch")));
    }

    #[test]
    fn test_return_type_correct() {
        let diags = check(
            r#"
            fn get_name() -> String {
                return "Alice"
            }
            "#,
        );
        assert!(diags.is_empty());
    }

    // ── Untyped code (gradual typing) ───────────────────────────

    #[test]
    fn test_untyped_code_no_errors() {
        let diags = check(
            r#"
            let x = 42
            let y = "hello"
            fn foo(a, b) {
                return a + b
            }
            foo(x, y)
            "#,
        );
        assert!(diags.is_empty(), "Untyped code should produce no errors");
    }

    #[test]
    fn test_mixed_typed_untyped_no_false_positives() {
        let diags = check(
            r#"
            fn typed_fn(a: Int) -> Int {
                return a + 1
            }
            let x = 10
            typed_fn(x)
            "#,
        );
        assert!(diags.is_empty());
    }

    // ── Array/Map inference ─────────────────────────────────────

    #[test]
    fn test_array_homogeneous() {
        let diags = check("let x = [1, 2, 3]");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_empty_array() {
        let diags = check("let x = []");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_map_literal() {
        let diags = check(r#"let x = map { "a": 1, "b": 2 }"#);
        assert!(diags.is_empty());
    }

    // ── Option/Result inference ─────────────────────────────────

    #[test]
    fn test_option_some() {
        let diags = check("let x = Some(42)");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_option_none() {
        let diags = check("let x = None");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_result_ok() {
        let diags = check("let x = Ok(42)");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_result_err() {
        let diags = check(r#"let x = Err("bad")"#);
        assert!(diags.is_empty());
    }

    // ── Match expression ────────────────────────────────────────

    #[test]
    fn test_match_basic() {
        let diags = check(
            r#"
            let x = 42
            let y = match x {
                1 => "one",
                _ => "other"
            }
            "#,
        );
        assert!(diags.is_empty());
    }

    // ── Import resolution ───────────────────────────────────────

    #[test]
    fn test_import_stdlib() {
        let diags = check(
            r#"
            import { split } from "std/string"
            let parts = split("a,b,c", ",")
            "#,
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_import_wrong_arg_type() {
        let errs = check_errors(
            r#"
            import { split } from "std/string"
            split(42, ",")
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("expected String"));
        assert!(errs[0].message.contains("got Int"));
    }

    // ── Scope nesting ───────────────────────────────────────────

    #[test]
    fn test_scope_nesting() {
        let diags = check(
            r#"
            let x: Int = 1
            if true {
                let y: Int = x + 1
            }
            "#,
        );
        assert!(diags.is_empty());
    }

    // ── Struct checking ─────────────────────────────────────────

    #[test]
    fn test_struct_field_type() {
        let diags = check(
            r#"
            struct Point {
                x: Int,
                y: Int,
            }
            let p = Point { x: 1, y: 2 }
            "#,
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_struct_field_mismatch() {
        let errs = check_errors(
            r#"
            struct Point {
                x: Int,
                y: Int,
            }
            let p = Point { x: "wrong", y: 2 }
            "#,
        );
        assert!(!errs.is_empty());
        assert!(errs[0].message.contains("expected Int"));
        assert!(errs[0].message.contains("got String"));
    }

    // ── Forward references ──────────────────────────────────────

    #[test]
    fn test_forward_reference() {
        let diags = check(
            r#"
            let result: Int = add(1, 2)
            fn add(a: Int, b: Int) -> Int {
                return a + b
            }
            "#,
        );
        assert!(diags.is_empty(), "Forward references should work");
    }

    // ── Builtin function checking ───────────────────────────────

    #[test]
    fn test_builtin_len() {
        let diags = check(
            r#"
            let n: Int = len("hello")
            "#,
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_builtin_str() {
        let diags = check(
            r#"
            let s: String = str(42)
            "#,
        );
        assert!(diags.is_empty());
    }

    // ── Condition type warnings ─────────────────────────────────

    #[test]
    fn test_non_bool_condition_warning() {
        let warnings = check_warnings(
            r#"
            if 42 {
                let x = 1
            }
            "#,
        );
        assert!(!warnings.is_empty());
        assert!(warnings[0].message.contains("Int instead of Bool"));
    }

    // ── Strict lint tests ─────────────────────────────────────────

    fn check_strict(source: &str) -> Vec<TypeDiagnostic> {
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().unwrap();
        check_program_strict(&ast, source)
    }

    fn check_strict_warnings(source: &str) -> Vec<TypeDiagnostic> {
        check_strict(source)
            .into_iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect()
    }

    #[test]
    fn test_strict_warns_untyped_param() {
        let warnings = check_strict_warnings("fn greet(name) { return name }");
        let param_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("no type annotation"))
            .collect();
        assert_eq!(param_warnings.len(), 1);
        assert!(param_warnings[0].message.contains("name"));
        assert!(param_warnings[0].message.contains("greet"));
    }

    #[test]
    fn test_strict_warns_missing_return_type() {
        let warnings = check_strict_warnings("fn add(a: Int, b: Int) { return a + b }");
        let ret_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("no return type"))
            .collect();
        assert_eq!(ret_warnings.len(), 1);
        assert!(ret_warnings[0].message.contains("add"));
    }

    #[test]
    fn test_strict_no_warnings_fully_typed() {
        let warnings = check_strict_warnings("fn add(a: Int, b: Int) -> Int { return a + b }");
        let strict_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| {
                w.message.contains("no type annotation") || w.message.contains("no return type")
            })
            .collect();
        assert!(
            strict_warnings.is_empty(),
            "Fully typed function should have no strict warnings: {:?}",
            strict_warnings
        );
    }

    #[test]
    fn test_strict_warns_multiple_untyped_params() {
        let warnings = check_strict_warnings("fn calc(a, b, c) { return a }");
        let param_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("no type annotation"))
            .collect();
        assert_eq!(param_warnings.len(), 3);
    }

    #[test]
    fn test_non_strict_no_untyped_warnings() {
        // Normal (non-strict) mode should NOT warn about untyped params
        let warnings = check_warnings("fn greet(name) { return name }");
        let param_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("no type annotation"))
            .collect();
        assert!(
            param_warnings.is_empty(),
            "Non-strict mode should not warn about untyped params"
        );
    }

    // ── Contract type-checking tests ──────────────────────────────

    #[test]
    fn test_requires_valid_bool() {
        let errors = check_errors(
            r#"
            fn divide(a: Int, b: Int) -> Int
                requires b != 0
            {
                return a / b
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Valid requires (Bool expression) should produce no errors: {:?}",
            errors
        );
    }

    #[test]
    fn test_ensures_result_typed() {
        let errors = check_errors(
            r#"
            fn double(x: Int) -> Int
                ensures result == x * 2
            {
                return x * 2
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Valid ensures with result should produce no errors: {:?}",
            errors
        );
    }

    #[test]
    fn test_ensures_result_with_function_call() {
        // ensures len(result) > 0 — verify result is a type len() accepts
        let errors = check_errors(
            r#"
            fn greet(name: String) -> String
                ensures len(result) > 0
            {
                return "Hello, " + name
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "ensures len(result) > 0 should be valid when result is String: {:?}",
            errors
        );
    }

    #[test]
    fn test_old_in_ensures() {
        let errors = check_errors(
            r#"
            fn increment(x: Int) -> Int
                ensures result == old(x) + 1
            {
                return x + 1
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "old(x) should be same type as x (Int): {:?}",
            errors
        );
    }

    #[test]
    fn test_requires_and_ensures_together() {
        let errors = check_errors(
            r#"
            fn safe_divide(a: Int, b: Int) -> Int
                requires b != 0
                ensures result * b == a
            {
                return a / b
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Combined requires/ensures should work: {:?}",
            errors
        );
    }

    #[test]
    fn test_contract_no_errors_untyped() {
        // Contracts with untyped functions should not produce errors (gradual typing)
        let errors = check_errors(
            r#"
            fn process(data)
                requires len(data) > 0
            {
                return data
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Untyped function contracts should not produce errors: {:?}",
            errors
        );
    }

    // ── Request/Response type inference ─────────────────────────

    #[test]
    fn test_request_field_access_string() {
        let errors = check_errors(
            r#"
            fn handler(req: Request) {
                let m: String = req.method
                let p: String = req.path
                let b: String = req.body
                let i: String = req.id
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Request string fields should resolve to String: {:?}",
            errors
        );
    }

    #[test]
    fn test_request_field_access_maps() {
        let errors = check_errors(
            r#"
            fn handler(req: Request) {
                let params = req.params
                let headers = req.headers
                let qp = req.query_params
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Request map fields should resolve: {:?}",
            errors
        );
    }

    #[test]
    fn test_response_type_from_html() {
        let errors = check_errors(
            r#"
            import { html } from "std/http/server"
            fn handler(req: Request) -> Response {
                return html("<h1>Hello</h1>")
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "html() should return Response: {:?}",
            errors
        );
    }

    #[test]
    fn test_response_type_from_json() {
        let errors = check_errors(
            r#"
            import { json } from "std/http/server"
            fn handler(req: Request) -> Response {
                return json(map { "ok": true })
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "json() should return Response: {:?}",
            errors
        );
    }

    // ── unwrap() generic awareness ──────────────────────────────

    #[test]
    fn test_unwrap_optional() {
        let errors = check_errors(
            r#"
            let x = Some(42)
            let y: Int = unwrap(x)
            "#,
        );
        assert!(
            errors.is_empty(),
            "unwrap(Optional<Int>) should return Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_unwrap_result() {
        let errors = check_errors(
            r#"
            let x = Ok(42)
            let y: Int = unwrap(x)
            "#,
        );
        assert!(
            errors.is_empty(),
            "unwrap(Result<Int,Any>) should return Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_unwrap_method_optional() {
        let errors = check_errors(
            r#"
            let x = Some("hello")
            let y: String = x.unwrap()
            "#,
        );
        assert!(
            errors.is_empty(),
            "Some(String).unwrap() should return String: {:?}",
            errors
        );
    }

    #[test]
    fn test_unwrap_method_result() {
        let errors = check_errors(
            r#"
            let x = Ok(3.14)
            let y: Float = x.unwrap()
            "#,
        );
        assert!(
            errors.is_empty(),
            "Ok(Float).unwrap() should return Float: {:?}",
            errors
        );
    }

    // ── filter() element type preservation ──────────────────────

    #[test]
    fn test_filter_preserves_array_type() {
        let errors = check_errors(
            r#"
            fn is_positive(n: Int) -> Bool { return n > 0 }
            let nums: Array<Int> = [1, -2, 3]
            let result: Array<Int> = filter(nums, is_positive)
            "#,
        );
        assert!(
            errors.is_empty(),
            "filter(Array<Int>) should return Array<Int>: {:?}",
            errors
        );
    }

    // ── stdlib signature fixes ──────────────────────────────────

    #[test]
    fn test_cache_import() {
        let errors = check_errors(
            r#"
            import { Cache, cache_fetch } from "std/http"
            let c = Cache(600)
            "#,
        );
        assert!(
            errors.is_empty(),
            "Cache and cache_fetch should be importable: {:?}",
            errors
        );
    }

    #[test]
    fn test_parse_csv_import() {
        let errors = check_errors(
            r#"
            import { parse_csv } from "std/csv"
            let rows: Array<Array<String>> = parse_csv("a,b\n1,2")
            "#,
        );
        assert!(
            errors.is_empty(),
            "parse_csv should resolve with correct return type: {:?}",
            errors
        );
    }

    #[test]
    fn test_parse_datetime_import() {
        let errors = check_errors(
            r#"
            import { parse_datetime } from "std/time"
            let result = parse_datetime("2024-01-01 00:00", "%Y-%m-%d %H:%M")
            "#,
        );
        assert!(
            errors.is_empty(),
            "parse_datetime should be importable: {:?}",
            errors
        );
    }

    #[test]
    fn test_parse_datetime_unwrap_returns_int() {
        let errors = check_errors(
            r#"
            import { parse_datetime } from "std/time"
            let result = parse_datetime("2024-01-01 00:00", "%Y-%m-%d %H:%M")
            let ts: Int = unwrap(result)
            "#,
        );
        assert!(
            errors.is_empty(),
            "unwrap(parse_datetime(...)) should return Int: {:?}",
            errors
        );
    }

    // ── Collection functions preserve element type ──────────────

    #[test]
    fn test_sort_preserves_array_type() {
        let errors = check_errors(
            r#"
            let nums: Array<Int> = [3, 1, 2]
            let sorted: Array<Int> = sort(nums)
            "#,
        );
        assert!(
            errors.is_empty(),
            "sort(Array<Int>) should return Array<Int>: {:?}",
            errors
        );
    }

    #[test]
    fn test_reverse_preserves_array_type() {
        let errors = check_errors(
            r#"
            let names: Array<String> = ["a", "b", "c"]
            let rev: Array<String> = reverse(names)
            "#,
        );
        assert!(
            errors.is_empty(),
            "reverse(Array<String>) should return Array<String>: {:?}",
            errors
        );
    }

    #[test]
    fn test_slice_preserves_array_type() {
        let errors = check_errors(
            r#"
            let nums: Array<Int> = [1, 2, 3, 4, 5]
            let sliced: Array<Int> = slice(nums, 1, 3)
            "#,
        );
        assert!(
            errors.is_empty(),
            "slice(Array<Int>) should return Array<Int>: {:?}",
            errors
        );
    }

    #[test]
    fn test_concat_preserves_array_type() {
        let errors = check_errors(
            r#"
            let a: Array<Int> = [1, 2]
            let b: Array<Int> = [3, 4]
            let c: Array<Int> = concat(a, b)
            "#,
        );
        assert!(
            errors.is_empty(),
            "concat(Array<Int>, Array<Int>) should return Array<Int>: {:?}",
            errors
        );
    }

    #[test]
    fn test_flatten_preserves_element_type() {
        let errors = check_errors(
            r#"
            let nested: Array<Array<Int>> = [[1, 2], [3, 4]]
            let flat: Array<Int> = flatten(nested)
            "#,
        );
        assert!(
            errors.is_empty(),
            "flatten(Array<Array<Int>>) should return Array<Int>: {:?}",
            errors
        );
    }

    #[test]
    fn test_push_preserves_array_type() {
        let errors = check_errors(
            r#"
            let nums: Array<Int> = [1, 2]
            let result: Array<Int> = push(nums, 3)
            "#,
        );
        assert!(
            errors.is_empty(),
            "push(Array<Int>, Int) should return Array<Int>: {:?}",
            errors
        );
    }

    #[test]
    fn test_first_returns_element_type() {
        let errors = check_errors(
            r#"
            let nums: Array<Int> = [1, 2, 3]
            let f: Int = first(nums)
            "#,
        );
        assert!(
            errors.is_empty(),
            "first(Array<Int>) should return Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_last_returns_element_type() {
        let errors = check_errors(
            r#"
            let names: Array<String> = ["a", "b", "c"]
            let l: String = last(names)
            "#,
        );
        assert!(
            errors.is_empty(),
            "last(Array<String>) should return String: {:?}",
            errors
        );
    }

    #[test]
    fn test_pop_returns_element_type() {
        let errors = check_errors(
            r#"
            let nums: Array<Float> = [1.0, 2.0]
            let p: Float = pop(nums)
            "#,
        );
        assert!(
            errors.is_empty(),
            "pop(Array<Float>) should return Float: {:?}",
            errors
        );
    }

    // ── Math functions preserve numeric type ────────────────────

    #[test]
    fn test_abs_preserves_int() {
        let errors = check_errors(
            r#"
            let x: Int = -5
            let y: Int = abs(x)
            "#,
        );
        assert!(
            errors.is_empty(),
            "abs(Int) should return Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_abs_preserves_float() {
        let errors = check_errors(
            r#"
            let x: Float = -3.14
            let y: Float = abs(x)
            "#,
        );
        assert!(
            errors.is_empty(),
            "abs(Float) should return Float: {:?}",
            errors
        );
    }

    #[test]
    fn test_min_max_int() {
        let errors = check_errors(
            r#"
            let a: Int = 3
            let b: Int = 7
            let lo: Int = min(a, b)
            let hi: Int = max(a, b)
            "#,
        );
        assert!(
            errors.is_empty(),
            "min/max(Int, Int) should return Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_min_max_float_promotion() {
        let errors = check_errors(
            r#"
            let a: Int = 3
            let b: Float = 7.5
            let lo: Float = min(a, b)
            "#,
        );
        assert!(
            errors.is_empty(),
            "min(Int, Float) should return Float: {:?}",
            errors
        );
    }

    #[test]
    fn test_clamp_preserves_type() {
        let errors = check_errors(
            r#"
            let x: Int = 15
            let clamped: Int = clamp(x, 0, 10)
            "#,
        );
        assert!(
            errors.is_empty(),
            "clamp(Int, Int, Int) should return Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_map_index_returns_value_type() {
        let errors = check_errors(
            r#"
            let m: Map<String, Int> = map { "a": 1, "b": 2 }
            let v: Int = m["a"]
            "#,
        );
        assert!(
            errors.is_empty(),
            "map[key] should return value type Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_keys_returns_array_of_key_type() {
        let errors = check_errors(
            r#"
            import { keys } from "std/collections"
            let m: Map<String, Int> = map { "a": 1, "b": 2 }
            let k: Array<String> = keys(m)
            "#,
        );
        assert!(
            errors.is_empty(),
            "keys(Map<String, Int>) should return Array<String>: {:?}",
            errors
        );
    }

    #[test]
    fn test_values_returns_array_of_value_type() {
        let errors = check_errors(
            r#"
            import { values } from "std/collections"
            let m: Map<String, Int> = map { "a": 1, "b": 2 }
            let v: Array<Int> = values(m)
            "#,
        );
        assert!(
            errors.is_empty(),
            "values(Map<String, Int>) should return Array<Int>: {:?}",
            errors
        );
    }

    #[test]
    fn test_entries_returns_array_of_arrays() {
        let errors = check_errors(
            r#"
            import { entries } from "std/collections"
            let m: Map<String, Int> = map { "a": 1, "b": 2 }
            let e: Array<Array<Any>> = entries(m)
            "#,
        );
        assert!(
            errors.is_empty(),
            "entries(Map<String, Int>) should return Array<Array<Any>>: {:?}",
            errors
        );
    }

    #[test]
    fn test_get_key_returns_value_type() {
        let errors = check_errors(
            r#"
            import { get_key } from "std/collections"
            let m: Map<String, Int> = map { "a": 1, "b": 2 }
            let v: Int = get_key(m, "a")
            "#,
        );
        assert!(
            errors.is_empty(),
            "get_key(Map<String, Int>, key) should return Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_values_on_string_string_map() {
        let errors = check_errors(
            r#"
            import { values } from "std/collections"
            let headers: Map<String, String> = map { "content-type": "text/html" }
            let vals: Array<String> = values(headers)
            "#,
        );
        assert!(
            errors.is_empty(),
            "values(Map<String, String>) should return Array<String>: {:?}",
            errors
        );
    }

    // ── Step 1: transform/map callback return type inference ──────────

    #[test]
    fn test_transform_infers_callback_return_type() {
        let errors = check_errors(
            r#"
            fn double(n: Int) -> Int { return n * 2 }
            let nums: Array<Int> = [1, 2, 3]
            let result: Array<Int> = transform(nums, double)
            "#,
        );
        assert!(
            errors.is_empty(),
            "transform with typed callback should infer Array<Int>: {:?}",
            errors
        );
    }

    #[test]
    fn test_transform_with_string_callback() {
        let errors = check_errors(
            r#"
            fn to_str(n: Int) -> String { return str(n) }
            let nums: Array<Int> = [1, 2, 3]
            let result: Array<String> = transform(nums, to_str)
            "#,
        );
        assert!(
            errors.is_empty(),
            "transform should infer Array<String> from callback: {:?}",
            errors
        );
    }

    #[test]
    fn test_transform_unresolvable_falls_back() {
        // When callback is not a known function, should still return Array<Any> (no error)
        let errors = check_errors(
            r#"
            let nums = [1, 2, 3]
            let result = transform(nums, some_unknown_fn)
            "#,
        );
        assert!(
            errors.is_empty(),
            "Unresolvable callback should fall back gracefully: {:?}",
            errors
        );
    }

    // ── Step 2: parse_json / fetch return types ──────────────────────

    #[test]
    fn test_parse_json_returns_result_map() {
        let errors = check_errors(
            r#"
            import { parse_json } from "std/http/server"
            fn handler(req: Request) {
                let result = parse_json(req)
                match result {
                    Ok(data) => {
                        let val = data["key"]
                    },
                    Err(e) => {
                        let msg: String = e
                    }
                }
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "parse_json -> match Ok(data) -> data[key] should work: {:?}",
            errors
        );
    }

    #[test]
    fn test_parse_json_unwrap_returns_map() {
        let errors = check_errors(
            r#"
            import { parse_json } from "std/http/server"
            fn handler(req: Request) {
                let data = parse_json(req).unwrap()
                let val = data["key"]
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "parse_json(req).unwrap() should return Map type: {:?}",
            errors
        );
    }

    // ── Step 3: match arm type narrowing ─────────────────────────────

    #[test]
    fn test_match_narrows_result_ok() {
        // Test that match arm narrowing extracts the inner type from Result
        let errors = check_errors(
            r#"
            import { parse_datetime } from "std/time"
            let result = parse_datetime("2024-01-01", "%Y-%m-%d")
            let value: Int = match result {
                Ok(n) => n,
                Err(e) => 0
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Match on Result<Int, String> should narrow Ok(n) to Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_match_narrows_option_some() {
        let errors = check_errors(
            r#"
            fn find_item(id: Int) -> Option<String> {
                return Some("found")
            }
            let result = find_item(1)
            let value: String = match result {
                Some(s) => s,
                None => "default"
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Match on Option<String> should narrow Some(s) to String: {:?}",
            errors
        );
    }

    // ── Step 4: cross-file import resolution helpers ─────────────────

    #[test]
    fn test_resolve_import_path_std_returns_none() {
        let ctx = TypeContext::new("");
        assert!(
            ctx.resolve_import_path("std/string").is_none(),
            "std/ paths should not resolve to files"
        );
    }

    #[test]
    fn test_resolve_import_path_no_current_file() {
        let ctx = TypeContext::new("");
        assert!(
            ctx.resolve_import_path("./utils").is_none(),
            "Without current_file, relative imports should return None"
        );
    }

    #[test]
    fn test_resolve_import_path_relative() {
        let mut ctx = TypeContext::new("");
        ctx.current_file = Some("/project/server.tnt".to_string());
        let result = ctx.resolve_import_path("./lib/utils");
        assert!(result.is_some(), "Relative import should resolve");
        let path = result.unwrap();
        assert!(
            path.to_string_lossy().contains("lib")
                && path.to_string_lossy().contains("utils")
                && path.to_string_lossy().ends_with(".tnt"),
            "Should resolve to lib/utils.tnt, got: {}",
            path.display()
        );
    }
}
