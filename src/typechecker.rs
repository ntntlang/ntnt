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

/// Classification of type diagnostics for structured matching.
///
/// Used by `check_program_with_lint_mode` to promote annotation warnings
/// to errors in strict mode without brittle substring matching.
#[derive(Debug, Clone, PartialEq)]
pub enum DiagnosticKind {
    /// Missing type annotation on function parameter
    MissingParamAnnotation,
    /// Missing return type annotation on function
    MissingReturnAnnotation,
    /// Missing type annotation on lambda parameter
    MissingLambdaParamAnnotation,
    /// JavaScript-style `${ident}` interpolation in a string literal where
    /// `ident` is a variable in scope (NTNT interpolation is `#{expr}`)
    JsStyleInterpolation,
    /// Dot-call on a method name that is not a defined or imported function
    /// (UFCS means `x.f()` needs a reachable function `f`)
    UnknownMethod,
    /// `let x = { ... }` binds a bare block (Unit or its last expression),
    /// not a map — almost always a missing `map` keyword or stray brace
    BlockBinding,
    /// A call with literal arguments statically violates the callee's
    /// `requires` clause — guaranteed E004 at runtime
    StaticContractViolation,
    /// General type error (mismatch, undefined, etc.)
    General,
}

/// A diagnostic produced by the type checker
#[derive(Debug, Clone)]
pub struct TypeDiagnostic {
    pub severity: Severity,
    pub kind: DiagnosticKind,
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
    /// Number of required parameters (those without defaults). Defaults to params.len().
    pub required_params: usize,
    /// Generic type parameter names (e.g., ["T", "U"] for `fn foo<T, U>`).
    /// Empty for non-generic functions.
    pub type_params: Vec<String>,
}

/// Exported type definitions from a parsed file (functions, structs, enums, aliases)
#[derive(Debug, Clone, Default)]
struct FileExports {
    functions: HashMap<String, FunctionSig>,
    structs: HashMap<String, Vec<(String, Type)>>,
    enums: HashMap<String, Vec<(String, Option<Vec<Type>>)>>,
    type_aliases: HashMap<String, Type>,
    struct_type_params: HashMap<String, Vec<String>>,
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
    /// Generic struct type parameters: struct_name -> [param_names]
    struct_type_params: HashMap<String, Vec<String>>,
    /// Builtin and stdlib function signatures
    builtin_sigs: HashMap<String, FunctionSig>,
    /// Return type of current function being checked
    current_return_type: Option<Type>,
    /// Collected return expression types during function body analysis
    collected_returns: Vec<Type>,
    /// Collected diagnostics
    diagnostics: Vec<TypeDiagnostic>,
    /// Source lines for line number lookup
    source_lines: Vec<String>,
    /// Forward-scanning cursor for line lookup (0-indexed into source_lines)
    search_after: usize,
    /// When true, warn about untyped function parameters and missing return types
    strict_lint: bool,
    /// True when an import could not be resolved — unknown-method warnings
    /// are suppressed because unseen imports make the check unreliable
    has_unresolved_import: bool,
    /// File path of the current file being checked (for resolving relative imports)
    current_file: Option<String>,
    /// Cache of already-parsed module exports (to avoid re-parsing)
    module_cache: HashMap<String, FileExports>,
    /// Set of files currently being resolved (for circular import detection)
    resolving_files: Vec<String>,
    /// Detected circular import cycles (accumulated from nested contexts)
    detected_cycles: Vec<String>,
    /// Last attributed line per `${...}` interpolation finding (snippet + ident).
    /// Suppresses the double-visit of expression statements
    /// (check_statement + infer_statement_terminal_type) while still
    /// reporting genuinely distinct later sites of the same snippet.
    js_interp_reported: HashMap<String, usize>,
    /// `requires` clauses per user function, for static contract checking
    /// at call sites with literal arguments (DD-063 Rec 9)
    function_requires: HashMap<String, Vec<crate::ast::ContractCondition>>,
}

/// Returns true if NTNT_STRICT mode is enabled.
///
/// **Deprecated:** Use `NTNT_LINT_MODE=strict` instead.
/// This function emits a one-time deprecation warning to stderr when
/// `NTNT_STRICT` is detected.
pub fn is_strict_mode() -> bool {
    use std::sync::Once;
    static DEPRECATION_WARNED: Once = Once::new();

    let is_set = std::env::var("NTNT_STRICT").map_or(false, |v| v == "1" || v == "true");
    if is_set {
        DEPRECATION_WARNED.call_once(|| {
            eprintln!("[DEPRECATED] NTNT_STRICT is deprecated. Use NTNT_LINT_MODE=strict instead.");
        });
    }
    is_set
}

/// Run the type checker in strict mode. Returns `Some(errors)` if strict mode is
/// enabled and type errors were found, `None` otherwise (either not strict, or no errors).
pub fn strict_check(ast: &Program, source: &str) -> Option<Vec<TypeDiagnostic>> {
    strict_check_with_file(ast, source, None)
}

/// Strict check with file path for cross-file import resolution.
///
/// Runs when either `NTNT_STRICT=1` (deprecated) or `NTNT_LINT_MODE=strict` is set.
pub fn strict_check_with_file(
    ast: &Program,
    source: &str,
    file_path: Option<&str>,
) -> Option<Vec<TypeDiagnostic>> {
    let lint_strict = matches!(
        crate::config::get_lint_mode(),
        crate::config::LintMode::Strict
    );
    if !is_strict_mode() && !lint_strict {
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

/// Entry point using `LintMode` enum (from `NTNT_LINT_MODE` env or CLI flags).
pub fn check_program_with_lint_mode(
    ast: &Program,
    source: &str,
    lint_mode: crate::config::LintMode,
    file_path: Option<&str>,
) -> Vec<TypeDiagnostic> {
    let strict = matches!(
        lint_mode,
        crate::config::LintMode::Warn | crate::config::LintMode::Strict
    );
    let mut diagnostics = check_program_with_options(ast, source, strict, file_path);

    // In strict mode, promote annotation warnings to errors using structured kind
    if matches!(lint_mode, crate::config::LintMode::Strict) {
        for d in &mut diagnostics {
            if d.severity == Severity::Warning
                && matches!(
                    d.kind,
                    DiagnosticKind::MissingParamAnnotation
                        | DiagnosticKind::MissingReturnAnnotation
                        | DiagnosticKind::MissingLambdaParamAnnotation
                        | DiagnosticKind::JsStyleInterpolation
                        | DiagnosticKind::UnknownMethod
                        | DiagnosticKind::BlockBinding
                        | DiagnosticKind::StaticContractViolation
                )
            {
                d.severity = Severity::Error;
            }
        }
    }

    diagnostics
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

    // Emit diagnostics for any circular imports detected during resolution.
    // Deduplicate: the same cycle may be detected from multiple nested contexts.
    let mut seen_cycles = std::collections::HashSet::new();
    for cycle_msg in std::mem::take(&mut ctx.detected_cycles) {
        if seen_cycles.insert(cycle_msg.clone()) {
            // Try to find the import line that participates in the cycle
            let line = ctx
                .source_lines
                .iter()
                .position(|l| {
                    let trimmed = l.trim();
                    trimmed.starts_with("import ")
                        && cycle_msg.lines().next().unwrap_or("").contains(
                            &std::path::Path::new(trimmed.split('"').nth(1).unwrap_or(""))
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default(),
                        )
                })
                .map(|i| i + 1)
                .unwrap_or(0);
            ctx.warning(cycle_msg, line, None);
        }
    }

    ctx.diagnostics
}

/// Convert a BinaryOp to its source-level string representation.
fn binary_op_str(op: &BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::Pow => "**",
        BinaryOp::Eq => "==",
        BinaryOp::Ne => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        BinaryOp::And => "&&",
        BinaryOp::Or => "||",
        BinaryOp::NullCoalesce => "??",
    }
}

/// Find JavaScript-style `${ident...}` interpolation candidates in a string literal.
///
/// Returns `(head_ident, full_snippet)` pairs, e.g. `("name", "${name}")` or
/// `("user", "${user.name}")`. Only sequences whose first character after `${`
/// can start an identifier (Unicode, matching NTNT identifiers) and that close
/// with `}` before the next newline are reported; `${}`, `${1}`, and `${{` are
/// skipped. Callers decide relevance by resolving the head identifier against
/// their scope/environment.
pub fn find_js_interpolation_idents(s: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let n = chars.len();
    let mut i = 0;
    while i + 1 < n {
        if chars[i].1 != '$' || chars[i + 1].1 != '{' {
            i += 1;
            continue;
        }
        let start = chars[i].0;
        let mut j = i + 2;
        // The first character after `${` must start an identifier
        if j >= n || !(chars[j].1.is_alphabetic() || chars[j].1 == '_') {
            i += 2;
            continue;
        }
        let ident_start = chars[j].0;
        while j < n && (chars[j].1.is_alphanumeric() || chars[j].1 == '_') {
            j += 1;
        }
        let ident_end = if j < n { chars[j].0 } else { s.len() };
        // Require a balanced closing `}` before the next newline, so nested
        // content like "${PATH:-${name}}" yields the full outer span instead
        // of a truncated "${PATH:-${name" snippet.
        let mut close = None;
        let mut k = j;
        let mut depth = 1usize;
        while k < n {
            match chars[k].1 {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(chars[k].0);
                        break;
                    }
                }
                '\n' => break,
                _ => {}
            }
            k += 1;
        }
        if let Some(close) = close {
            results.push((
                s[ident_start..ident_end].to_string(),
                s[start..=close].to_string(),
            ));
        }
        // Resume right after the head identifier so a nested `${...}` inside
        // the same span (e.g. "${PATH:-${name}}") is still examined.
        i = j;
    }
    results
}

/// A literal value produced by const-folding contract clauses (DD-063 Rec 9)
#[derive(Debug, Clone, PartialEq)]
enum ConstValue {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
}

impl ConstValue {
    fn render(&self) -> String {
        match self {
            ConstValue::Int(n) => n.to_string(),
            ConstValue::Float(f) => f.to_string(),
            ConstValue::Str(s) => format!("{:?}", s),
            ConstValue::Bool(b) => b.to_string(),
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            ConstValue::Int(n) => Some(*n as f64),
            ConstValue::Float(f) => Some(*f),
            _ => None,
        }
    }
}

/// Render an expression from the const-evaluable subset back to source-ish
/// text for diagnostics. Only called for clauses const_eval fully handled,
/// so every node here is renderable.
fn render_const_expr(expr: &Expression) -> String {
    use crate::ast::{BinaryOp, UnaryOp};
    match expr {
        Expression::Integer(n) => n.to_string(),
        Expression::Float(f) => f.to_string(),
        Expression::Bool(b) => b.to_string(),
        Expression::String(text) => format!("{:?}", text),
        Expression::Identifier(name) => name.clone(),
        Expression::Unary { operator, operand } => match operator {
            UnaryOp::Neg => format!("-{}", render_const_expr(operand)),
            UnaryOp::Not => format!("!{}", render_const_expr(operand)),
        },
        Expression::Binary {
            left,
            operator,
            right,
        } => {
            let op = match operator {
                BinaryOp::Add => "+",
                BinaryOp::Sub => "-",
                BinaryOp::Mul => "*",
                BinaryOp::Div => "/",
                BinaryOp::Mod => "%",
                BinaryOp::Pow => "**",
                BinaryOp::Eq => "==",
                BinaryOp::Ne => "!=",
                BinaryOp::Lt => "<",
                BinaryOp::Le => "<=",
                BinaryOp::Gt => ">",
                BinaryOp::Ge => ">=",
                BinaryOp::And => "&&",
                BinaryOp::Or => "||",
                BinaryOp::NullCoalesce => "??",
            };
            format!(
                "{} {} {}",
                render_const_expr(left),
                op,
                render_const_expr(right)
            )
        }
        _ => "<expr>".to_string(),
    }
}

/// Const-evaluate an expression over literal bindings. Deliberately minimal:
/// literals, bound identifiers, unary neg/not, `+ - *`, comparisons, and
/// logical and/or. Anything else — calls, indexing, division (integer
/// semantics live in the interpreter), null-coalescing — returns None and
/// the clause is skipped, so this can never produce a false positive.
fn const_eval(expr: &Expression, bindings: &HashMap<String, ConstValue>) -> Option<ConstValue> {
    use crate::ast::{BinaryOp, UnaryOp};

    Some(match expr {
        Expression::Integer(n) => ConstValue::Int(*n),
        Expression::Float(f) => ConstValue::Float(*f),
        Expression::Bool(b) => ConstValue::Bool(*b),
        Expression::String(text) if !text.contains("#{") => ConstValue::Str(text.clone()),
        Expression::Identifier(name) => bindings.get(name)?.clone(),
        Expression::Unary { operator, operand } => match (operator, const_eval(operand, bindings)?)
        {
            (UnaryOp::Neg, ConstValue::Int(n)) => ConstValue::Int(n.checked_neg()?),
            (UnaryOp::Neg, ConstValue::Float(f)) => ConstValue::Float(-f),
            (UnaryOp::Not, ConstValue::Bool(b)) => ConstValue::Bool(!b),
            _ => return None,
        },
        Expression::Binary {
            left,
            operator,
            right,
        } => {
            let lhs = const_eval(left, bindings)?;
            // Short-circuit ops evaluate rhs lazily like the runtime
            match operator {
                BinaryOp::And => {
                    if let ConstValue::Bool(false) = lhs {
                        return Some(ConstValue::Bool(false));
                    }
                }
                BinaryOp::Or => {
                    if let ConstValue::Bool(true) = lhs {
                        return Some(ConstValue::Bool(true));
                    }
                }
                _ => {}
            }
            let rhs = const_eval(right, bindings)?;
            match operator {
                BinaryOp::Add => match (&lhs, &rhs) {
                    (ConstValue::Int(a), ConstValue::Int(b)) => ConstValue::Int(a.checked_add(*b)?),
                    _ => ConstValue::Float(lhs.as_f64()? + rhs.as_f64()?),
                },
                BinaryOp::Sub => match (&lhs, &rhs) {
                    (ConstValue::Int(a), ConstValue::Int(b)) => ConstValue::Int(a.checked_sub(*b)?),
                    _ => ConstValue::Float(lhs.as_f64()? - rhs.as_f64()?),
                },
                BinaryOp::Mul => match (&lhs, &rhs) {
                    (ConstValue::Int(a), ConstValue::Int(b)) => ConstValue::Int(a.checked_mul(*b)?),
                    _ => ConstValue::Float(lhs.as_f64()? * rhs.as_f64()?),
                },
                BinaryOp::Eq | BinaryOp::Ne => {
                    // Int/Int compares exactly — f64 promotion would merge
                    // distinct integers above 2^53 and break the
                    // no-false-positives invariant
                    let equal = match (&lhs, &rhs) {
                        (ConstValue::Str(a), ConstValue::Str(b)) => a == b,
                        (ConstValue::Bool(a), ConstValue::Bool(b)) => a == b,
                        (ConstValue::Int(a), ConstValue::Int(b)) => a == b,
                        (ConstValue::Float(_), ConstValue::Float(_))
                        | (ConstValue::Int(_), ConstValue::Float(_))
                        | (ConstValue::Float(_), ConstValue::Int(_)) => {
                            lhs.as_f64()? == rhs.as_f64()?
                        }
                        _ => return None,
                    };
                    ConstValue::Bool(if matches!(operator, BinaryOp::Eq) {
                        equal
                    } else {
                        !equal
                    })
                }
                BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                    let ordering = match (&lhs, &rhs) {
                        // Exact for Int/Int (see Eq above)
                        (ConstValue::Int(a), ConstValue::Int(b)) => a.cmp(b),
                        (ConstValue::Float(_), ConstValue::Float(_))
                        | (ConstValue::Int(_), ConstValue::Float(_))
                        | (ConstValue::Float(_), ConstValue::Int(_)) => {
                            lhs.as_f64()?.partial_cmp(&rhs.as_f64()?)?
                        }
                        _ => return None,
                    };
                    ConstValue::Bool(match operator {
                        BinaryOp::Lt => ordering.is_lt(),
                        BinaryOp::Le => ordering.is_le(),
                        BinaryOp::Gt => ordering.is_gt(),
                        _ => ordering.is_ge(),
                    })
                }
                BinaryOp::And => match (&lhs, &rhs) {
                    (ConstValue::Bool(a), ConstValue::Bool(b)) => ConstValue::Bool(*a && *b),
                    _ => return None,
                },
                BinaryOp::Or => match (&lhs, &rhs) {
                    (ConstValue::Bool(a), ConstValue::Bool(b)) => ConstValue::Bool(*a || *b),
                    _ => return None,
                },
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// Extract a search-friendly string from an Expression AST node.
/// Returns a string that is likely unique near the expression's source location.
fn expr_search_hint(expr: &Expression) -> String {
    match expr {
        Expression::Identifier(name) => name.clone(),
        Expression::Call { function, .. } => {
            if let Expression::Identifier(name) = function.as_ref() {
                format!("{}(", name)
            } else {
                String::new()
            }
        }
        Expression::Binary { left, operator, .. } => {
            let left_hint = expr_search_hint(left);
            if left_hint.is_empty() {
                binary_op_str(operator).to_string()
            } else {
                format!("{} {}", left_hint, binary_op_str(operator))
            }
        }
        Expression::Index { object, .. } => {
            if let Expression::Identifier(name) = object.as_ref() {
                format!("{}[", name)
            } else {
                String::new()
            }
        }
        Expression::FieldAccess { object, field } => {
            if let Expression::Identifier(name) = object.as_ref() {
                format!("{}.{}", name, field)
            } else {
                field.clone()
            }
        }
        Expression::MethodCall { object, method, .. } => {
            if let Expression::Identifier(name) = object.as_ref() {
                format!("{}.{}", name, method)
            } else {
                format!(".{}", method)
            }
        }
        _ => String::new(),
    }
}

/// Generate an actionable hint for a non-Bool condition type.
fn condition_type_hint(cond_type: &Type) -> String {
    match cond_type {
        Type::Int => "Use an explicit comparison, e.g. != 0".to_string(),
        Type::Float => "Use an explicit comparison, e.g. != 0.0".to_string(),
        Type::String => "Use len(s) > 0 or s != \"\"".to_string(),
        Type::Optional(_) => "Use != None or match on Some/None".to_string(),
        Type::Array(_) => "Use len(arr) > 0 for non-empty check".to_string(),
        _ => "Add an explicit comparison that evaluates to Bool".to_string(),
    }
}

/// Generate an actionable hint for a comparison between incompatible types.
fn comparison_type_hint(left_type: &Type, right_type: &Type, left_expr: &Expression) -> String {
    // Check if the left side is a map value access (likely untyped)
    let is_map_access = matches!(
        left_expr,
        Expression::Index { .. } | Expression::FieldAccess { .. }
    );

    if is_map_access
        && (matches!(left_type, Type::Any)
            || matches!(right_type, Type::Any)
            || !left_type.is_compatible(right_type))
    {
        return format!(
            "Map values may have mixed types. Use int(value) ?? default, str(value), or another explicit conversion before comparing"
        );
    }

    match (left_type, right_type) {
        (Type::Int, Type::String) | (Type::String, Type::Int) => {
            "Convert types: use int(s) ?? default to handle parse errors, or str(n) to convert Int to String"
                .to_string()
        }
        (Type::Float, Type::String) | (Type::String, Type::Float) => {
            "Convert types: use float(s) ?? default to handle parse errors, or str(n) to convert Float to String"
                .to_string()
        }
        (Type::Int, Type::Float) | (Type::Float, Type::Int) => {
            "Convert types: use float(n) or int(f) ?? default to match types".to_string()
        }
        _ => format!(
            "Cannot compare {} with {}. Convert to the same type first",
            left_type.name(),
            right_type.name()
        ),
    }
}

impl TypeContext {
    fn new(source: &str) -> Self {
        TypeContext {
            scopes: vec![HashMap::new()],
            functions: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            type_aliases: HashMap::new(),
            struct_type_params: HashMap::new(),
            builtin_sigs: HashMap::new(),
            current_return_type: None,
            collected_returns: Vec::new(),
            diagnostics: Vec::new(),
            source_lines: source.lines().map(|l| l.to_string()).collect(),
            search_after: 0,
            strict_lint: false,
            has_unresolved_import: false,
            current_file: None,
            module_cache: HashMap::new(),
            resolving_files: Vec::new(),
            detected_cycles: Vec::new(),
            js_interp_reported: HashMap::new(),
            function_requires: HashMap::new(),
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

    /// Update a variable's type in the scope where it was originally defined.
    fn rebind(&mut self, name: &str, typ: Type) {
        if matches!(typ, Type::Any) {
            return; // Don't widen concrete types back to Any
        }
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), typ);
                return;
            }
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

    fn emit_with_kind(
        &mut self,
        severity: Severity,
        kind: DiagnosticKind,
        message: String,
        line: usize,
        hint: Option<String>,
    ) {
        self.diagnostics.push(TypeDiagnostic {
            severity,
            kind,
            message,
            line,
            column: 0,
            hint,
        });
    }

    fn emit(&mut self, severity: Severity, message: String, line: usize, hint: Option<String>) {
        self.emit_with_kind(severity, DiagnosticKind::General, message, line, hint);
    }

    fn error(&mut self, message: String, line: usize, hint: Option<String>) {
        self.emit(Severity::Error, message, line, hint);
    }

    fn warning(&mut self, message: String, line: usize, hint: Option<String>) {
        self.emit(Severity::Warning, message, line, hint);
    }

    /// Statically check a call's literal arguments against the callee's
    /// `requires` clauses (DD-063 Rec 9). Only fires when every clause input
    /// const-evaluates — anything dynamic skips silently, so there are no
    /// false positives; a hit is a guaranteed E004 at runtime.
    fn check_static_contract(&mut self, fn_name: &str, arguments: &[Expression]) {
        let Some(clauses) = self.function_requires.get(fn_name) else {
            return;
        };
        let Some(sig) = self.functions.get(fn_name) else {
            return;
        };

        // Bind parameter names to const values from literal arguments,
        // remembering declaration order so hints render deterministically
        let param_order: Vec<String> = sig.params.iter().map(|(n, _)| n.clone()).collect();
        let mut bindings: HashMap<String, ConstValue> = HashMap::new();
        for (param_name, arg) in param_order.iter().zip(arguments.iter()) {
            if let Some(value) = const_eval(arg, &HashMap::new()) {
                bindings.insert(param_name.clone(), value);
            }
        }
        if bindings.is_empty() {
            return;
        }

        let clauses = clauses.clone();
        for clause in &clauses {
            if let Some(ConstValue::Bool(false)) = const_eval(&clause.expression, &bindings) {
                let rendered: Vec<String> = param_order
                    .iter()
                    .filter_map(|name| {
                        bindings
                            .get(name)
                            .map(|value| format!("{} = {}", name, value.render()))
                    })
                    .collect();
                // Render as many leading arguments as are source-faithful
                // ("divide(10, 0") to disambiguate between multiple calls
                let mut args_prefix: Vec<String> = Vec::new();
                for arg in arguments {
                    let rendered = match arg {
                        Expression::Integer(n) => Some(n.to_string()),
                        Expression::Bool(b) => Some(b.to_string()),
                        Expression::String(text) if !text.contains("#{") => {
                            Some(format!("{:?}", text))
                        }
                        Expression::Unary {
                            operator: crate::ast::UnaryOp::Neg,
                            operand,
                        } => match operand.as_ref() {
                            Expression::Integer(n) => Some(format!("-{}", n)),
                            _ => None,
                        },
                        _ => None,
                    };
                    match rendered {
                        Some(text) => args_prefix.push(text),
                        None => break,
                    }
                }
                let args_needle = if args_prefix.is_empty() {
                    None
                } else {
                    Some(args_prefix.join(", "))
                };
                let line = self.find_line_near_call(fn_name, args_needle.as_deref());
                self.emit_with_kind(
                    Severity::Warning,
                    DiagnosticKind::StaticContractViolation,
                    format!(
                        "call to '{}' statically violates its requires clause — this always fails at runtime (E004)",
                        fn_name
                    ),
                    line,
                    Some(format!(
                        "requires {} is false with {}",
                        render_const_expr(&clause.expression),
                        rendered.join(", ")
                    )),
                );
            }
        }
    }

    /// Warn on `x.method()` when `method` is not a defined function,
    /// builtin, or callable in-scope binding — UFCS means the call can only
    /// fail at runtime (E007). Skipped for `Any` receivers (module aliases,
    /// untyped params) and whenever an import could not be resolved, since
    /// both make the check unreliable.
    fn check_unknown_method(&mut self, object: &Expression, method: &str, obj_type: &Type) {
        if matches!(obj_type, Type::Any) || self.has_unresolved_import {
            return;
        }
        // Scope bindings only count when callable: `let double = fn(x){..}`
        // suppresses, `let length = 42` does not (that call fails at runtime).
        // Any-typed bindings suppress because we can't tell.
        let callable_binding = matches!(
            self.lookup(method),
            Some(Type::Function { .. }) | Some(Type::Any)
        );
        if self.functions.contains_key(method)
            || self.builtin_sigs.contains_key(method)
            || callable_binding
        {
            return;
        }

        let mut candidates: Vec<String> = self.functions.keys().cloned().collect();
        candidates.extend(self.builtin_sigs.keys().cloned());
        let alias = crate::error::METHOD_ALIAS_HINTS
            .iter()
            .find(|(from, to)| *from == method && candidates.iter().any(|c| c == *to))
            .map(|(_, to)| to.to_string());
        let suggestion = alias.or_else(|| crate::error::find_suggestion(method, &candidates));

        let receiver = expr_search_hint(object);
        let hint = match &suggestion {
            Some(target) => format!(
                "NTNT methods resolve to free functions — try {}({})",
                target, receiver
            ),
            None => format!(
                "NTNT methods resolve to free functions — call name({}, ...) or define fn {}(...)",
                receiver, method
            ),
        };

        let line = self.find_line_near(&format!(".{}(", method));
        self.emit_with_kind(
            Severity::Warning,
            DiagnosticKind::UnknownMethod,
            format!(
                "Unknown method '{}' — no function with this name is defined or imported",
                method
            ),
            line,
            Some(hint),
        );
    }

    /// Warn when a string literal contains JavaScript-style `${ident}` and
    /// `ident` is a variable in scope. NTNT interpolation is `#{expr}`, so
    /// the `${...}` text would be output literally.
    fn check_js_style_interpolation(&mut self, s: &str) {
        if !s.contains("${") {
            return;
        }
        for (ident, snippet) in find_js_interpolation_idents(s) {
            if self.lookup(&ident).is_none() {
                continue;
            }
            // Locate by the `${ident` prefix: identifiers cannot contain
            // escape sequences, so the prefix appears verbatim in the source
            // even when the parsed snippet was decoded (e.g. holds a real tab).
            let needle = format!("${{{}", ident);
            let key = format!("{}#{}", snippet, ident);
            let line = match self.js_interp_reported.get(&key).copied() {
                // Seen before: report again only for a genuinely later site.
                // The re-visit of the same expression statement falls back to
                // an already-attributed (or earlier) line and is skipped.
                Some(last) => {
                    let l = self.find_line_near_from(&needle, last + 1);
                    if l == 0 || l <= last {
                        continue;
                    }
                    l
                }
                None => self.find_line_near(&needle),
            };
            self.js_interp_reported.insert(key, line);
            self.emit_with_kind(
                Severity::Warning,
                DiagnosticKind::JsStyleInterpolation,
                format!(
                    "String literal contains \"{}\" — NTNT interpolation is \"#{{{}}}\"; the \"${{...}}\" text will be output literally",
                    snippet, ident
                ),
                line,
                Some(format!(
                    "Use \"#{{{}}}\". If literal ${{...}} output is intended (shell/JS content), build it with concatenation (\"$\" + \"{{...}}\") or a \"\"\"template string\"\"\".",
                    ident
                )),
            );
        }
    }

    // ── Line number lookup ────────────────────────────────────────────

    /// Search for `needle` starting from current cursor position.
    /// Advances cursor on forward-match. Falls back to full-file search.
    fn find_line_near(&mut self, needle: &str) -> usize {
        // Forward search from cursor
        for i in self.search_after..self.source_lines.len() {
            if self.source_lines[i].contains(needle) {
                self.search_after = i;
                return i + 1; // 1-indexed
            }
        }
        // Fallback: search from beginning (don't advance cursor)
        for (i, line) in self.source_lines.iter().enumerate() {
            if line.contains(needle) {
                return i + 1;
            }
        }
        0
    }

    /// Find the line of a CALL to `fn_name`, skipping its definition line
    /// (`fn name(` also contains `name(`). When the first argument is a
    /// source-faithful literal, a more specific needle disambiguates between
    /// multiple calls to the same function.
    fn find_line_near_call(&mut self, fn_name: &str, first_arg: Option<&str>) -> usize {
        let call_needle = format!("{}(", fn_name);
        let def_needle = format!("fn {}", fn_name);
        let specific = first_arg.map(|arg| format!("{}({}", fn_name, arg));

        let matches_line = |line: &str| -> bool {
            if line.contains(&def_needle) {
                return false;
            }
            match &specific {
                // Tolerate a space after '(' is not needed: NTNT style puts
                // the arg immediately after, and the fallback covers the rest
                Some(needle) => line.contains(needle.as_str()) || line.contains(&call_needle),
                None => line.contains(&call_needle),
            }
        };

        // Prefer the specific needle in a first pass when available
        if let Some(needle) = &specific {
            for i in self.search_after..self.source_lines.len() {
                if self.source_lines[i].contains(needle.as_str())
                    && !self.source_lines[i].contains(&def_needle)
                {
                    self.search_after = i;
                    return i + 1;
                }
            }
        }
        for i in self.search_after..self.source_lines.len() {
            if matches_line(&self.source_lines[i]) {
                self.search_after = i;
                return i + 1;
            }
        }
        // Fallback: search from the beginning (don't advance cursor)
        for (i, line) in self.source_lines.iter().enumerate() {
            if matches_line(line) {
                return i + 1;
            }
        }
        0
    }

    /// Search for `needle` starting from a known anchor line.
    fn find_line_near_from(&mut self, needle: &str, after_line: usize) -> usize {
        let start = if after_line > 0 { after_line - 1 } else { 0 };
        for i in start..self.source_lines.len() {
            if self.source_lines[i].contains(needle) {
                self.search_after = i;
                return i + 1;
            }
        }
        self.find_line_near(needle)
    }

    // ── Type resolution ───────────────────────────────────────────────

    /// Convert AST TypeExpr to internal Type
    fn resolve_type_expr(&self, te: &TypeExpr) -> Type {
        match te {
            TypeExpr::Named(name) => match name.as_str() {
                "Int" => Type::Int,
                "Float" => Type::Float,
                "String" => Type::String,
                "Secret" => Type::Secret,
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
                    // Single uppercase letter or common type param names: keep as Named
                    // so generic unification can resolve them (e.g., T, U, V, K, V2, etc.)
                    // Multi-char all-uppercase names also treated as type params.
                    let looks_like_type_param = name.len() == 1
                        || name.chars().all(|c| c.is_uppercase() || c.is_ascii_digit());
                    if looks_like_type_param {
                        return Type::Named(name.clone());
                    }
                    // Treat other unresolved names as Any
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
        } else if b.is_compatible(a) {
            b.clone()
        } else {
            // Flatten nested unions and deduplicate
            let mut members = Vec::new();
            match a {
                Type::Union(ts) => members.extend(ts.clone()),
                _ => members.push(a.clone()),
            }
            match b {
                Type::Union(ts) => members.extend(ts.clone()),
                _ => members.push(b.clone()),
            }
            // Deduplicate: remove members that are compatible with an earlier member
            let mut deduped = Vec::new();
            for m in &members {
                if !deduped.iter().any(|d: &Type| m.is_compatible(d)) {
                    deduped.push(m.clone());
                }
            }
            match deduped.len() {
                0 => Type::Any,
                1 => deduped.into_iter().next().unwrap(),
                _ => Type::Union(deduped),
            }
        }
    }

    fn unwrap_return_otherwise_success_type(&self, expr_ty: &Type) -> Type {
        match expr_ty {
            Type::Optional(inner) => (**inner).clone(),
            Type::Generic { name, args } if name == "Result" && !args.is_empty() => args[0].clone(),
            _ => expr_ty.clone(),
        }
    }

    fn infer_null_coalesce_type(
        &mut self,
        left: &Expression,
        right: &Expression,
        left_ty: &Type,
    ) -> Type {
        let left_is_result = matches!(left_ty, Type::Generic { name, .. } if name == "Result");
        let known_variant = match left {
            Expression::EnumVariant {
                enum_name, variant, ..
            } if enum_name == "Option" || enum_name == "Result" => Some(variant.as_str()),
            Expression::Call { function, .. } => match function.as_ref() {
                Expression::Identifier(name)
                    if name == "Some" && matches!(left_ty, Type::Optional(_)) =>
                {
                    Some(name.as_str())
                }
                Expression::Identifier(name)
                    if matches!(name.as_str(), "Ok" | "Err") && left_is_result =>
                {
                    Some(name.as_str())
                }
                _ => None,
            },
            Expression::Identifier(name)
                if name == "None" && matches!(left_ty, Type::Optional(_)) =>
            {
                Some("None")
            }
            _ => None,
        };

        if let Some(variant) = known_variant {
            match variant {
                "Some" => {
                    if let Type::Optional(inner) = left_ty {
                        return (**inner).clone();
                    }
                    return Type::Any;
                }
                "Ok" => {
                    if let Type::Generic { name, args } = left_ty {
                        if name == "Result" && !args.is_empty() {
                            return args[0].clone();
                        }
                    }
                    return Type::Any;
                }
                "None" | "Err" => return self.infer_expression(right),
                _ => {}
            }
        }

        if !matches!(left_ty, Type::Optional(_) | Type::Any) && !left_is_result {
            return left_ty.clone();
        }

        let right_ty = self.infer_expression(right);
        self.infer_binary_op(&BinaryOp::NullCoalesce, left_ty, &right_ty)
    }

    fn return_otherwise_uses_value_fallback_union(&self, expr_ty: &Type) -> bool {
        matches!(expr_ty, Type::Optional(_))
            || matches!(expr_ty, Type::Generic { name, args } if name == "Result" && !args.is_empty())
    }

    fn return_otherwise_expr_may_runtime_fail(expr: &Expression) -> bool {
        match expr {
            Expression::Integer(_)
            | Expression::Float(_)
            | Expression::String(_)
            | Expression::Bool(_)
            | Expression::Unit
            | Expression::Identifier(_) => false,
            Expression::Array(items) => items
                .iter()
                .any(Self::return_otherwise_expr_may_runtime_fail),
            Expression::MapLiteral(entries) => entries.iter().any(|(k, v)| {
                Self::return_otherwise_expr_may_runtime_fail(k)
                    || Self::return_otherwise_expr_may_runtime_fail(v)
            }),
            Expression::InterpolatedString(parts) => parts.iter().any(|part| match part {
                StringPart::Literal(_) => false,
                StringPart::Expr(expr) => Self::return_otherwise_expr_may_runtime_fail(expr),
            }),
            _ => true,
        }
    }

    fn merge_return_otherwise_type(&self, expr_ty: &Type, fallback_ty: Type) -> Type {
        let success_ty = self.unwrap_return_otherwise_success_type(expr_ty);

        if self.return_otherwise_uses_value_fallback_union(expr_ty) {
            self.union_type(&success_ty, &fallback_ty)
        } else {
            // For plain expressions, the otherwise block only runs on runtime failure.
            // Keep the declared success type as the main return type and separately
            // validate the fallback branch when the expression can actually fail.
            success_ty
        }
    }

    fn check_return_otherwise_fallback_block(&mut self, otherwise_block: &Block) -> Type {
        self.push_scope();
        self.bind("err", Type::Any);
        let fallback_ty = self.check_block(otherwise_block);
        self.pop_scope();
        fallback_ty
    }

    fn infer_statement_terminal_type(&mut self, stmt: &Statement) -> Type {
        let inner = match stmt {
            Statement::Located { stmt, .. } => stmt.as_ref(),
            other => other,
        };

        match inner {
            Statement::Expression(expr) => self.infer_expression(expr),
            Statement::Return {
                value: Some(expr),
                otherwise,
            } => {
                let expr_ty = self.infer_expression(expr);
                if let Some(otherwise_block) = otherwise {
                    let fallback_ty = self.infer_block_terminal_type(otherwise_block);
                    self.merge_return_otherwise_type(&expr_ty, fallback_ty)
                } else {
                    expr_ty
                }
            }
            _ => Type::Unit,
        }
    }

    fn infer_block_terminal_type(&mut self, block: &Block) -> Type {
        block
            .statements
            .last()
            .map(|stmt| self.infer_statement_terminal_type(stmt))
            .unwrap_or(Type::Unit)
    }

    fn validate_return_otherwise_plain_fallback(
        &mut self,
        expr: &Expression,
        expr_ty: &Type,
        fallback_ty: &Type,
    ) {
        if self.return_otherwise_uses_value_fallback_union(expr_ty)
            || !Self::return_otherwise_expr_may_runtime_fail(expr)
        {
            return;
        }

        let expected = self
            .current_return_type
            .clone()
            .unwrap_or_else(|| self.unwrap_return_otherwise_success_type(expr_ty));

        if !self.compatible(fallback_ty, &expected) && !matches!(fallback_ty, Type::Any) {
            let line = self.find_line_near("otherwise");
            self.error(
                format!(
                    "Return-otherwise fallback type mismatch: expected {} but fallback returns {}",
                    expected.name(),
                    fallback_ty.name()
                ),
                line,
                Some(
                    "Use a fallback value compatible with the surrounding return type".to_string(),
                ),
            );
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
                type_params,
                contract,
                ..
            } => {
                let tp_names: Vec<String> = type_params.iter().map(|t| t.name.clone()).collect();

                if let Some(contract) = contract {
                    if !contract.requires.is_empty() {
                        self.function_requires
                            .insert(name.clone(), contract.requires.clone());
                    }
                }

                let param_types: Vec<(String, Type)> = params
                    .iter()
                    .map(|p| {
                        let typ = if let Some(ref t) = p.type_annotation {
                            self.resolve_type_expr(t)
                        } else if let Some(ref default_expr) = p.default {
                            // Infer type from default expression
                            self.infer_expression(default_expr)
                        } else {
                            Type::Any
                        };
                        (p.name.clone(), typ)
                    })
                    .collect();

                let required_params = params.iter().filter(|p| p.default.is_none()).count();

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
                        required_params,
                        type_params: tp_names,
                    },
                );
            }
            Statement::Struct {
                name,
                fields,
                type_params,
                ..
            } => {
                if name == "Secret" {
                    let line = self.find_line_near("struct Secret");
                    self.error(
                        "'Secret' is reserved for opaque std/secrets values".to_string(),
                        line,
                        Some("Choose a different struct name".to_string()),
                    );
                    return;
                }
                // Store type parameter names for generic structs (e.g., struct Pair<A, B>)
                let tp_names: Vec<String> = type_params.iter().map(|t| t.name.clone()).collect();
                if !tp_names.is_empty() {
                    self.struct_type_params.insert(name.clone(), tp_names);
                }
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
                if name == "Secret" {
                    let line = self.find_line_near("enum Secret");
                    self.error(
                        "'Secret' is reserved for opaque std/secrets values".to_string(),
                        line,
                        Some("Choose a different enum name".to_string()),
                    );
                    return;
                }
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
                if name == "Secret" {
                    let line = self.find_line_near("type Secret");
                    self.error(
                        "'Secret' is reserved for opaque std/secrets values".to_string(),
                        line,
                        Some("Choose a different type alias name".to_string()),
                    );
                    return;
                }
                // Insert a placeholder first so self-references resolve to Type::Named(name)
                // rather than Type::Any during resolution (supports recursive type aliases).
                self.type_aliases
                    .insert(name.clone(), Type::Named(name.clone()));
                let resolved = self.resolve_type_expr(target);
                self.type_aliases.insert(name.clone(), resolved);
            }
            Statement::Impl { methods, .. } => {
                for method in methods {
                    self.collect_declaration(method);
                }
            }
            Statement::Located { stmt, .. } => self.collect_declaration(stmt),
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
                otherwise,
                ..
            } => {
                // Bare-brace bindings are a silent footgun (DD-063 Rec 6):
                // `let e = {}` binds Unit and `let m = { 5 }` binds 5 — the
                // author almost always meant `map { ... }` or has a stray
                // brace. Multi-statement blocks are left alone (block
                // expressions are an intentional scoping feature), as are
                // annotated bindings — `let x: Int = { 5 }` signals intent,
                // and a wrong annotation surfaces as a type error anyway.
                if let (None, Some(Expression::Block(block))) = (type_annotation, value) {
                    let bound_to = name.as_str();
                    if block.statements.is_empty() {
                        let line = self.find_line_near(&format!("let {}", bound_to));
                        self.emit_with_kind(
                            Severity::Warning,
                            DiagnosticKind::BlockBinding,
                            format!(
                                "`let {} = {{}}` binds Unit — a bare {{}} is an empty block, not a map",
                                bound_to
                            ),
                            line,
                            Some("For an empty map, use `map {}`".to_string()),
                        );
                    } else if block.statements.len() == 1 {
                        let line = self.find_line_near(&format!("let {}", bound_to));
                        self.emit_with_kind(
                            Severity::Warning,
                            DiagnosticKind::BlockBinding,
                            format!(
                                "`let {} = {{ ... }}` binds a block's value, not a map",
                                bound_to
                            ),
                            line,
                            Some(
                                "For a map, use `map { ... }`; if the block is intentional, this single-expression form is equivalent to binding the expression directly"
                                    .to_string(),
                            ),
                        );
                    }
                }

                let inferred = value
                    .as_ref()
                    .map(|v| self.infer_expression(v))
                    .unwrap_or(Type::Any);

                // When otherwise is present, unwrap Result<T,E> -> T or Option<T> -> T
                let inferred = if let Some(otherwise_block) = otherwise {
                    // Check that the otherwise block diverges.
                    // This is an error, not a warning: non-diverging otherwise blocks
                    // always crash at runtime ("otherwise block must diverge"), so
                    // catching this at lint time prevents production outages.
                    // See Finding #76: production outage from silent runtime error.
                    if !self.block_diverges(otherwise_block) {
                        let line = self.find_line_near("otherwise");
                        self.error(
                            "otherwise block does not diverge — it must end with return, break, or continue".to_string(),
                            line,
                            Some("Add a return, break, or continue statement".to_string()),
                        );
                    }
                    // Check the otherwise block for type errors
                    self.push_scope();
                    // Bind 'err' for the otherwise block
                    self.bind("err", Type::Any);
                    self.check_block(otherwise_block);
                    self.pop_scope();

                    match &inferred {
                        Type::Optional(t) => (**t).clone(),
                        Type::Generic { name: n, args } if n == "Result" && !args.is_empty() => {
                            args[0].clone()
                        }
                        _ => inferred,
                    }
                } else {
                    inferred
                };

                if let Some(ann) = type_annotation {
                    let expected = self.resolve_type_expr(ann);
                    if !self.compatible(&inferred, &expected) {
                        let line = self.find_line_near(&format!("let {}", name));
                        let hint = if matches!(inferred, Type::Any)
                            || matches!(inferred, Type::Array(ref inner) if matches!(inner.as_ref(), Type::Any))
                        {
                            format!(
                                "The value is untyped (from map access or untyped function). Use int()/str()/float() to convert, or add type annotations to the source function. Expected {}",
                                expected.name()
                            )
                        } else {
                            format!("Expected {}", expected.name())
                        };
                        self.error(
                            format!(
                                "Type mismatch: variable '{}' declared as {} but initialized with {}",
                                name,
                                expected.name(),
                                inferred.name()
                            ),
                            line,
                            Some(hint),
                        );
                    }
                    // Strict mode: warn about Float → Int precision loss
                    if self.strict_lint
                        && matches!(expected, Type::Int)
                        && matches!(inferred, Type::Float)
                    {
                        let line = self.find_line_near(&format!("let {}", name));
                        self.warning(
                            format!(
                                "Implicit Float to Int conversion for '{}' may lose precision",
                                name
                            ),
                            line,
                            Some(
                                "Use round(), floor(), or int() for explicit conversion"
                                    .to_string(),
                            ),
                        );
                    }
                    self.bind(name, expected);
                } else if let Some(pattern) = pattern {
                    // Destructuring: bind pattern variables with inferred types
                    self.bind_pattern(pattern, &inferred);
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
                    let fn_line = self.find_line_near(&format!("fn {}", name));
                    for param in params {
                        if param.type_annotation.is_none() && param.pattern.is_none() {
                            self.emit_with_kind(
                                Severity::Warning,
                                DiagnosticKind::MissingParamAnnotation,
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
                        self.emit_with_kind(
                            Severity::Warning,
                            DiagnosticKind::MissingReturnAnnotation,
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
                    if let Some(ref pat) = param.pattern {
                        // Destructured param: only bind pattern variables, not the synthetic name
                        self.bind_pattern(pat, &typ);
                    } else {
                        self.bind(&param.name, typ);
                    }
                }

                // Set expected return type
                let prev_return = self.current_return_type.take();
                let prev_collected = std::mem::take(&mut self.collected_returns);
                let resolved_return = return_type.as_ref().map(|t| self.resolve_type_expr(t));
                self.current_return_type = resolved_return.clone();

                // Type-check contract expressions (requires/ensures)
                if let Some(contract) = contract {
                    let fn_line = self.find_line_near(&format!("fn {}", name));

                    // requires: check each expression evaluates to Bool
                    for req_clause in &contract.requires {
                        let req_type = self.infer_expression(&req_clause.expression);
                        if !self.compatible(&req_type, &Type::Bool)
                            && !matches!(req_type, Type::Any)
                        {
                            let line = if req_clause.line > 0 {
                                req_clause.line
                            } else {
                                self.find_line_near_from("requires", fn_line)
                            };
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

                        for ens_clause in &contract.ensures {
                            let ens_type = self.infer_expression(&ens_clause.expression);
                            if !self.compatible(&ens_type, &Type::Bool)
                                && !matches!(ens_type, Type::Any)
                            {
                                let line = if ens_clause.line > 0 {
                                    ens_clause.line
                                } else {
                                    self.find_line_near_from("ensures", fn_line)
                                };
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

                // Infer return type for unannotated functions
                if return_type.is_none() {
                    let mut returns = std::mem::take(&mut self.collected_returns);
                    // Include trailing expression if non-trivial
                    if !matches!(body_type, Type::Unit | Type::Any) {
                        returns.push(body_type.clone());
                    } else if returns.is_empty() {
                        returns.push(Type::Unit);
                    }
                    if !returns.is_empty() {
                        let mut unified = returns[0].clone();
                        for ret in &returns[1..] {
                            unified = self.union_type(&unified, ret);
                        }
                        if !matches!(unified, Type::Any) {
                            if let Some(sig) = self.functions.get_mut(name) {
                                sig.return_type = unified;
                            }
                        }
                    }
                } else {
                    self.collected_returns.clear();
                }

                // Verify return type if annotated
                if let Some(expected_ret) = self.current_return_type.clone() {
                    if !self.compatible(&body_type, &expected_ret)
                        && !matches!(body_type, Type::Any)
                    {
                        let line = self.find_line_near(&format!("fn {}", name));
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
                self.collected_returns = prev_collected;
                self.pop_scope();
            }

            Statement::Return { value, otherwise } => {
                let expr_ty = value
                    .as_ref()
                    .map(|expr| self.infer_expression(expr))
                    .unwrap_or(Type::Unit);

                let actual = if let Some(otherwise_block) = otherwise {
                    let fallback_ty = self.check_return_otherwise_fallback_block(otherwise_block);
                    if let Some(expr) = value.as_ref() {
                        self.validate_return_otherwise_plain_fallback(expr, &expr_ty, &fallback_ty);
                    }
                    self.merge_return_otherwise_type(&expr_ty, fallback_ty)
                } else {
                    expr_ty
                };

                self.collected_returns.push(actual.clone());
                if let Some(expected) = self.current_return_type.clone() {
                    if !self.compatible(&actual, &expected) && !matches!(actual, Type::Any) {
                        let line = self.find_line_near("return");
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

            Statement::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_type = self.infer_expression(condition);
                if !self.compatible(&cond_type, &Type::Bool) && !matches!(cond_type, Type::Any) {
                    let hint_str = expr_search_hint(condition);
                    let needle = if hint_str.is_empty() {
                        "if ".to_string()
                    } else {
                        hint_str
                    };
                    let line = self.find_line_near(&needle);
                    let hint = condition_type_hint(&cond_type);
                    self.warning(
                        format!("Condition has type {} instead of Bool", cond_type.name()),
                        line,
                        Some(hint),
                    );
                }

                // Extract narrowing facts from the condition
                let (true_facts, false_facts) = self.extract_narrowing_facts(condition);

                // Then branch: apply true_facts
                self.push_scope();
                self.apply_facts(&true_facts);
                self.check_block(then_branch);
                let then_diverges = self.block_diverges(then_branch);
                self.pop_scope();

                // Else branch: apply false_facts
                if let Some(else_b) = else_branch {
                    self.push_scope();
                    self.apply_facts(&false_facts);
                    self.check_block(else_b);
                    self.pop_scope();
                }

                // Guard clause pattern: if then-branch diverges and no else,
                // false_facts apply to everything after the if statement
                if then_diverges && else_branch.is_none() {
                    self.apply_facts(&false_facts);
                }
            }

            Statement::While { condition, body } => {
                let cond_type = self.infer_expression(condition);
                if !self.compatible(&cond_type, &Type::Bool) && !matches!(cond_type, Type::Any) {
                    let hint_str = expr_search_hint(condition);
                    let needle = if hint_str.is_empty() {
                        "while ".to_string()
                    } else {
                        hint_str
                    };
                    let line = self.find_line_near(&needle);
                    let hint = condition_type_hint(&cond_type);
                    self.warning(
                        format!(
                            "While condition has type {} instead of Bool",
                            cond_type.name()
                        ),
                        line,
                        Some(hint),
                    );
                }
                self.push_scope();
                self.check_block(body);
                self.pop_scope();
            }

            Statement::ForIn {
                variable,
                pattern,
                iterable,
                body,
            } => {
                let iter_type = self.infer_expression(iterable);
                let elem_type = match &iter_type {
                    Type::Array(inner) => (**inner).clone(),
                    Type::String => {
                        // The interpreter yields zero iterations for for..in on strings.
                        // Warn the user to use chars() instead.
                        let line = self.find_line_near(&format!("for {} in", variable));
                        self.warning(
                            "for..in on String yields zero iterations. Use chars() for character iteration.".to_string(),
                            line,
                            Some("Replace with: for c in chars(<string>) (and import { chars } from \"std/string\")".to_string()),
                        );
                        Type::String
                    }
                    Type::Map { key_type, .. } => (**key_type).clone(),
                    _ => Type::Any,
                };
                self.push_scope();
                if let Some(pat) = pattern {
                    self.bind_pattern(pat, &elem_type);
                } else {
                    self.bind(variable, elem_type);
                }
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
                wildcard,
            } => {
                self.register_import(items, source, alias.as_deref(), *wildcard);
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
                            let line = self.find_line_near("invariant");
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
            | Statement::Intent { .. }
            | Statement::Defer(_)
            | Statement::Server { .. } => {}

            Statement::Job {
                options,
                perform_params,
                perform_body,
                on_failure,
                ..
            } => {
                // Type-check option expressions
                for (_opt_name, opt_expr) in options {
                    self.infer_expression(opt_expr);
                }

                // Type-check perform body (like a function body)
                self.push_scope();
                for param in perform_params {
                    let typ = param
                        .type_annotation
                        .as_ref()
                        .map(|t| self.resolve_type_expr(t))
                        .unwrap_or(Type::Any);
                    self.bind(&param.name, typ);
                }
                for stmt in &perform_body.statements {
                    self.check_statement(stmt);
                }
                self.pop_scope();

                // Type-check on_failure body if present
                if let Some((failure_params, failure_body)) = on_failure {
                    self.push_scope();
                    for param in failure_params {
                        let typ = param
                            .type_annotation
                            .as_ref()
                            .map(|t| self.resolve_type_expr(t))
                            .unwrap_or(Type::Any);
                        self.bind(&param.name, typ);
                    }
                    for stmt in &failure_body.statements {
                        self.check_statement(stmt);
                    }
                    self.pop_scope();
                }
            }
            Statement::Located { stmt, .. } => self.check_statement(stmt),
        }
    }

    // ── Flow-sensitive narrowing ─────────────────────────────────

    /// Extract narrowing facts from a condition expression.
    /// Returns (true_branch_facts, false_branch_facts) where each fact
    /// maps a variable name to its narrowed type.
    fn extract_narrowing_facts(
        &self,
        condition: &Expression,
    ) -> (Vec<(String, Type)>, Vec<(String, Type)>) {
        match condition {
            // x == None → true: x is None, false: x is unwrapped
            Expression::Binary {
                left,
                operator: BinaryOp::Eq,
                right,
            } => {
                if let (Expression::Identifier(name), Expression::Identifier(rhs)) =
                    (left.as_ref(), right.as_ref())
                {
                    if rhs == "None" {
                        if let Some(typ) = self.lookup(name) {
                            if let Type::Optional(inner) = typ {
                                return (
                                    vec![],                                  // true: x is None (no useful narrowing)
                                    vec![(name.clone(), (**inner).clone())], // false: x is T
                                );
                            }
                        }
                    }
                }
                // Also handle None == x
                if let (Expression::Identifier(lhs), Expression::Identifier(name)) =
                    (left.as_ref(), right.as_ref())
                {
                    if lhs == "None" {
                        if let Some(typ) = self.lookup(name) {
                            if let Type::Optional(inner) = typ {
                                return (vec![], vec![(name.clone(), (**inner).clone())]);
                            }
                        }
                    }
                }
                (vec![], vec![])
            }
            // x != None → true: x is unwrapped, false: x is None
            Expression::Binary {
                left,
                operator: BinaryOp::Ne,
                right,
            } => {
                if let (Expression::Identifier(name), Expression::Identifier(rhs)) =
                    (left.as_ref(), right.as_ref())
                {
                    if rhs == "None" {
                        if let Some(typ) = self.lookup(name) {
                            if let Type::Optional(inner) = typ {
                                return (
                                    vec![(name.clone(), (**inner).clone())], // true: x is T
                                    vec![],                                  // false: x is None
                                );
                            }
                        }
                    }
                }
                // Also handle None != x
                if let (Expression::Identifier(lhs), Expression::Identifier(name)) =
                    (left.as_ref(), right.as_ref())
                {
                    if lhs == "None" {
                        if let Some(typ) = self.lookup(name) {
                            if let Type::Optional(inner) = typ {
                                return (vec![(name.clone(), (**inner).clone())], vec![]);
                            }
                        }
                    }
                }
                (vec![], vec![])
            }
            // !cond → swap true/false facts
            Expression::Unary {
                operator: UnaryOp::Not,
                operand,
            } => {
                let (true_facts, false_facts) = self.extract_narrowing_facts(operand);
                (false_facts, true_facts)
            }
            // is_some(x), is_none(x), is_ok(x), is_err(x)
            Expression::Call {
                function,
                arguments,
            } => {
                if let Expression::Identifier(fn_name) = function.as_ref() {
                    if arguments.len() == 1 {
                        if let Expression::Identifier(var_name) = &arguments[0] {
                            if let Some(typ) = self.lookup(var_name) {
                                match fn_name.as_str() {
                                    "is_some" => {
                                        if let Type::Optional(inner) = typ {
                                            return (
                                                vec![(var_name.clone(), (**inner).clone())],
                                                vec![],
                                            );
                                        }
                                    }
                                    "is_none" => {
                                        if let Type::Optional(inner) = typ {
                                            return (
                                                vec![],
                                                vec![(var_name.clone(), (**inner).clone())],
                                            );
                                        }
                                    }
                                    "is_ok" => {
                                        if let Type::Generic { name, args } = typ {
                                            if name == "Result" && !args.is_empty() {
                                                return (
                                                    vec![(var_name.clone(), args[0].clone())],
                                                    vec![],
                                                );
                                            }
                                        }
                                    }
                                    "is_err" => {
                                        if let Type::Generic { name, args } = typ {
                                            if name == "Result" && !args.is_empty() {
                                                return (
                                                    vec![],
                                                    vec![(var_name.clone(), args[0].clone())],
                                                );
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                (vec![], vec![])
            }
            _ => (vec![], vec![]),
        }
    }

    /// Apply narrowing facts to the current scope
    fn apply_facts(&mut self, facts: &[(String, Type)]) {
        for (name, typ) in facts {
            self.rebind(name, typ.clone());
        }
    }

    // ── Match exhaustiveness checking ─────────────────────────────

    /// Check whether a match expression covers all variants.
    /// Emits a warning if variants are missing and no wildcard/variable pattern exists.
    fn check_match_exhaustiveness(&mut self, scrutinee_type: &Type, arms: &[MatchArm]) {
        // Skip checking for Any/unknown types
        if matches!(scrutinee_type, Type::Any) {
            return;
        }

        // Check if any arm has a wildcard or variable pattern (catches everything)
        let has_catch_all = arms
            .iter()
            .any(|arm| matches!(arm.pattern, Pattern::Wildcard | Pattern::Variable(_)));
        if has_catch_all {
            return;
        }

        // Determine expected variants based on scrutinee type
        let expected_variants: Option<Vec<String>> = match scrutinee_type {
            Type::Optional(_) => Some(vec!["Some".to_string(), "None".to_string()]),
            Type::Generic { name, .. } if name == "Result" => {
                Some(vec!["Ok".to_string(), "Err".to_string()])
            }
            Type::Named(name) => {
                if let Some(variants) = self.enums.get(name) {
                    Some(variants.iter().map(|(v, _)| v.clone()).collect())
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(expected) = expected_variants {
            // Collect variant names from match arms
            let covered: Vec<String> = arms
                .iter()
                .filter_map(|arm| match &arm.pattern {
                    Pattern::Variant { variant, .. } => Some(variant.clone()),
                    _ => None,
                })
                .collect();

            let missing: Vec<&String> = expected.iter().filter(|v| !covered.contains(v)).collect();

            if !missing.is_empty() {
                let missing_names: Vec<_> = missing.iter().map(|s| s.as_str()).collect();
                let line = self.find_line_near("match ");
                self.warning(
                    format!(
                        "Non-exhaustive match: missing variant(s) {}",
                        missing_names.join(", ")
                    ),
                    line,
                    Some("Add the missing variants or a wildcard '_' pattern".to_string()),
                );
            }
        }
    }

    /// Determine whether a block always diverges (returns/breaks/continues)
    fn block_diverges(&self, block: &Block) -> bool {
        if block.statements.is_empty() {
            return false;
        }
        let last = block.statements.last().map(|s| match s {
            Statement::Located { stmt, .. } => stmt.as_ref(),
            other => other,
        });
        match last {
            Some(Statement::Return { .. }) => true,
            Some(Statement::Break) | Some(Statement::Continue) => true,
            Some(Statement::If {
                then_branch,
                else_branch: Some(else_branch),
                ..
            }) => self.block_diverges(then_branch) && self.block_diverges(else_branch),
            Some(Statement::Expression(Expression::Block(inner))) => self.block_diverges(inner),
            _ => false,
        }
    }

    /// Check a block and return the type of the last expression
    fn check_block(&mut self, block: &Block) -> Type {
        let mut last_type = Type::Unit;
        for stmt in &block.statements {
            self.check_statement(stmt);
            last_type = self.infer_statement_terminal_type(stmt);
        }
        last_type
    }

    // ── Expression type inference ─────────────────────────────────────

    fn infer_expression(&mut self, expr: &Expression) -> Type {
        match expr {
            Expression::Integer(_) => Type::Int,
            Expression::Float(_) => Type::Float,
            Expression::String(s) => {
                self.check_js_style_interpolation(s);
                Type::String
            }
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

                if matches!(operator, BinaryOp::NullCoalesce) {
                    return self.infer_null_coalesce_type(left, right, &left_type);
                }

                let right_type = self.infer_expression(right);

                // Validate comparison operand compatibility
                if matches!(
                    operator,
                    BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::Le
                        | BinaryOp::Gt
                        | BinaryOp::Ge
                ) {
                    if !left_type.is_compatible(&right_type) {
                        let is_eq_ne = matches!(operator, BinaryOp::Eq | BinaryOp::Ne);
                        let either_optional = matches!(&left_type, Type::Optional(_))
                            || matches!(&right_type, Type::Optional(_));

                        // For ==/!=, allow comparing with Optional (None checks are valid)
                        if !(is_eq_ne && either_optional) {
                            let op_str = binary_op_str(operator);
                            let search_hint = expr_search_hint(left);
                            let needle = if search_hint.is_empty() {
                                op_str.to_string()
                            } else {
                                format!("{} {}", search_hint, op_str)
                            };
                            let line = self.find_line_near(&needle);
                            let hint = comparison_type_hint(&left_type, &right_type, left);
                            self.warning(
                                format!(
                                    "Comparison '{}' between incompatible types {} and {}",
                                    op_str,
                                    left_type.name(),
                                    right_type.name()
                                ),
                                line,
                                Some(hint),
                            );
                        }
                    }
                }

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
                self.check_unknown_method(object, method, &obj_type);
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
                    // Generic struct field access with type param substitution
                    Type::Generic { name, args } => {
                        if let Some(struct_fields) = self.structs.get(name).cloned() {
                            if let Some(tp_names) = self.struct_type_params.get(name).cloned() {
                                let bindings: HashMap<String, Type> = tp_names
                                    .iter()
                                    .zip(args.iter())
                                    .map(|(n, t)| (n.clone(), t.clone()))
                                    .collect();
                                for (fname, ftype) in &struct_fields {
                                    if fname == field {
                                        return Self::substitute_type_params(ftype, &bindings);
                                    }
                                }
                            }
                        }
                        Type::Any
                    }
                    // Map field access: map.field is syntactic sugar for map["field"]
                    Type::Map { value_type, .. } => (**value_type).clone(),
                    _ => Type::Any,
                }
            }

            // Index expressions can yield None at runtime (out-of-bounds or
            // missing key), but the checker infers the unwrapped element type.
            // Resolved per DD-063 Rec 3 direction (b): the RUNTIME is now loud
            // on array/string out-of-bounds (E010 in strict, [WARN] in warn;
            // map missing-key stays silent by design). Inferring Option<T>
            // here is deferred: the runtime returns bare elements in-bounds,
            // so an Option<T> annotation would bless unwrap()/match-Some/
            // is_some() code that crashes on valid values — any future change
            // needs a nullable-union vs enum-Option unification DD first (see
            // plans/dd-063-scoping-notes.md PR-5 options analysis).
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

            Expression::InterpolatedString(parts) => {
                // Literal segments can still carry js-style `${...}` (e.g. "hi #{a} and ${b}")
                for part in parts {
                    if let StringPart::Literal(s) = part {
                        self.check_js_style_interpolation(s);
                    }
                }
                // In strict mode, warn when interpolating complex types
                if self.strict_lint {
                    for part in parts {
                        if let StringPart::Expr(expr) = part {
                            let expr_type = self.infer_expression(expr);
                            match &expr_type {
                                Type::Array(_) | Type::Map { .. } | Type::Function { .. } => {
                                    let hint_str = expr_search_hint(expr);
                                    let needle = if hint_str.is_empty() {
                                        "{".to_string()
                                    } else {
                                        hint_str
                                    };
                                    let line = self.find_line_near(&needle);
                                    self.warning(
                                        format!(
                                            "Interpolating {} value may not produce useful output",
                                            expr_type.name()
                                        ),
                                        line,
                                        Some(
                                            "Consider using str() or stringify() to convert"
                                                .to_string(),
                                        ),
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Type::String
            }
            Expression::TemplateString(_) => Type::String,

            Expression::StructLiteral { name, fields } => {
                // Check field types match struct definition
                if let Some(struct_fields) = self.structs.get(name).cloned() {
                    let tp_names = self
                        .struct_type_params
                        .get(name)
                        .cloned()
                        .unwrap_or_default();
                    // For generic structs, collect type param bindings from field values
                    let mut bindings: HashMap<String, Type> = HashMap::new();
                    for (fname, fexpr) in fields {
                        let actual = self.infer_expression(fexpr);
                        if let Some((_, expected)) = struct_fields.iter().find(|(n, _)| n == fname)
                        {
                            // If expected type is a generic type param, record/check the binding
                            if let Type::Named(tp_name) = expected {
                                if tp_names.contains(tp_name) {
                                    if let Some(bound) = bindings.get(tp_name) {
                                        // A binding for this type param already exists;
                                        // ensure it is compatible (e.g., struct Pair<A> { a: A, b: A }
                                        // with { a: 1, b: "x" } should error)
                                        if !self.compatible(&actual, bound)
                                            && !matches!(actual, Type::Any)
                                            && !matches!(bound, Type::Any)
                                        {
                                            let line = self.find_line_near(name);
                                            self.error(
                                                format!(
                                                    "In struct '{}', generic type parameter '{}' has incompatible bindings: {} and {}",
                                                    name,
                                                    tp_name,
                                                    bound.name(),
                                                    actual.name()
                                                ),
                                                line,
                                                None,
                                            );
                                        }
                                    } else {
                                        bindings.insert(tp_name.clone(), actual.clone());
                                    }
                                    // Skip further field-vs-struct compatibility check
                                    continue;
                                }
                            }
                            if !self.compatible(&actual, expected) && !matches!(actual, Type::Any) {
                                let line = self.find_line_near(name);
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
                    // Return Generic type with resolved type args so field access can infer types
                    if !tp_names.is_empty() {
                        let args: Vec<Type> = tp_names
                            .iter()
                            .map(|n| bindings.get(n).cloned().unwrap_or(Type::Any))
                            .collect();
                        return Type::Generic {
                            name: name.clone(),
                            args,
                        };
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
                                let line =
                                    self.find_line_near(&format!("{}::{}", enum_name, variant));
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
                                        let line = self
                                            .find_line_near(&format!("{}::{}", enum_name, variant));
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
                    ("Option", "Some") => {
                        if let Some(first) = arguments.first() {
                            let inner = self.infer_expression(first);
                            if let Type::Optional(_) = &inner {
                                let line = self.find_line_near("Some(");
                                self.warning(
                                    format!(
                                        "Wrapping Optional value in Some() creates double-wrapped Optional<{}>. \
                                         Did you mean to assign directly?",
                                        inner.name()
                                    ),
                                    line,
                                    Some(
                                        "Remove the Some() wrapper if the value is already Optional"
                                            .to_string(),
                                    ),
                                );
                            }
                            Type::Optional(Box::new(inner))
                        } else {
                            Type::Optional(Box::new(Type::Any))
                        }
                    }
                    ("Option", "None") => Type::Optional(Box::new(Type::Any)),
                    ("Result", "Ok") => {
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
                    ("Result", "Err") => {
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

                // Strict lint: warn about untyped lambda parameters
                if self.strict_lint {
                    for param in params {
                        if param.type_annotation.is_none() && param.pattern.is_none() {
                            let line = self.find_line_near(&param.name);
                            self.emit_with_kind(
                                Severity::Warning,
                                DiagnosticKind::MissingLambdaParamAnnotation,
                                format!("Lambda parameter '{}' has no type annotation", param.name),
                                line,
                                Some(format!("Add a type: {}: Type", param.name)),
                            );
                        }
                    }
                }

                // Save and reset collected_returns for lambda scope
                let prev_return = self.current_return_type.take();
                let prev_collected = std::mem::take(&mut self.collected_returns);
                let param_types: Vec<Type> = params
                    .iter()
                    .map(|p| {
                        let typ = p
                            .type_annotation
                            .as_ref()
                            .map(|t| self.resolve_type_expr(t))
                            .unwrap_or(Type::Any);
                        self.bind(&p.name, typ.clone());
                        if let Some(ref pat) = p.pattern {
                            self.bind_pattern(pat, &typ);
                        }
                        typ
                    })
                    .collect();
                let body_type = self.check_block(body);

                // Compute return type from collected returns + trailing expression
                let mut returns = std::mem::take(&mut self.collected_returns);
                if !matches!(body_type, Type::Unit | Type::Any) {
                    returns.push(body_type.clone());
                } else if returns.is_empty() {
                    returns.push(body_type.clone());
                }
                let ret = if returns.is_empty() {
                    Type::Unit
                } else {
                    let mut unified = returns[0].clone();
                    for r in &returns[1..] {
                        unified = self.union_type(&unified, r);
                    }
                    unified
                };

                // Restore outer function's collected_returns
                self.current_return_type = prev_return;
                self.collected_returns = prev_collected;
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

                // Apply narrowing facts to if-expression branches
                let (true_facts, false_facts) = self.extract_narrowing_facts(condition);

                self.push_scope();
                self.apply_facts(&true_facts);
                let then_type = self.infer_expression(then_branch);
                self.pop_scope();

                self.push_scope();
                self.apply_facts(&false_facts);
                let else_type = self.infer_expression(else_branch);
                self.pop_scope();

                self.union_type(&then_type, &else_type)
            }

            Expression::Match { scrutinee, arms } => {
                let scrutinee_type = self.infer_expression(scrutinee);

                // Check match exhaustiveness before processing arms
                self.check_match_exhaustiveness(&scrutinee_type, arms);

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
                let value_type = self.infer_expression(value);
                if let Expression::Identifier(name) = target.as_ref() {
                    self.rebind(name, value_type);
                }
                Type::Unit
            }

            Expression::TryCatch { body } => {
                self.push_scope();
                let body_type = self.check_block(body);
                self.pop_scope();
                // Result<body_type, String>
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![body_type, Type::String],
                }
            }

            Expression::Await(inner) => self.infer_expression(inner),
            Expression::Try(inner) => {
                let try_hint = expr_search_hint(inner);
                let try_needle = if try_hint.is_empty() {
                    "?".to_string()
                } else {
                    format!("{}?", try_hint)
                };
                let inner_type = self.infer_expression(inner);
                match &inner_type {
                    Type::Optional(t) => {
                        let unwrapped = (**t).clone();
                        // Validate enclosing function returns Optional (or Any/untyped)
                        if let Some(ret) = self.current_return_type.clone() {
                            if !matches!(ret, Type::Optional(_) | Type::Any) {
                                let line = self.find_line_near(&try_needle);
                                self.warning(
                                    format!(
                                        "? on Optional requires enclosing function to return Optional, but returns {}",
                                        ret.name()
                                    ),
                                    line,
                                    Some("The ? operator early-returns None, which is incompatible with the declared return type".to_string()),
                                );
                            }
                        }
                        unwrapped
                    }
                    Type::Generic { name, args } if name == "Result" && !args.is_empty() => {
                        let ok_type = args[0].clone();
                        // Validate enclosing function returns Result (or Any/untyped)
                        if let Some(ret) = self.current_return_type.clone() {
                            let is_result =
                                matches!(ret, Type::Generic { ref name, .. } if name == "Result");
                            if !is_result && !matches!(ret, Type::Any) {
                                let line = self.find_line_near(&try_needle);
                                self.warning(
                                    format!(
                                        "? on Result requires enclosing function to return Result, but returns {}",
                                        ret.name()
                                    ),
                                    line,
                                    Some("The ? operator early-returns Err, which is incompatible with the declared return type".to_string()),
                                );
                            }
                        }
                        ok_type
                    }
                    _ => inner_type, // gradual typing: pass through for Any/unknown
                }
            }
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
                // a ?? b: unwrap Option<T>/Result<T, E> on success, otherwise use b
                match left {
                    Type::Optional(inner) => self.union_type(inner, right),
                    Type::Generic { name, args } if name == "Result" && !args.is_empty() => {
                        self.union_type(&args[0], right)
                    }
                    _ => left.clone(),
                }
            }
        }
    }

    /// Infer a lambda expression with expected parameter types from the call context.
    /// Untyped parameters are filled in from `expected_param_types`; explicit annotations win.
    fn infer_lambda_with_expected(
        &mut self,
        params: &[Parameter],
        body: &Block,
        expected_param_types: &[Type],
    ) -> Type {
        self.push_scope();
        let prev_return = self.current_return_type.take();
        let prev_collected = std::mem::take(&mut self.collected_returns);

        let param_types: Vec<Type> = params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let typ = if let Some(t) = &p.type_annotation {
                    self.resolve_type_expr(t) // Explicit annotation always wins
                } else if i < expected_param_types.len()
                    && !matches!(expected_param_types[i], Type::Any)
                {
                    expected_param_types[i].clone() // Inferred from context
                } else {
                    Type::Any // Gradual fallback
                };
                self.bind(&p.name, typ.clone());
                typ
            })
            .collect();

        let body_type = self.check_block(body);

        // Compute return type from collected returns + trailing expression
        let mut returns = std::mem::take(&mut self.collected_returns);
        if !matches!(body_type, Type::Unit | Type::Any) {
            returns.push(body_type.clone());
        } else if returns.is_empty() {
            returns.push(body_type.clone());
        }
        let ret = if returns.is_empty() {
            Type::Unit
        } else {
            let mut unified = returns[0].clone();
            for r in &returns[1..] {
                unified = self.union_type(&unified, r);
            }
            unified
        };

        self.current_return_type = prev_return;
        self.collected_returns = prev_collected;
        self.pop_scope();
        Type::Function {
            params: param_types,
            return_type: Box::new(ret),
        }
    }

    /// Determine expected parameter types for a callback argument based on the
    /// function being called and the already-inferred preceding argument types.
    fn get_callback_expected_types(
        &self,
        func_name: &str,
        preceding_arg_types: &[Type],
        callback_arg_index: usize,
    ) -> Option<Vec<Type>> {
        match func_name {
            // filter, find, any, all, each: fn(T) -> Bool
            "filter" | "find" | "any" | "all" | "each"
                if callback_arg_index == 1 && !preceding_arg_types.is_empty() =>
            {
                if let Type::Array(elem) = &preceding_arg_types[0] {
                    Some(vec![(**elem).clone()])
                } else {
                    None
                }
            }
            // transform(Array<T>, fn(T) -> R) -> Array<R>
            "transform" if callback_arg_index == 1 && !preceding_arg_types.is_empty() => {
                if let Type::Array(elem) = &preceding_arg_types[0] {
                    Some(vec![(**elem).clone()])
                } else {
                    None
                }
            }
            // sort_by(Array<T>, fn(T, T) -> Int) -> Array<T>
            "sort_by" if callback_arg_index == 1 && !preceding_arg_types.is_empty() => {
                if let Type::Array(elem) = &preceding_arg_types[0] {
                    Some(vec![(**elem).clone(), (**elem).clone()])
                } else {
                    None
                }
            }
            // reduce(Array<T>, init: U, fn(U, T) -> U) -> U
            "reduce" if callback_arg_index == 2 && preceding_arg_types.len() >= 2 => {
                if let Type::Array(elem) = &preceding_arg_types[0] {
                    Some(vec![preceding_arg_types[1].clone(), (**elem).clone()])
                } else {
                    None
                }
            }
            // User-defined function with Function parameter type
            _ => {
                if let Some(sig) = self
                    .functions
                    .get(func_name)
                    .or_else(|| self.builtin_sigs.get(func_name))
                {
                    if callback_arg_index < sig.params.len() {
                        if let Type::Function { params, .. } = &sig.params[callback_arg_index].1 {
                            return Some(params.clone());
                        }
                    }
                }
                None
            }
        }
    }

    /// Infer the return type of a function call
    fn infer_call(&mut self, function: &Expression, arguments: &[Expression]) -> Type {
        // Get function name early for bidirectional inference
        let fn_name_for_bidir = match function {
            Expression::Identifier(name) => Some(name.clone()),
            _ => None,
        };

        // Infer argument types with bidirectional inference for lambda arguments.
        // Non-lambda args are inferred eagerly; lambdas consult the function signature
        // to fill in expected parameter types from preceding arguments.
        let mut arg_types: Vec<Type> = Vec::with_capacity(arguments.len());
        for (i, arg) in arguments.iter().enumerate() {
            if let Expression::Lambda { params, body } = arg {
                if let Some(ref name) = fn_name_for_bidir {
                    if let Some(expected) = self.get_callback_expected_types(name, &arg_types, i) {
                        arg_types.push(self.infer_lambda_with_expected(params, body, &expected));
                        continue;
                    }
                }
            }
            arg_types.push(self.infer_expression(arg));
        }

        // Get function name for lookup
        let fn_name = match function {
            Expression::Identifier(name) => Some(name.clone()),
            _ => None,
        };

        if let Some(name) = &fn_name {
            // Static contract check: literal arguments vs requires clauses
            self.check_static_contract(name, arguments);

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
                "sort" | "sort_desc" | "reverse" if arguments.len() >= 1 => {
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
                // push(Array<T>, T) -> Array<T>; narrows Array<Any> to Array<ItemType>
                "push" if arguments.len() == 2 => {
                    if let Type::Array(inner) = &arg_types[0] {
                        if matches!(inner.as_ref(), Type::Any)
                            && !matches!(&arg_types[1], Type::Any)
                        {
                            return Type::Array(Box::new(arg_types[1].clone()));
                        }
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
                "get_index" if arguments.len() >= 2 => {
                    if let Type::Array(elem_type) = &arg_types[0] {
                        return (**elem_type).clone();
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
                        if let Type::Optional(_) = first {
                            let line = self.find_line_near("Some(");
                            self.warning(
                                format!(
                                    "Wrapping Optional value in Some() creates double-wrapped Optional<{}>. \
                                     Did you mean to assign directly?",
                                    first.name()
                                ),
                                line,
                                Some(
                                    "Remove the Some() wrapper if the value is already Optional"
                                        .to_string(),
                                ),
                            );
                        }
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
                if sig.variadic {
                    // Variadic: check minimum argument count
                    if arg_types.len() < sig.required_params {
                        let line = self.find_line_near(&format!("{}(", name));
                        self.error(
                            format!(
                                "Function '{}' expects at least {} argument(s), got {}",
                                name,
                                sig.required_params,
                                arg_types.len()
                            ),
                            line,
                            None,
                        );
                        return sig.return_type;
                    }
                } else if arg_types.len() < sig.required_params
                    || arg_types.len() > sig.params.len()
                {
                    let line = self.find_line_near(&format!("{}(", name));
                    let expected = if sig.required_params == sig.params.len() {
                        format!("{}", sig.params.len())
                    } else {
                        format!("{} to {}", sig.required_params, sig.params.len())
                    };
                    self.error(
                        format!(
                            "Function '{}' expects {} argument(s), got {}",
                            name,
                            expected,
                            arg_types.len()
                        ),
                        line,
                        None,
                    );
                    return sig.return_type;
                }

                // Check argument types for declared parameters (always, including variadic)
                {
                    let check_count = std::cmp::min(arg_types.len(), sig.params.len());
                    for (i, (arg_type, (param_name, param_type))) in arg_types[..check_count]
                        .iter()
                        .zip(sig.params.iter())
                        .enumerate()
                    {
                        // Skip type-checking for generic type params — they accept any type.
                        let is_type_param =
                            matches!(param_type, Type::Named(n) if sig.type_params.contains(n));
                        if is_type_param {
                            continue;
                        }
                        if !self.compatible(arg_type, param_type)
                            && !matches!(arg_type, Type::Any)
                            && !matches!(param_type, Type::Any)
                        {
                            let line = self.find_line_near(&format!("{}(", name));
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

                // Generic type unification: if the function has type params,
                // infer T from the concrete argument types and substitute into return type.
                if !sig.type_params.is_empty() {
                    let (bindings, conflicts) =
                        Self::unify_type_params(&sig.type_params, &sig.params, &arg_types);
                    // Emit errors for conflicting type param bindings
                    // e.g., fn f<T>(a: T, b: T) called with (Int, String)
                    for (param_name, first_type, second_type) in &conflicts {
                        let line = self.find_line_near(&format!("{}(", name));
                        self.error(
                            format!(
                                "Type parameter '{}' in '{}': conflicting types {} and {}",
                                param_name,
                                name,
                                first_type.name(),
                                second_type.name()
                            ),
                            line,
                            Some(format!(
                                "All arguments for '{}' must have the same type",
                                param_name
                            )),
                        );
                    }
                    if !bindings.is_empty() {
                        return Self::substitute_type_params(&sig.return_type, &bindings);
                    }
                }

                return sig.return_type;
            }
        }

        // Unknown function or dynamic call
        Type::Any
    }

    /// Unify generic type parameters with concrete argument types.
    /// Returns a map from type param name → concrete type, and a list of
    /// conflicts (type param bound to incompatible types across arguments).
    ///
    /// Example: `fn identity<T>(x: T) -> T` called with `Int` arg → `{"T": Int}`
    /// Example: `fn f<T>(a: T, b: T)` called with `(Int, String)` → conflict on T
    fn unify_type_params(
        type_params: &[String],
        param_sigs: &[(String, Type)],
        arg_types: &[Type],
    ) -> (HashMap<String, Type>, Vec<(String, Type, Type)>) {
        let mut bindings: HashMap<String, Type> = HashMap::new();
        let mut conflicts: Vec<(String, Type, Type)> = Vec::new();
        for ((_param_name, param_type), arg_type) in param_sigs.iter().zip(arg_types.iter()) {
            Self::unify_one(
                param_type,
                arg_type,
                type_params,
                &mut bindings,
                &mut conflicts,
            );
        }
        (bindings, conflicts)
    }

    /// Recursively unify a single param type pattern with a concrete type.
    fn unify_one(
        pattern: &Type,
        concrete: &Type,
        type_params: &[String],
        bindings: &mut HashMap<String, Type>,
        conflicts: &mut Vec<(String, Type, Type)>,
    ) {
        match pattern {
            // If the pattern is a named type that's a type parameter → bind it
            Type::Any => {} // Any is already maximally general
            Type::Named(name) if type_params.contains(name) => {
                if let Some(existing) = bindings.get(name) {
                    // Check for conflict: same type param bound to different types
                    if existing != concrete
                        && !matches!(existing, Type::Any)
                        && !matches!(concrete, Type::Any)
                    {
                        conflicts.push((name.clone(), existing.clone(), concrete.clone()));
                    }
                } else {
                    bindings.insert(name.clone(), concrete.clone());
                }
            }
            // Recurse into compound types
            Type::Array(inner_pattern) => {
                if let Type::Array(inner_concrete) = concrete {
                    Self::unify_one(
                        inner_pattern,
                        inner_concrete,
                        type_params,
                        bindings,
                        conflicts,
                    );
                }
            }
            Type::Optional(inner_pattern) => {
                let inner_concrete = match concrete {
                    Type::Optional(c) => c.as_ref(),
                    other => other, // T? unified with T also binds T
                };
                Self::unify_one(
                    inner_pattern,
                    inner_concrete,
                    type_params,
                    bindings,
                    conflicts,
                );
            }
            Type::Function {
                params: fn_params,
                return_type: fn_ret,
            } => {
                if let Type::Function {
                    params: concrete_params,
                    return_type: concrete_ret,
                } = concrete
                {
                    for (fp, cp) in fn_params.iter().zip(concrete_params.iter()) {
                        Self::unify_one(fp, cp, type_params, bindings, conflicts);
                    }
                    Self::unify_one(fn_ret, concrete_ret, type_params, bindings, conflicts);
                }
            }
            _ => {} // Concrete types don't produce bindings
        }
    }

    /// Substitute resolved type parameter bindings into a type.
    ///
    /// Example: return type `T` with bindings `{"T": Int}` → `Int`
    fn substitute_type_params(ty: &Type, bindings: &HashMap<String, Type>) -> Type {
        match ty {
            Type::Named(name) => {
                if let Some(resolved) = bindings.get(name) {
                    resolved.clone()
                } else {
                    ty.clone()
                }
            }
            Type::Array(inner) => {
                Type::Array(Box::new(Self::substitute_type_params(inner, bindings)))
            }
            Type::Optional(inner) => {
                Type::Optional(Box::new(Self::substitute_type_params(inner, bindings)))
            }
            Type::Function {
                params,
                return_type,
            } => Type::Function {
                params: params
                    .iter()
                    .map(|p| Self::substitute_type_params(p, bindings))
                    .collect(),
                return_type: Box::new(Self::substitute_type_params(return_type, bindings)),
            },
            Type::Tuple(types) => Type::Tuple(
                types
                    .iter()
                    .map(|t| Self::substitute_type_params(t, bindings))
                    .collect(),
            ),
            _ => ty.clone(),
        }
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
            Pattern::Array { elements, rest } => {
                let elem_type = match scrutinee_type {
                    Type::Array(inner) => (**inner).clone(),
                    _ => Type::Any,
                };
                for p in elements {
                    self.bind_pattern(p, &elem_type);
                }
                if let Some(rest_name) = rest {
                    self.bind(rest_name, Type::Array(Box::new(elem_type)));
                }
            }
            Pattern::Map { fields, rest } => {
                let value_type = match scrutinee_type {
                    Type::Map { value_type, .. } => (**value_type).clone(),
                    _ => Type::Any,
                };
                for (_key, p) in fields {
                    self.bind_pattern(p, &value_type);
                }
                if let Some(rest_name) = rest {
                    self.bind(rest_name, scrutinee_type.clone());
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

    /// Parse a file and extract all exports (functions, structs, enums, type aliases)
    fn extract_file_exports(&mut self, file_path: &std::path::Path) -> FileExports {
        use crate::lexer::Lexer;
        use crate::parser::Parser;

        let path_str = file_path.to_string_lossy().to_string();

        // Check for circular imports — must come before the cache check because
        // Pass 1 exports are cached early to break infinite recursion, but we still
        // want to warn the user about the cycle.
        if self.resolving_files.contains(&path_str) {
            // Build cycle chain for diagnostic
            if let Some(start) = self.resolving_files.iter().position(|f| f == &path_str) {
                let mut chain: Vec<String> = self.resolving_files[start..]
                    .iter()
                    .map(|f| {
                        std::path::Path::new(f)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| f.clone())
                    })
                    .collect();
                chain.push(
                    std::path::Path::new(&path_str)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| path_str.clone()),
                );
                self.detected_cycles.push(format!(
                    "Circular import detected: {}\n  \
                     Hint: break one of these imports to resolve the cycle",
                    chain.join(" → ")
                ));
            }
            // Return cached Pass 1 exports if available, otherwise empty
            if let Some(cached) = self.module_cache.get(&path_str) {
                return cached.clone();
            }
            return FileExports::default();
        }

        // Check cache (non-circular — fully resolved from a previous import)
        if let Some(cached) = self.module_cache.get(&path_str) {
            return cached.clone();
        }

        // Read and parse
        let source_code = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(_) => return FileExports::default(),
        };

        let lexer = Lexer::new(&source_code);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = Parser::new(tokens);
        let ast = match parser.parse() {
            Ok(ast) => ast,
            Err(_) => return FileExports::default(),
        };

        // Mark as resolving (circular import protection)
        self.resolving_files.push(path_str.clone());

        // Create a temporary context for Pass 1 + Pass 2
        let mut temp_ctx = TypeContext::new(&source_code);
        temp_ctx.current_file = Some(path_str.clone());
        temp_ctx.register_builtins();
        // Share module cache, resolving files, and cycle detector to prevent infinite recursion
        temp_ctx.module_cache = std::mem::take(&mut self.module_cache);
        temp_ctx.resolving_files = std::mem::take(&mut self.resolving_files);
        temp_ctx.detected_cycles = std::mem::take(&mut self.detected_cycles);

        // Run Pass 1 on the imported file to collect declarations
        for stmt in &ast.statements {
            temp_ctx.collect_declaration(stmt);
        }

        // Cache Pass 1 exports immediately to break circular imports
        let pass1_exports = FileExports {
            functions: temp_ctx.functions.clone(),
            structs: temp_ctx.structs.clone(),
            enums: temp_ctx.enums.clone(),
            type_aliases: temp_ctx.type_aliases.clone(),
            struct_type_params: temp_ctx.struct_type_params.clone(),
        };
        temp_ctx
            .module_cache
            .insert(path_str.clone(), pass1_exports);

        // Run Pass 2 to trigger return type inference for unannotated functions
        // (diagnostics from the imported file are discarded)
        for stmt in &ast.statements {
            temp_ctx.check_statement(stmt);
        }

        // Extract all exports (now with inferred return types)
        let exports = FileExports {
            functions: temp_ctx.functions,
            structs: temp_ctx.structs,
            enums: temp_ctx.enums,
            type_aliases: temp_ctx.type_aliases,
            struct_type_params: temp_ctx.struct_type_params,
        };

        // Update cache with Pass 2 results
        temp_ctx
            .module_cache
            .insert(path_str.clone(), exports.clone());

        // Restore shared state back to self
        self.module_cache = temp_ctx.module_cache;
        self.resolving_files = temp_ctx.resolving_files;
        self.resolving_files.retain(|f| f != &path_str);
        self.detected_cycles = temp_ctx.detected_cycles;

        exports
    }

    fn register_import(
        &mut self,
        items: &[ImportItem],
        source: &str,
        alias: Option<&str>,
        wildcard: bool,
    ) {
        // If it's a module alias import, bind the module name
        if let Some(alias_name) = alias {
            self.bind(alias_name, Type::Any);
            return;
        }

        if wildcard {
            let module_sigs = get_module_signatures(source);
            if !module_sigs.is_empty() {
                for (name, sig) in module_sigs {
                    self.builtin_sigs.insert(name, sig);
                }
                return;
            }

            match self.resolve_import_path(source) {
                Some(file_path) if file_path.exists() => {
                    let exports = self.extract_file_exports(&file_path);
                    for (name, sig) in exports.functions {
                        self.builtin_sigs.insert(name, sig);
                    }
                    for (name, fields) in exports.structs {
                        self.structs.insert(name.clone(), fields);
                        self.bind(&name, Type::Named(name.clone()));
                    }
                    for (name, variants) in exports.enums {
                        self.enums.insert(name.clone(), variants);
                        self.bind(&name, Type::Named(name.clone()));
                    }
                    for (name, typ) in exports.type_aliases {
                        self.type_aliases.insert(name, typ);
                    }
                }
                // Wildcard import of something we can't see — any method
                // name might exist in it
                _ => self.has_unresolved_import = true,
            }
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
        if let Some(file_path) = self.resolve_import_path(source).filter(|p| p.exists()) {
            let exports = self.extract_file_exports(&file_path);
            for item in items {
                let local_name = item.alias.as_ref().unwrap_or(&item.name);
                if let Some(sig) = exports.functions.get(&item.name) {
                    self.builtin_sigs.insert(local_name.clone(), sig.clone());
                } else if let Some(fields) = exports.structs.get(&item.name) {
                    self.structs.insert(local_name.clone(), fields.clone());
                    // Also import generic type params if the struct has them
                    if let Some(tp) = exports.struct_type_params.get(&item.name) {
                        self.struct_type_params
                            .insert(local_name.clone(), tp.clone());
                    }
                } else if let Some(variants) = exports.enums.get(&item.name) {
                    self.enums.insert(local_name.clone(), variants.clone());
                } else if let Some(typ) = exports.type_aliases.get(&item.name) {
                    self.type_aliases.insert(local_name.clone(), typ.clone());
                } else {
                    // Not found in any export category
                    self.bind(local_name, Type::Named(item.name.clone()));
                }
            }
            return;
        }

        // Unknown module — bind all as Any
        self.has_unresolved_import = true;
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
                {
                    let params: Vec<(String, Type)> = vec![$(($pname.to_string(), $ptype)),*];
                    let required_params = params.len();
                    b.insert($name.to_string(), FunctionSig {
                        params,
                        return_type: $ret,
                        variadic: false,
                        required_params,
                        type_params: vec![],
                    });
                }
            };
            ($name:expr, [$($pname:expr => $ptype:expr),*], $ret:expr, variadic) => {
                {
                    let params: Vec<(String, Type)> = vec![$(($pname.to_string(), $ptype)),*];
                    let required_params = params.len();
                    b.insert($name.to_string(), FunctionSig {
                        params,
                        return_type: $ret,
                        variadic: true,
                        required_params,
                        type_params: vec![],
                    });
                }
            };
            ($name:expr, [$($pname:expr => $ptype:expr),*], $ret:expr, required($n:expr)) => {
                {
                    let params: Vec<(String, Type)> = vec![$(($pname.to_string(), $ptype)),*];
                    b.insert($name.to_string(), FunctionSig {
                        params,
                        return_type: $ret,
                        variadic: false,
                        required_params: $n,
                        type_params: vec![],
                    });
                }
            };
        }

        // I/O
        sig!("print", ["value" => Type::Any], Type::Unit, variadic);
        sig!("input", ["prompt" => Type::String], Type::String);

        // Conversion
        sig!("str", ["value" => Type::Any], Type::String);
        sig!("int", ["value" => Type::Any], Type::Generic { name: "Result".to_string(), args: vec![Type::Int, Type::String] });
        sig!("float", ["value" => Type::Any], Type::Generic { name: "Result".to_string(), args: vec![Type::Float, Type::String] });
        sig!("bool", ["value" => Type::Any], Type::Bool);
        sig!("type", ["value" => Type::Any], Type::String);
        sig!("typeof", ["value" => Type::Any], Type::String);

        // Collections
        // chars() is defined in std/string (not a global builtin)
        sig!("len", ["value" => Type::Any], Type::Int);
        sig!("push", ["array" => Type::Array(Box::new(Type::Any)), "item" => Type::Any], Type::Array(Box::new(Type::Any)));
        sig!("pop", ["array" => Type::Array(Box::new(Type::Any))], Type::Any);
        sig!("keys", ["map" => Type::Any], Type::Array(Box::new(Type::String)));
        sig!("values", ["map" => Type::Any], Type::Array(Box::new(Type::Any)));
        sig!("entries", ["map" => Type::Any], Type::Array(Box::new(Type::Any)));
        sig!("has_key", ["map" => Type::Any, "key" => Type::String], Type::Bool);
        sig!("get_key", ["map" => Type::Any, "key" => Type::String], Type::Any);
        sig!("get_index", ["array" => Type::Array(Box::new(Type::Any)), "index" => Type::Int], Type::Any);
        sig!(
            "sort",
            [
                "array" => Type::Array(Box::new(Type::Any)),
                "key_or_fn" => Type::Any
            ],
            Type::Array(Box::new(Type::Any)),
            required(1)
        );
        sig!("sort_desc", ["array" => Type::Array(Box::new(Type::Any))], Type::Array(Box::new(Type::Any)), variadic);
        sig!("reverse", ["value" => Type::Any], Type::Any);
        sig!("includes", ["haystack" => Type::Any, "needle" => Type::Any], Type::Bool);
        sig!("has_value", ["haystack" => Type::Any, "needle" => Type::Any], Type::Bool); // deprecated alias
        sig!("merge", ["map1" => Type::Any, "map2" => Type::Any], Type::Any);
        sig!("get_or", ["map" => Type::Any, "key" => Type::String, "default" => Type::Any], Type::Any);
        sig!("filter", ["array" => Type::Array(Box::new(Type::Any)), "predicate" => Type::Any], Type::Array(Box::new(Type::Any)));
        sig!("transform", ["array" => Type::Array(Box::new(Type::Any)), "mapper" => Type::Any], Type::Array(Box::new(Type::Any)));
        sig!("flat_map", ["array" => Type::Array(Box::new(Type::Any)), "mapper" => Type::Any], Type::Array(Box::new(Type::Any)));
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
        sig!("libs", ["dir" => Type::String], Type::Unit);
        sig!("template", ["path" => Type::String, "vars" => Type::Any], Type::String);

        // Utility
        sig!("unwrap", ["value" => Type::Any], Type::Any);
        // Runtime-global Option/Result helpers (interpreter globals; keeping
        // builtin_sigs the single source of truth for the unknown-method check)
        sig!("is_some", ["value" => Type::Any], Type::Bool);
        sig!("is_none", ["value" => Type::Any], Type::Bool);
        sig!("is_ok", ["value" => Type::Any], Type::Bool);
        sig!("is_err", ["value" => Type::Any], Type::Bool);
        sig!("unwrap_or", ["value" => Type::Any, "default" => Type::Any], Type::Any);

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
                ("body_bytes".to_string(), Type::Array(Box::new(Type::Int))),
                ("id".to_string(), Type::String),
                ("ip".to_string(), Type::String),
                ("peer_ip".to_string(), Type::String),
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
            {
                let params: Vec<(String, Type)> = vec![$(($pname.to_string(), $ptype)),*];
                let required_params = params.len();
                sigs.insert($name.to_string(), FunctionSig {
                    params,
                    return_type: $ret,
                    variadic: false,
                    required_params,
                    type_params: vec![],
                });
            }
        };
        ($name:expr, [$($pname:expr => $ptype:expr),*], $ret:expr, variadic) => {
            {
                let params: Vec<(String, Type)> = vec![$(($pname.to_string(), $ptype)),*];
                let required_params = params.len();
                sigs.insert($name.to_string(), FunctionSig {
                    params,
                    return_type: $ret,
                    variadic: true,
                    required_params,
                    type_params: vec![],
                });
            }
        };
        ($name:expr, [$($pname:expr => $ptype:expr),*], $ret:expr, required($n:expr)) => {
            {
                let params: Vec<(String, Type)> = vec![$(($pname.to_string(), $ptype)),*];
                sigs.insert($name.to_string(), FunctionSig {
                    params,
                    return_type: $ret,
                    variadic: false,
                    required_params: $n,
                    type_params: vec![],
                });
            }
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
            sig!("html_escape", ["s" => Type::String], Type::String);
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
            sig!("capture_pattern", ["s" => Type::String, "pattern" => Type::String], Type::Optional(Box::new(Type::Array(Box::new(Type::String)))));
            sig!("capture_all_pattern", ["s" => Type::String, "pattern" => Type::String], Type::Array(Box::new(Type::Array(Box::new(Type::String)))));
            sig!("capture_named_pattern", ["s" => Type::String, "pattern" => Type::String], Type::Optional(Box::new(Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::String) })));
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
            sig!("paginate", ["total_items" => Type::Int, "page" => Type::Int, "per_page" => Type::Int], Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) });
            sig!("push", ["array" => Type::Array(Box::new(Type::Any)), "item" => Type::Any], Type::Array(Box::new(Type::Any)));
            sig!("pop", ["array" => Type::Array(Box::new(Type::Any))], Type::Any);
            sig!("keys", ["map" => Type::Any], Type::Array(Box::new(Type::String)));
            sig!("values", ["map" => Type::Any], Type::Array(Box::new(Type::Any)));
            sig!("entries", ["map" => Type::Any], Type::Array(Box::new(Type::Any)));
            sig!("has_key", ["map" => Type::Any, "key" => Type::String], Type::Bool);
            sig!("get_key", ["map" => Type::Any, "key" => Type::String], Type::Any, variadic);
            sig!("get_index", ["array" => Type::Array(Box::new(Type::Any)), "index" => Type::Int], Type::Any, variadic);
            sig!("first", ["array" => Type::Array(Box::new(Type::Any))], Type::Any, variadic);
            sig!("last", ["array" => Type::Array(Box::new(Type::Any))], Type::Any, variadic);
            sig!("concat", ["a" => Type::Any, "b" => Type::Any], Type::Any);
            sig!("slice", ["array" => Type::Array(Box::new(Type::Any)), "start" => Type::Int], Type::Array(Box::new(Type::Any)), variadic);
            sig!(
                "sort",
                [
                    "array" => Type::Array(Box::new(Type::Any)),
                    "key_or_fn" => Type::Any
                ],
                Type::Array(Box::new(Type::Any)),
                required(1)
            );
            sig!(
                "sort_by",
                [
                    "array" => Type::Array(Box::new(Type::Any)),
                    "comparator" => Type::Function {
                        params: vec![Type::Any, Type::Any],
                        return_type: Box::new(Type::Int),
                    }
                ],
                Type::Array(Box::new(Type::Any))
            );
            sig!("sort_desc", ["array" => Type::Array(Box::new(Type::Any))], Type::Array(Box::new(Type::Any)), variadic);
            sig!("merge", ["map1" => Type::Any, "map2" => Type::Any], Type::Any);
            sig!("get_or", ["map" => Type::Any, "key" => Type::String, "default" => Type::Any], Type::Any);
            sig!("flat_map", ["array" => Type::Array(Box::new(Type::Any)), "mapper" => Type::Any], Type::Array(Box::new(Type::Any)));
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
        "std/kv" => {
            sig!("get_int", ["kv" => Type::Any, "key" => Type::String, "default" => Type::Int], Type::Int, required(2));
            sig!("get_float", ["kv" => Type::Any, "key" => Type::String, "default" => Type::Float], Type::Float, required(2));
            sig!("get_json", ["kv" => Type::Any, "key" => Type::String, "default" => Type::Any], Type::Any, required(2));
            sig!("get_str", ["kv" => Type::Any, "key" => Type::String, "default" => Type::String], Type::String, required(2));
            let kv_int_result = Type::Generic {
                name: "Result".to_string(),
                args: vec![Type::Int, Type::String],
            };
            sig!("incr", ["kv" => Type::Any, "key" => Type::String], kv_int_result.clone());
            sig!("decr", ["kv" => Type::Any, "key" => Type::String], kv_int_result.clone());
            sig!("incr_by", ["kv" => Type::Any, "key" => Type::String, "amount" => Type::Int], kv_int_result.clone());
            sig!("decr_by", ["kv" => Type::Any, "key" => Type::String, "amount" => Type::Int], kv_int_result);
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
            sig!("file_stat", ["path" => Type::String], Type::Generic { name: "Result".to_string(), args: vec![Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }, Type::String] });
        }
        "std/env" => {
            sig!("get_env", ["name" => Type::String], Type::Optional(Box::new(Type::String)));
            sig!("set_env", ["name" => Type::String, "value" => Type::String], Type::Unit);
            sig!("all_env", [], Type::Any);
            sig!(
                "load_env",
                ["path" => Type::String],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![Type::Unit, Type::String],
                }
            );
            sig!("args", [], Type::Array(Box::new(Type::String)));
            sig!("cwd", [], Type::String);
        }
        "std/secrets" => {
            let secret = Type::Secret;
            sig!(
                "get_secret",
                ["name" => Type::String],
                Type::Optional(Box::new(secret.clone()))
            );
            sig!("require_secret", ["name" => Type::String], secret);
        }
        "std/http" => {
            sig!("fetch", ["url_or_options" => Type::Union(vec![Type::String, Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }])], Type::Generic {
                name: "Result".to_string(),
                args: vec![Type::Named("Response".to_string()), Type::String],
            }, variadic);
            sig!("download", ["url" => Type::String, "path" => Type::String], Type::Any);
            sig!("Cache", ["ttl" => Type::Int], Type::Any);
            sig!("cache_fetch", [
                "cache" => Type::Any,
                "url_or_options" => Type::Union(vec![
                    Type::String,
                    Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    },
                ])
            ], Type::Any, variadic);
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
        "std/email" => {
            let opts_map = Type::Map {
                key_type: Box::new(Type::String),
                value_type: Box::new(Type::Any),
            };
            let send_result = Type::Generic {
                name: "Result".to_string(),
                args: vec![
                    Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::String),
                    },
                    Type::String,
                ],
            };
            sig!("configure_email", ["config" => opts_map.clone()], Type::Unit);
            sig!("send_email", ["opts" => opts_map.clone()], send_result);
            sig!(
                "send_email_batch",
                ["emails" => Type::Array(Box::new(opts_map))],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![Type::Array(Box::new(Type::Any)), Type::String],
                }
            );
        }
        "std/validate" => {
            let validator = Type::Named("Validator".to_string());
            let string_map = Type::Map {
                key_type: Box::new(Type::String),
                value_type: Box::new(Type::Any),
            };
            sig!("schema", ["rules" => string_map.clone()], Type::Named("Schema".to_string()));
            sig!(
                "validate",
                ["schema" => Type::Any, "data" => string_map],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::String),
                        },
                    ],
                }
            );
            sig!("min_value", ["bound" => Type::Any], validator.clone());
            sig!("max_value", ["bound" => Type::Any], validator.clone());
            sig!("min_length", ["bound" => Type::Int], validator.clone());
            sig!("max_length", ["bound" => Type::Int], validator.clone());
            sig!("one_of", ["options" => Type::Array(Box::new(Type::Any))], validator.clone());
            sig!("matches", ["pattern" => Type::String], validator.clone());
            sig!("default", ["value" => Type::Any], validator);
            // required/optional/string/email/url are value exports (no sigs)
        }
        "std/url" => {
            sig!("encode", ["s" => Type::String], Type::String);
            sig!("decode", ["s" => Type::String], Type::String);
            sig!("parse", ["url" => Type::String], Type::Any);
            sig!("parse_query", ["query" => Type::String], Type::Any);
            sig!("build_query", ["params" => Type::Any], Type::String);
            sig!("join_url", ["base" => Type::String, "path" => Type::String], Type::String);
            sig!("join", ["base" => Type::String, "path" => Type::String], Type::String);
            // deprecated alias
        }
        "std/net" => {
            let result_map = Type::Generic {
                name: "Result".to_string(),
                args: vec![
                    Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    },
                    Type::String,
                ],
            };
            let result_bool = Type::Generic {
                name: "Result".to_string(),
                args: vec![Type::Bool, Type::String],
            };
            let result_string = Type::Generic {
                name: "Result".to_string(),
                args: vec![Type::String, Type::String],
            };
            let result_string_array = Type::Generic {
                name: "Result".to_string(),
                args: vec![Type::Array(Box::new(Type::String)), Type::String],
            };
            let result_map_array = Type::Generic {
                name: "Result".to_string(),
                args: vec![
                    Type::Array(Box::new(Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    })),
                    Type::String,
                ],
            };
            let opts = Type::Map {
                key_type: Box::new(Type::String),
                value_type: Box::new(Type::Any),
            };

            sig!("ip_parse", ["ip_or_cidr" => Type::String], result_map.clone());
            sig!("subnet_contains", ["cidr" => Type::String, "ip_or_cidr" => Type::String], result_bool.clone());
            sig!("subnet_overlaps", ["a" => Type::String, "b" => Type::String], result_bool);
            sig!("subnet_split", ["cidr" => Type::String, "new_prefix" => Type::Int, "opts" => opts.clone()], result_string_array.clone(), required(2));
            sig!("subnet_supernet", ["cidr" => Type::String, "new_prefix" => Type::Int], result_string, required(1));
            sig!("subnet_summarize", ["cidrs" => Type::Array(Box::new(Type::String))], result_string_array.clone());
            sig!("ip_range_to_cidrs", ["start_ip" => Type::String, "end_ip" => Type::String], result_string_array.clone());
            sig!(
                "net_capabilities",
                [],
                Type::Map {
                    key_type: Box::new(Type::String),
                    value_type: Box::new(Type::Bool),
                }
            );
            sig!("ping", ["host" => Type::String, "opts" => opts.clone()], result_map.clone(), required(1));
            sig!("traceroute", ["host" => Type::String, "opts" => opts.clone()], result_map.clone(), required(1));
            sig!("tcp_connect", ["host" => Type::String, "port" => Type::Int, "opts" => opts.clone()], result_map.clone(), required(2));
            sig!("reachable", ["host" => Type::String, "opts" => opts.clone()], result_map.clone(), required(1));
            sig!("port_scan", ["host" => Type::String, "ports" => Type::Array(Box::new(Type::Int)), "opts" => opts.clone()], result_map_array.clone(), required(2));
            sig!("dns_lookup", ["name" => Type::String, "record_type" => Type::String, "opts" => opts.clone()], result_map_array, required(1));
            sig!("dns_reverse", ["ip" => Type::String, "opts" => opts.clone()], result_string_array, required(1));
            sig!("tls_info", ["host" => Type::String, "opts" => opts], result_map, required(1));
        }
        "std/path" => {
            sig!("join_path", ["parts" => Type::String], Type::String, variadic);
            sig!("join", ["parts" => Type::String], Type::String, variadic); // deprecated alias
            sig!("dirname", ["path" => Type::String], Type::String);
            sig!("basename", ["path" => Type::String], Type::String);
            sig!("extname", ["path" => Type::String], Type::String);
            sig!("is_absolute", ["path" => Type::String], Type::Bool);
        }
        "std/time" => {
            sig!("from_now", ["timestamp" => Type::Union(vec![Type::Int, Type::Float])], Type::String);
            sig!("time_ago", ["timestamp" => Type::Union(vec![Type::Int, Type::Float])], Type::String);
            sig!("now", [], Type::Any);
            sig!("now_millis", [], Type::Int);
            sig!("format", ["time" => Type::Any, "fmt" => Type::String], Type::String);
            sig!("elapsed", ["start" => Type::Any], Type::Any);
            sig!("duration", ["ms" => Type::Int], Type::Any);
            sig!("parse_datetime", ["date_str" => Type::String, "format" => Type::String], Type::Generic {
                name: "Result".to_string(),
                args: vec![Type::Int, Type::String],
            });
            sig!("before", ["timestamp1" => Type::Int, "timestamp2" => Type::Int], Type::Bool);
            sig!("after", ["timestamp1" => Type::Int, "timestamp2" => Type::Int], Type::Bool);
        }
        "std/concurrent" => {
            // Channel operations
            // channel() returns [TxChannel, RxChannel] — destructure with let [tx, rx] = channel()
            sig!("channel", [], Type::Array(Box::new(Type::Any)));
            sig!("send", ["tx" => Type::Named("TxChannel".to_string()), "value" => Type::Any], Type::Bool);
            sig!("recv", ["rx" => Type::Named("RxChannel".to_string())], Type::Any);
            sig!("recv_timeout", ["rx" => Type::Named("RxChannel".to_string()), "millis" => Type::Int], Type::Optional(Box::new(Type::Any)));
            sig!("try_recv", ["rx" => Type::Named("RxChannel".to_string())], Type::Optional(Box::new(Type::Any)));
            sig!("close", ["rx" => Type::Named("RxChannel".to_string())], Type::Bool);
            sig!("select", ["channels" => Type::Array(Box::new(Type::Named("RxChannel".to_string()))), "timeout_ms" => Type::Union(vec![Type::Int, Type::String])], Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }, required(1));
            // Task operations
            sig!("spawn", ["handler" => Type::Any], Type::Named("Task".to_string()));
            sig!("await_task", ["task" => Type::Named("Task".to_string())], Type::Generic { name: "Result".to_string(), args: vec![Type::Any, Type::String] });
            sig!("try_await", ["task" => Type::Named("Task".to_string())], Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) });
            sig!("cancel_task", ["task" => Type::Named("Task".to_string())], Type::Bool);
            // parallel/race accept Array<Function> but typed as Array<Any> — runtime validates via spawn()
            sig!("parallel", ["fns" => Type::Array(Box::new(Type::Any))], Type::Any);
            sig!("race", ["fns" => Type::Array(Box::new(Type::Any))], Type::Any);
            sig!("after", ["delay" => Type::Union(vec![Type::Int, Type::String]), "handler" => Type::Any], Type::Named("Task".to_string()));
            // Schedule operations
            sig!("schedule", ["interval" => Type::Union(vec![Type::String, Type::Int]), "handler" => Type::Any], Type::Named("Schedule".to_string()));
            sig!("cancel_schedule", ["schedule" => Type::Named("Schedule".to_string())], Type::Bool);
            // Utilities
            sig!("sleep_ms", ["ms" => Type::Int], Type::Unit);
            sig!("thread_count", [], Type::Int);
        }
        "std/jobs" => {
            sig!("configure_queue", ["opts" => Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }], Type::Generic { name: "Result".to_string(), args: vec![Type::Unit, Type::String] });
            // enqueue supports 2 forms: (job_name, args) -> Result<String, String>
            // and (batch_handle, job_name, args) -> Result<Unit, String>
            // Return type uses Any because the typechecker can't do overload resolution —
            // Union(String, Unit) would make unwrap() produce String|() which breaks
            // downstream code expecting String (e.g. job_status(unwrap(enqueue(...)))).
            sig!("enqueue", ["batch_or_job" => Type::Union(vec![Type::String, Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }]), "job_name_or_args" => Type::Union(vec![Type::String, Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }]), "args" => Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }], Type::Generic { name: "Result".to_string(), args: vec![Type::Any, Type::String] }, required(2));
            sig!("job_status", ["job_id" => Type::String], Type::Generic { name: "Result".to_string(), args: vec![Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }, Type::String] });
            sig!("cancel_job", ["job_id" => Type::String, "opts" => Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }], Type::Generic { name: "Result".to_string(), args: vec![Type::Bool, Type::String] }, required(1));
            sig!("retry_job", ["job_id" => Type::String], Type::Generic { name: "Result".to_string(), args: vec![Type::Bool, Type::String] });
            sig!("list_jobs", ["opts" => Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }], Type::Generic { name: "Result".to_string(), args: vec![Type::Array(Box::new(Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) })), Type::String] }, required(0));
            sig!("delete_jobs", ["opts" => Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }], Type::Generic { name: "Result".to_string(), args: vec![Type::Int, Type::String] });
            sig!("enqueue_at", ["job_name" => Type::String, "timestamp" => Type::Int, "args" => Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }], Type::Generic { name: "Result".to_string(), args: vec![Type::String, Type::String] });
            sig!("enqueue_in", ["job_name" => Type::String, "delay_secs" => Type::Int, "args" => Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }], Type::Generic { name: "Result".to_string(), args: vec![Type::String, Type::String] });
            sig!("work_async", ["opts" => Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }], Type::Array(Box::new(Type::Named("Task".to_string()))), required(0));
            sig!("work_jobs", ["opts" => Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }], Type::Unit, required(0));
            sig!("scale_workers", ["band_name" => Type::String, "count" => Type::Int], Type::Generic { name: "Result".to_string(), args: vec![Type::Unit, Type::String] });
            sig!(
                "worker_status",
                [],
                Type::Map {
                    key_type: Box::new(Type::String),
                    value_type: Box::new(Type::Any)
                }
            );
            sig!("pause_queue", ["queue" => Type::String], Type::Generic { name: "Result".to_string(), args: vec![Type::Unit, Type::String] });
            sig!("resume_queue", ["queue" => Type::String], Type::Generic { name: "Result".to_string(), args: vec![Type::Unit, Type::String] });
            sig!("queue_status", ["queue" => Type::String], Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) });
            sig!("assert_enqueued", ["job_name" => Type::String, "args" => Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }], Type::Generic { name: "Result".to_string(), args: vec![Type::Bool, Type::String] }, required(1));
            sig!("assert_not_enqueued", ["job_name" => Type::String], Type::Generic { name: "Result".to_string(), args: vec![Type::Bool, Type::String] });
            sig!(
                "drain_jobs",
                [],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![Type::Int, Type::String]
                }
            );
            sig!(
                "clear_jobs",
                [],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![Type::Unit, Type::String]
                }
            );
            sig!("enqueue_batch", ["job_name" => Type::String, "args" => Type::Array(Box::new(Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }))], Type::Generic { name: "Result".to_string(), args: vec![Type::Array(Box::new(Type::String)), Type::String] });
            sig!("batch", ["name" => Type::String, "opts" => Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }], Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }, required(1));
            sig!("seal", ["batch_handle" => Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }], Type::Generic { name: "Result".to_string(), args: vec![Type::Unit, Type::String] });
            sig!("batch_status", ["batch_id_or_handle" => Type::Any], Type::Generic { name: "Result".to_string(), args: vec![Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }, Type::String] });
            sig!("batch_id", [], Type::Optional(Box::new(Type::String)));
            sig!("enqueue_into", ["batch_id_or_handle" => Type::Any, "job_type" => Type::String, "args" => Type::Map { key_type: Box::new(Type::String), value_type: Box::new(Type::Any) }], Type::Generic { name: "Result".to_string(), args: vec![Type::String, Type::String] });
        }
        "std/csv" => {
            sig!("parse", ["s" => Type::String], Type::Array(Box::new(Type::Array(Box::new(Type::String)))));
            sig!("parse_csv", ["s" => Type::String], Type::Array(Box::new(Type::Array(Box::new(Type::String)))));
            sig!("parse_with_headers", ["s" => Type::String], Type::Array(Box::new(Type::Any)));
            sig!("stringify", ["data" => Type::Array(Box::new(Type::Any))], Type::String);
            sig!("stringify_with_headers", ["data" => Type::Array(Box::new(Type::Any)), "headers" => Type::Array(Box::new(Type::String))], Type::String);
        }
        "std/auth" => {
            sig!(
                "local_user",
                [
                    "identifier" => Type::String,
                    "options" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    }
                ],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::String,
                    ],
                },
                required(1)
            );
            sig!(
                "update_local_user_metadata",
                [
                    "identifier" => Type::String,
                    "metadata" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    },
                    "options" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    }
                ],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::String,
                    ],
                },
                required(2)
            );
            sig!(
                "has_group",
                [
                    "subject" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    },
                    "group_ids" => Type::Union(vec![Type::String, Type::Array(Box::new(Type::String))])
                ],
                Type::Bool
            );
            sig!(
                "bootstrap_local_user",
                [
                    "identifier" => Type::String,
                    "password" => Type::String,
                    "options" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    }
                ],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::String,
                    ],
                },
                required(2)
            );
            sig!(
                "set_local_password",
                [
                    "identifier" => Type::String,
                    "current_password" => Type::String,
                    "new_password" => Type::String,
                    "options" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    }
                ],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::String,
                    ],
                },
                required(3)
            );
            sig!(
                "begin_totp_enrollment",
                [
                    "identifier" => Type::String,
                    "options" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    }
                ],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::String,
                    ],
                },
                required(1)
            );
            sig!(
                "confirm_totp_enrollment",
                [
                    "identifier" => Type::String,
                    "code" => Type::String,
                    "options" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    }
                ],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::String,
                    ],
                },
                required(2)
            );
            sig!(
                "verify_local_totp",
                [
                    "identifier" => Type::String,
                    "code" => Type::String,
                    "options" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    }
                ],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::String,
                    ],
                },
                required(2)
            );
            sig!(
                "totp_status",
                [
                    "identifier" => Type::String,
                    "options" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    }
                ],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::String,
                    ],
                },
                required(1)
            );
            sig!(
                "reset_totp",
                [
                    "identifier" => Type::String,
                    "options" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    }
                ],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::String,
                    ],
                },
                required(1)
            );
            sig!(
                "issue_magic_link",
                [
                    "identifier" => Type::String,
                    "options" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    }
                ],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::String,
                    ],
                },
                required(1)
            );
            sig!(
                "consume_magic_link",
                ["token" => Type::String],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::String,
                    ],
                },
                required(1)
            );
            sig!(
                "magic_link_flow",
                [
                    "req" => Type::Any,
                    "options" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    }
                ],
                Type::Named("Response".to_string())
            );
            sig!(
                "issue_password_reset",
                [
                    "identifier" => Type::String,
                    "options" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    }
                ],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::String,
                    ],
                },
                required(1)
            );
            sig!(
                "consume_password_reset",
                [
                    "token" => Type::String,
                    "new_password" => Type::String,
                    "options" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    }
                ],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::String,
                    ],
                },
                required(2)
            );
            sig!(
                "verify_local_password",
                [
                    "identifier" => Type::String,
                    "password" => Type::String,
                    "options" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    }
                ],
                Type::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        Type::Map {
                            key_type: Box::new(Type::String),
                            value_type: Box::new(Type::Any),
                        },
                        Type::String,
                    ],
                },
                required(2)
            );
            sig!(
                "auth_challenge_csrf_token",
                [
                    "req" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    },
                    "kind" => Type::String
                ],
                Type::Generic {
                    name: "Option".to_string(),
                    args: vec![Type::String],
                },
                required(1)
            );
            sig!(
                "auth_challenge_csrf_field",
                [
                    "req" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    },
                    "kind" => Type::String
                ],
                Type::String,
                required(1)
            );
            sig!(
                "verify_auth_challenge_csrf",
                [
                    "req" => Type::Map {
                        key_type: Box::new(Type::String),
                        value_type: Box::new(Type::Any),
                    },
                    "token" => Type::String,
                    "kind" => Type::String
                ],
                Type::Bool,
                required(2)
            );
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
            sig!("csrf_generate", [], Type::Any);
            sig!("csrf_validate", ["token" => Type::String, "hash" => Type::String], Type::Bool);
            sig!("base64_encode", ["data" => Type::String], Type::String);
            sig!("base64_decode", ["encoded" => Type::String], Type::Any);
            sig!("base64url_encode", ["data" => Type::String], Type::String);
            sig!("base64url_decode", ["encoded" => Type::String], Type::Any);
            sig!("aes_generate_key", [], Type::String);
            sig!("aes_encrypt", ["plaintext" => Type::String, "key" => Type::String], Type::Any);
            sig!("aes_decrypt", ["ciphertext" => Type::String, "key" => Type::String], Type::Any);
            sig!("argon2_hash", ["password" => Type::String], Type::String);
            sig!("argon2_verify", ["password" => Type::String, "hash" => Type::String], Type::Bool);
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

    #[test]
    fn js_interp_detector_extracts_simple_ident() {
        let found = find_js_interpolation_idents("hello ${name}!");
        assert_eq!(found, vec![("name".to_string(), "${name}".to_string())]);
    }

    #[test]
    fn js_interp_detector_extracts_head_ident_from_complex_content() {
        let found = find_js_interpolation_idents("a ${user.name} b ${VAR:-default} c");
        assert_eq!(
            found,
            vec![
                ("user".to_string(), "${user.name}".to_string()),
                ("VAR".to_string(), "${VAR:-default}".to_string()),
            ]
        );
    }

    #[test]
    fn js_interp_detector_rejects_non_identifier_starts() {
        assert!(find_js_interpolation_idents("pos ${1} empty ${} brace ${{x}").is_empty());
    }

    #[test]
    fn js_interp_detector_requires_close_before_newline() {
        assert!(find_js_interpolation_idents("open ${name\nrest }").is_empty());
    }

    #[test]
    fn js_interp_detector_handles_multiple_occurrences() {
        let found = find_js_interpolation_idents("${a} and ${b}");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, "a");
        assert_eq!(found[1].0, "b");
    }

    #[test]
    fn js_interp_detector_finds_nested_interpolation() {
        let found = find_js_interpolation_idents("echo ${PATH:-${name}} done");
        assert_eq!(
            found,
            vec![
                ("PATH".to_string(), "${PATH:-${name}}".to_string()),
                ("name".to_string(), "${name}".to_string()),
            ]
        );
    }

    #[test]
    fn js_interp_detector_balances_plain_braces() {
        let found = find_js_interpolation_idents("v ${a{b}c} w");
        assert_eq!(found, vec![("a".to_string(), "${a{b}c}".to_string())]);
    }

    #[test]
    fn js_interp_detector_supports_unicode_identifiers() {
        let found = find_js_interpolation_idents("héllo ${naïve}!");
        assert_eq!(found, vec![("naïve".to_string(), "${naïve}".to_string())]);
    }

    #[test]
    fn js_interp_detector_ignores_plain_dollars_and_braces() {
        assert!(find_js_interpolation_idents("cost: $5 {not interp} $ {gap}").is_empty());
    }

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

    #[test]
    fn test_std_auth_local_user_signature_checks_args() {
        let errs = check_errors(
            r#"
            import { local_user } from "std/auth"
            local_user(42)
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("expected String"));
        assert!(errs[0].message.contains("got Int"));
    }

    #[test]
    fn test_std_auth_update_local_user_metadata_signature_checks_args() {
        let errs = check_errors(
            r#"
            import { update_local_user_metadata } from "std/auth"
            update_local_user_metadata("admin@example.com", "not metadata")
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("expected Map"));
        assert!(errs[0].message.contains("got String"));
    }

    #[test]
    fn test_std_auth_has_group_signature_allows_request_or_session_values() {
        let errs = check_errors(
            r#"
            import { has_group } from "std/auth"
            let req = map { "method": "GET", "path": "/admin", "headers": map {} }
            let ok = has_group(req, ["admins", "owners"])
            "#,
        );
        assert!(errs.is_empty(), "unexpected diagnostics: {errs:?}");
    }

    #[test]
    fn test_std_auth_has_group_signature_rejects_non_string_group_ids() {
        let errs = check_errors(
            r#"
            import { has_group } from "std/auth"
            let req = map { "method": "GET", "path": "/admin", "headers": map {} }
            has_group(req, ["admins", 42])
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("expected"));
        assert!(errs[0].message.contains("String"));
    }

    #[test]
    fn test_std_auth_verify_local_password_signature_checks_args() {
        let errs = check_errors(
            r#"
            import { verify_local_password } from "std/auth"
            verify_local_password(42, "pw")
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("expected String"));
        assert!(errs[0].message.contains("got Int"));
    }

    #[test]
    fn test_std_auth_bootstrap_local_user_signature_checks_args() {
        let errs = check_errors(
            r#"
            import { bootstrap_local_user } from "std/auth"
            bootstrap_local_user(42, "pw")
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("expected String"));
        assert!(errs[0].message.contains("got Int"));
    }

    #[test]
    fn test_std_auth_set_local_password_signature_checks_args() {
        let errs = check_errors(
            r#"
            import { set_local_password } from "std/auth"
            set_local_password(42, "current", "new")
            "#,
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("expected String"));
        assert!(errs[0].message.contains("got Int"));
    }

    #[test]
    fn test_std_auth_magic_link_signatures_check_args() {
        let errs = check_errors(
            r#"
            import { issue_magic_link, consume_magic_link } from "std/auth"
            issue_magic_link(42)
            consume_magic_link(123)
            "#,
        );
        assert_eq!(errs.len(), 2);
        assert!(errs
            .iter()
            .all(|err| err.message.contains("expected String")));
    }

    #[test]
    fn test_std_auth_magic_link_signatures_accept_valid_calls() {
        let errs = check_errors(
            r#"
            import { issue_magic_link, consume_magic_link } from "std/auth"
            let issued = issue_magic_link("admin@example.com", map { "identifier_kind": "email", "ttl_seconds": 900 })
            let consumed = consume_magic_link("selector.verifier")
            "#,
        );
        assert!(errs.is_empty(), "unexpected type errors: {errs:?}");
    }

    #[test]
    fn test_std_auth_password_reset_signatures_check_args() {
        let errs = check_errors(
            r#"
            import { issue_password_reset, consume_password_reset } from "std/auth"
            issue_password_reset(42)
            consume_password_reset("token", 123)
            "#,
        );
        assert_eq!(errs.len(), 2);
        assert!(errs
            .iter()
            .all(|err| err.message.contains("expected String")));
    }

    #[test]
    fn test_std_auth_password_reset_signatures_allow_options() {
        let errs = check_errors(
            r#"
            import { issue_password_reset, consume_password_reset } from "std/auth"
            let issued = issue_password_reset("admin@example.com", map { "identifier_kind": "email", "ttl_seconds": 600 })
            let consumed = consume_password_reset("selector.verifier", "new-password", map { "revoke_sessions": true })
            "#,
        );
        assert!(errs.is_empty(), "unexpected diagnostics: {errs:?}");
    }

    #[test]
    fn test_std_auth_totp_helper_signatures_check_args() {
        let errs = check_errors(
            r#"
            import { begin_totp_enrollment, confirm_totp_enrollment, verify_local_totp, totp_status, reset_totp } from "std/auth"
            begin_totp_enrollment(42)
            confirm_totp_enrollment("admin@example.com", 123456)
            verify_local_totp("admin@example.com", 123456)
            totp_status(42)
            reset_totp(42)
            "#,
        );
        assert_eq!(errs.len(), 5);
        assert!(errs
            .iter()
            .all(|err| err.message.contains("expected String")));
    }

    #[test]
    fn test_std_auth_totp_helper_signatures_allow_options() {
        let errs = check_errors(
            r#"
            import { begin_totp_enrollment, confirm_totp_enrollment, verify_local_totp, totp_status, reset_totp } from "std/auth"
            let setup = begin_totp_enrollment("admin@example.com", map { "issuer": "Admin", "label": "admin@example.com" })
            let confirmed = confirm_totp_enrollment("admin@example.com", "123456", map { "identifier_kind": "email" })
            let verified = verify_local_totp("admin@example.com", "123456", map { "identifier_kind": "email" })
            let status = totp_status("admin@example.com", map { "identifier_kind": "email" })
            let reset = reset_totp("admin@example.com", map { "identifier_kind": "email" })
            "#,
        );
        assert!(errs.is_empty(), "unexpected diagnostics: {errs:?}");
    }

    #[test]
    fn test_std_auth_challenge_csrf_signatures_check_args() {
        let errs = check_errors(
            r#"
            import { auth_challenge_csrf_field, auth_challenge_csrf_token, verify_auth_challenge_csrf } from "std/auth"
            auth_challenge_csrf_token("not request")
            auth_challenge_csrf_field(map { "method": "GET", "path": "/" }, 42)
            verify_auth_challenge_csrf(map { "method": "POST", "path": "/" }, 123)
            "#,
        );
        assert_eq!(errs.len(), 3, "expected three errors, got: {:?}", errs);
    }

    #[test]
    fn test_std_auth_challenge_csrf_signatures_allow_kind() {
        let errs = check_errors(
            r#"
            import { auth_challenge_csrf_field, auth_challenge_csrf_token, verify_auth_challenge_csrf } from "std/auth"
            let req = map { "method": "POST", "path": "/local/totp", "headers": map {} }
            let token = auth_challenge_csrf_token(req, "local.totp")
            let field = auth_challenge_csrf_field(req, "local.totp")
            let ok = verify_auth_challenge_csrf(req, "token", "local.totp")
            "#,
        );
        assert!(errs.is_empty(), "unexpected diagnostics: {errs:?}");
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

    // ── Layer 1: Return type inference ─────────────────────────────

    #[test]
    fn test_infer_return_type_string() {
        let errors = check_errors(
            r#"
            fn greet() {
                return "hello"
            }
            let s: String = greet()
            "#,
        );
        assert!(
            errors.is_empty(),
            "Inferred String return should satisfy let s: String: {:?}",
            errors
        );
    }

    #[test]
    fn test_infer_return_type_int() {
        let errors = check_errors(
            r#"
            fn get_num() {
                return 42
            }
            let n: Int = get_num()
            "#,
        );
        assert!(
            errors.is_empty(),
            "Inferred Int return should satisfy let n: Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_infer_return_type_multiple_returns() {
        let errors = check_errors(
            r#"
            fn pick(x: Bool) {
                if x {
                    return "yes"
                }
                return "no"
            }
            let s: String = pick(true)
            "#,
        );
        assert!(
            errors.is_empty(),
            "Multiple String returns should unify to String: {:?}",
            errors
        );
    }

    #[test]
    fn test_infer_return_type_option() {
        // Some + None branches should not cause errors when caller is untyped
        let errors = check_errors(
            r#"
            fn maybe(x: Bool) {
                if x {
                    return Some(1)
                }
                return None
            }
            let v = maybe(true)
            "#,
        );
        assert!(
            errors.is_empty(),
            "Some + None returns should not error: {:?}",
            errors
        );
    }

    #[test]
    fn test_inferred_return_benefits_later_callers() {
        let errors = check_errors(
            r#"
            fn make_name() {
                return "Alice"
            }
            fn use_name() {
                let name: String = make_name()
                return name
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Later function should benefit from inferred return type of earlier function: {:?}",
            errors
        );
    }

    #[test]
    fn test_annotated_function_unchanged() {
        let errors = check_errors(
            r#"
            fn add(a: Int, b: Int) -> Int {
                return a + b
            }
            let x: Int = add(1, 2)
            "#,
        );
        assert!(
            errors.is_empty(),
            "Annotated functions should still work: {:?}",
            errors
        );
    }

    #[test]
    fn test_untyped_no_false_positives() {
        let diags = check(
            r#"
            fn foo(a, b) {
                return a + b
            }
            let x = foo(1, 2)
            let y = foo("a", "b")
            "#,
        );
        assert!(
            diags.is_empty(),
            "Fully untyped code should still produce no diagnostics: {:?}",
            diags
        );
    }

    // ── Layer 2: Double-Option warning ─────────────────────────────

    #[test]
    fn test_double_option_warning() {
        let warnings = check_warnings("let x = Some(Some(42))");
        let double_opt: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("double-wrapped"))
            .collect();
        assert_eq!(
            double_opt.len(),
            1,
            "Some(Some(42)) should warn about double wrapping: {:?}",
            warnings
        );
    }

    #[test]
    fn test_single_option_no_warning() {
        let warnings = check_warnings("let x = Some(42)");
        let double_opt: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("double-wrapped"))
            .collect();
        assert!(
            double_opt.is_empty(),
            "Some(42) should not warn about double wrapping: {:?}",
            warnings
        );
    }

    #[test]
    fn test_some_none_warns() {
        let warnings = check_warnings("let x = Some(None)");
        let double_opt: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("double-wrapped"))
            .collect();
        assert_eq!(
            double_opt.len(),
            1,
            "Some(None) should warn about double wrapping: {:?}",
            warnings
        );
    }

    // ── Layer 3: Array narrowing + assignment refinement ───────────

    #[test]
    fn test_push_narrows_empty_array() {
        let errors = check_errors(
            r#"
            let items = push([], 42)
            let n: Int = first(items)
            "#,
        );
        assert!(
            errors.is_empty(),
            "push([], 42) should return Array<Int>, so first() is Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_push_preserves_existing_type() {
        let errors = check_errors(
            r#"
            let items = push([1, 2], 3)
            let n: Int = first(items)
            "#,
        );
        assert!(
            errors.is_empty(),
            "push([1,2], 3) should return Array<Int>: {:?}",
            errors
        );
    }

    #[test]
    fn test_assignment_refines_type() {
        let errors = check_errors(
            r#"
            let mut items = []
            items = push(items, "x")
            let s: String = first(items)
            "#,
        );
        assert!(
            errors.is_empty(),
            "After items = push(items, \"x\"), first(items) should be String: {:?}",
            errors
        );
    }

    // ── Phase 1: Union type correctness ─────────────────────────

    #[test]
    fn test_union_int_compatible_with_int_or_string_target() {
        // Int should be compatible with Int | String target
        let errors = check_errors(
            r#"
            let x: Int | String = 42
            "#,
        );
        assert!(
            errors.is_empty(),
            "Int should be compatible with Int | String target: {:?}",
            errors
        );
    }

    #[test]
    fn test_union_value_not_compatible_with_single_type() {
        // Union(Int, String) should NOT be compatible with Int target
        let errors = check_errors(
            r#"
            fn get_value() -> Int | String {
                return 42
            }
            let x: Int = get_value()
            "#,
        );
        assert!(
            !errors.is_empty(),
            "Int | String value should NOT be compatible with Int target"
        );
    }

    #[test]
    fn test_union_all_members_match_target() {
        // Union(Int, Int) is compatible with Int (all members match)
        // We can test this by returning Int from two branches and assigning to Int
        let errors = check_errors(
            r#"
            fn get_num(x: Bool) -> Int {
                if x { return 1 }
                return 2
            }
            let n: Int = get_num(true)
            "#,
        );
        assert!(
            errors.is_empty(),
            "Union(Int, Int) should be compatible with Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_union_bool_not_compatible_with_int_or_string() {
        let errors = check_errors(
            r#"
            let x: Int | String = true
            "#,
        );
        assert!(
            !errors.is_empty(),
            "Bool should NOT be compatible with Int | String"
        );
    }

    #[test]
    fn test_union_to_union_compatible() {
        // Int | String should be compatible with Int | String | Bool
        let errors = check_errors(
            r#"
            fn get_value() -> Int | String {
                return 42
            }
            let x: Int | String | Bool = get_value()
            "#,
        );
        assert!(
            errors.is_empty(),
            "Int | String should be compatible with Int | String | Bool: {:?}",
            errors
        );
    }

    #[test]
    fn test_union_nested_flattening() {
        use crate::types::Type;
        // Test that union_type flattens nested unions
        let ctx = TypeContext::new("");
        let a = Type::Union(vec![Type::Int, Type::String]);
        let b = Type::Bool;
        let result = ctx.union_type(&a, &b);
        match &result {
            Type::Union(members) => {
                assert_eq!(members.len(), 3, "Should flatten: {:?}", result);
            }
            _ => panic!("Expected Union, got {:?}", result),
        }
    }

    #[test]
    fn test_union_deduplication() {
        use crate::types::Type;
        // Test that union_type deduplicates
        let ctx = TypeContext::new("");
        let result = ctx.union_type(&Type::Int, &Type::Int);
        assert_eq!(result, Type::Int, "union(Int, Int) should be Int");
    }

    #[test]
    fn test_union_gradual_typing_preserved() {
        // Fully untyped code should still produce zero diagnostics
        let diags = check(
            r#"
            fn choose(x) {
                if x { return 42 }
                return "hello"
            }
            let result = choose(true)
            "#,
        );
        assert!(
            diags.is_empty(),
            "Untyped code should produce no diagnostics: {:?}",
            diags
        );
    }

    // ── Phase 2: Block divergence analysis ──────────────────────

    #[test]
    fn test_otherwise_without_return_errors() {
        // Non-diverging otherwise blocks are now errors (not warnings) since they
        // always crash at runtime — catching this at lint time prevents outages.
        let diags = check(
            r#"
            let x = Some(42) otherwise {
                let y = 1
            }
            "#,
        );
        let otherwise_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message.contains("otherwise"))
            .collect();
        assert!(
            !otherwise_diags.is_empty(),
            "otherwise block without return should produce a diagnostic: {:?}",
            diags
        );
        assert!(
            otherwise_diags
                .iter()
                .any(|d| d.severity == Severity::Error),
            "otherwise block without return should be an error: {:?}",
            otherwise_diags
        );
    }

    #[test]
    fn test_otherwise_with_return_no_warning() {
        let warnings = check_warnings(
            r#"
            fn foo() {
                let x = Some(42) otherwise {
                    return
                }
            }
            "#,
        );
        let otherwise_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("otherwise"))
            .collect();
        assert!(
            otherwise_warnings.is_empty(),
            "otherwise block with return should not warn: {:?}",
            otherwise_warnings
        );
    }

    #[test]
    fn test_otherwise_with_if_both_return_no_warning() {
        let warnings = check_warnings(
            r#"
            fn foo(debug: Bool) {
                let x = Some(42) otherwise {
                    if debug {
                        return
                    } else {
                        return
                    }
                }
            }
            "#,
        );
        let otherwise_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("otherwise"))
            .collect();
        assert!(
            otherwise_warnings.is_empty(),
            "otherwise with if/else both returning should not warn: {:?}",
            otherwise_warnings
        );
    }

    #[test]
    fn test_return_otherwise_plain_expression_keeps_plain_return_type() {
        let errors = check_errors(
            r#"
            fn plain() -> Int {
                return 42 otherwise { "fallback" }
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Plain return-otherwise should not widen to Int | String: {:?}",
            errors
        );
    }

    #[test]
    fn test_return_otherwise_fallback_diagnostics_are_not_duplicated() {
        let diags = check(
            r#"
            fn foo() -> Int {
                return Some(42) otherwise {
                    let bad: Int = "oops"
                    0
                }
            }
            "#,
        );
        let bad_binding_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message.contains("variable 'bad'"))
            .collect();
        assert_eq!(
            bad_binding_diags.len(),
            1,
            "Fallback block diagnostics should only fire once: {:?}",
            diags
        );
    }

    #[test]
    fn test_return_otherwise_runtime_fallback_must_match_return_type() {
        let errors = check_errors(
            r#"
            fn plain() -> Int {
                return 1 / 0 otherwise { "fallback" }
            }
            "#,
        );
        assert!(
            errors.iter().any(|e| e
                .message
                .contains("Return-otherwise fallback type mismatch")),
            "Runtime fallback should stay compatible with the surrounding return type: {:?}",
            errors
        );
    }

    #[test]
    fn test_lambda_with_early_return_infers_string() {
        let errors = check_errors(
            r#"
            let f = fn(x: Int) -> String {
                if x > 0 { return "positive" }
                return "non-positive"
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Lambda with early returns should infer String: {:?}",
            errors
        );
    }

    #[test]
    fn test_lambda_mixed_return_types_infers_union() {
        // Lambda with Int return and String trailing expr should produce a union
        let errors = check_errors(
            r#"
            fn test_fn() {
                let f = fn(x: Bool) {
                    if x { return 1 }
                    return "fallback"
                }
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Lambda with mixed returns in untyped context should produce no errors: {:?}",
            errors
        );
    }

    #[test]
    fn test_phase2_gradual_preserved() {
        // Gradual typing: untyped code (including unannotated lambdas) produces no type errors.
        let diags = check(
            r#"
            fn foo(a) {
                let b = fn(x) { x + 1 }
            }
            "#,
        );
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "Gradual typing should produce no type errors: {:?}",
            errors
        );
    }

    // ── Phase 3: Flow-sensitive type narrowing ──────────────────

    #[test]
    fn test_narrow_guard_clause_eq_none() {
        // if x == None { return } → x is narrowed to Int
        let errors = check_errors(
            r#"
            fn process(x: Int?) {
                if x == None { return }
                let n: Int = x
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "After guard clause 'if x == None {{ return }}', x should be narrowed to Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_narrow_ne_none_inside_body() {
        // if x != None { use x as Int }
        let errors = check_errors(
            r#"
            fn process(x: Int?) {
                if x != None {
                    let n: Int = x
                }
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Inside 'if x != None', x should be narrowed to Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_narrow_is_some() {
        let errors = check_errors(
            r#"
            fn process(x: Int?) {
                if is_some(x) {
                    let n: Int = x
                }
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Inside 'if is_some(x)', x should be narrowed to Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_narrow_is_ok() {
        let errors = check_errors(
            r#"
            fn process(result: Result<Int, String>) {
                if is_ok(result) {
                    let n: Int = result
                }
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Inside 'if is_ok(result)', result should be narrowed to Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_narrow_negation_guard() {
        // if !is_some(x) { return } → x is narrowed after
        let errors = check_errors(
            r#"
            fn process(x: Int?) {
                if !is_some(x) { return }
                let n: Int = x
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "After '!is_some(x) {{ return }}', x should be narrowed to Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_narrow_is_none_guard() {
        let errors = check_errors(
            r#"
            fn process(x: Int?) {
                if is_none(x) { return }
                let n: Int = x
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "After 'if is_none(x) {{ return }}', x should be narrowed to Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_narrow_if_expr() {
        // If-expression should also benefit from narrowing
        let errors = check_errors(
            r#"
            fn process(x: Int?) -> Int {
                return if is_some(x) { x } else { 0 }
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "If-expression should narrow x in then-branch: {:?}",
            errors
        );
    }

    #[test]
    fn test_no_narrowing_for_untyped() {
        // No narrowing for variables without type annotations (Any stays Any)
        let diags = check(
            r#"
            fn process(x) {
                if x == None { return }
                let n = x
            }
            "#,
        );
        assert!(
            diags.is_empty(),
            "Untyped variables should not trigger narrowing errors: {:?}",
            diags
        );
    }

    #[test]
    fn test_no_narrowing_for_non_optional() {
        // Non-optional types shouldn't be narrowed
        let diags = check(
            r#"
            fn process(x: Int) {
                if x == None { return }
                let n: Int = x
            }
            "#,
        );
        // No errors for already-Int variable
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "Non-optional types should not cause narrowing errors: {:?}",
            errors
        );
    }

    #[test]
    fn test_phase3_gradual_preserved() {
        let diags = check(
            r#"
            fn foo(x) {
                if x != None {
                    let y = x
                }
                if is_some(x) {
                    let z = x
                }
            }
            "#,
        );
        assert!(
            diags.is_empty(),
            "Gradual typing with narrowing patterns should produce no diagnostics: {:?}",
            diags
        );
    }

    // ── Phase 4: Match exhaustiveness ───────────────────────────

    #[test]
    fn test_match_option_missing_none_warns() {
        let warnings = check_warnings(
            r#"
            let x: Int? = Some(42)
            let y = match x {
                Some(v) => v
            }
            "#,
        );
        let exhaust_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("Non-exhaustive"))
            .collect();
        assert!(
            !exhaust_warnings.is_empty(),
            "Match on Option missing None should warn: {:?}",
            warnings
        );
        assert!(
            exhaust_warnings[0].message.contains("None"),
            "Warning should mention missing 'None': {}",
            exhaust_warnings[0].message
        );
    }

    #[test]
    fn test_match_option_both_variants_no_warning() {
        let warnings = check_warnings(
            r#"
            let x: Int? = Some(42)
            let y = match x {
                Some(v) => v,
                None => 0
            }
            "#,
        );
        let exhaust_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("Non-exhaustive"))
            .collect();
        assert!(
            exhaust_warnings.is_empty(),
            "Match with both Some and None should not warn: {:?}",
            exhaust_warnings
        );
    }

    #[test]
    fn test_match_result_missing_err_warns() {
        let warnings = check_warnings(
            r#"
            let x = Ok(42)
            let y = match x {
                Ok(v) => v
            }
            "#,
        );
        let exhaust_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("Non-exhaustive"))
            .collect();
        assert!(
            !exhaust_warnings.is_empty(),
            "Match on Result missing Err should warn: {:?}",
            warnings
        );
    }

    #[test]
    fn test_match_custom_enum_missing_variant() {
        let warnings = check_warnings(
            r#"
            enum Color { Red, Green, Blue }
            let c = Color::Red
            let name = match c {
                Red => "red",
                Green => "green"
            }
            "#,
        );
        let exhaust_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("Non-exhaustive"))
            .collect();
        assert!(
            !exhaust_warnings.is_empty(),
            "Match on enum missing variant should warn: {:?}",
            warnings
        );
        assert!(
            exhaust_warnings[0].message.contains("Blue"),
            "Warning should mention missing 'Blue': {}",
            exhaust_warnings[0].message
        );
    }

    #[test]
    fn test_match_wildcard_no_warning() {
        let warnings = check_warnings(
            r#"
            let x: Int? = Some(42)
            let y = match x {
                Some(v) => v,
                _ => 0
            }
            "#,
        );
        let exhaust_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("Non-exhaustive"))
            .collect();
        assert!(
            exhaust_warnings.is_empty(),
            "Wildcard should cover everything: {:?}",
            exhaust_warnings
        );
    }

    #[test]
    fn test_match_variable_pattern_no_warning() {
        let warnings = check_warnings(
            r#"
            let x: Int? = Some(42)
            let y = match x {
                Some(v) => v,
                other => 0
            }
            "#,
        );
        let exhaust_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("Non-exhaustive"))
            .collect();
        assert!(
            exhaust_warnings.is_empty(),
            "Variable pattern should cover everything: {:?}",
            exhaust_warnings
        );
    }

    #[test]
    fn test_match_non_enum_no_check() {
        // Match on Int/String should not trigger exhaustiveness checking
        let warnings = check_warnings(
            r#"
            let x = 42
            let y = match x {
                1 => "one",
                2 => "two"
            }
            "#,
        );
        let exhaust_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("Non-exhaustive"))
            .collect();
        assert!(
            exhaust_warnings.is_empty(),
            "Match on Int should not check exhaustiveness: {:?}",
            exhaust_warnings
        );
    }

    // ── Phase 5: Map field access + variadic arg checking ───────

    #[test]
    fn test_map_field_access_returns_value_type() {
        let errors = check_errors(
            r#"
            let m: Map<String, Int> = map { "x": 1 }
            let v: Int = m.x
            "#,
        );
        assert!(
            errors.is_empty(),
            "map.field should return the map's value type: {:?}",
            errors
        );
    }

    #[test]
    fn test_req_params_field_chain() {
        // req.params.id should chain: Request → Map<String,String> → String
        let errors = check_errors(
            r#"
            fn handler(req: Request) -> String {
                let id: String = req.params.id
                return id
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "req.params.id should resolve to String: {:?}",
            errors
        );
    }

    #[test]
    fn test_variadic_wrong_type_on_required_param() {
        let errors = check_errors(
            r#"
            import { json } from "std/http/server"
            json(42)
            "#,
        );
        // json's first param is Any, so this should pass
        // But let's test a variadic with typed params
        assert!(
            errors.is_empty(),
            "json(42) with Any param should not error: {:?}",
            errors
        );
    }

    #[test]
    fn test_variadic_too_few_args() {
        let errors = check_errors(
            r#"
            import { split } from "std/string"
            split("hello")
            "#,
        );
        assert!(!errors.is_empty(), "split() with too few args should error");
        assert!(
            errors[0].message.contains("expects 2") || errors[0].message.contains("argument"),
            "Error should mention arg count: {}",
            errors[0].message
        );
    }

    #[test]
    fn test_variadic_with_extra_args_ok() {
        let errors = check_errors(
            r#"
            import { json } from "std/http/server"
            json(map { "ok": true }, 201)
            "#,
        );
        assert!(
            errors.is_empty(),
            "Variadic function with extra args should not error: {:?}",
            errors
        );
    }

    #[test]
    fn test_print_any_param_works() {
        let errors = check_errors(
            r#"
            print(42)
            print("hello")
            print(true)
            "#,
        );
        assert!(
            errors.is_empty(),
            "print() with Any param should accept anything: {:?}",
            errors
        );
    }

    #[test]
    fn test_variadic_type_check_declared_params() {
        // Test that variadic still checks the declared param types
        let errors = check_errors(
            r#"
            import { html } from "std/http/server"
            html(42)
            "#,
        );
        assert!(
            !errors.is_empty(),
            "html() expects String, passing Int should error"
        );
    }

    // ── Phase 6: Cross-file pass 2 inference ────────────────────

    #[test]
    fn test_cross_file_annotated_function_resolves() {
        // Annotated stdlib imports should resolve types correctly (regression)
        let errors = check_errors(
            r#"
            import { split } from "std/string"
            let parts: Array<String> = split("a,b", ",")
            "#,
        );
        assert!(
            errors.is_empty(),
            "Annotated import should resolve return type: {:?}",
            errors
        );
    }

    #[test]
    fn test_cross_file_unannotated_function() {
        use std::io::Write;
        // Create a temp file with an unannotated function
        let dir = std::env::temp_dir().join("ntnt_test_phase6");
        let _ = std::fs::create_dir_all(&dir);
        let lib_path = dir.join("utils.tnt");
        let main_path = dir.join("main.tnt");

        // Write a lib with unannotated return type
        let mut f = std::fs::File::create(&lib_path).unwrap();
        writeln!(f, "fn double(x: Int) {{ return x * 2 }}").unwrap();

        // Write a main that imports it
        let main_src = r#"import { double } from "./utils"
let n: Int = double(5)"#;
        let mut f2 = std::fs::File::create(&main_path).unwrap();
        write!(f2, "{}", main_src).unwrap();

        let errors: Vec<_> = check_program_with_file(
            &{
                let lexer = crate::lexer::Lexer::new(main_src);
                let tokens: Vec<_> = lexer.collect();
                let mut parser = crate::parser::Parser::new(tokens);
                parser.parse().unwrap()
            },
            main_src,
            main_path.to_str().unwrap(),
        )
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();

        assert!(
            errors.is_empty(),
            "Import of unannotated function should infer return type via Pass 2: {:?}",
            errors
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cross_file_circular_import_no_crash() {
        use std::io::Write;
        // Create two files that import each other
        let dir = std::env::temp_dir().join("ntnt_test_circular");
        let _ = std::fs::create_dir_all(&dir);

        let a_path = dir.join("a.tnt");
        let b_path = dir.join("b.tnt");

        let mut fa = std::fs::File::create(&a_path).unwrap();
        writeln!(fa, "import {{ bar }} from \"./b\"").unwrap();
        writeln!(fa, "fn foo(x: Int) -> Int {{ return x + 1 }}").unwrap();

        let mut fb = std::fs::File::create(&b_path).unwrap();
        writeln!(fb, "import {{ foo }} from \"./a\"").unwrap();
        writeln!(fb, "fn bar(x: Int) -> Int {{ return x * 2 }}").unwrap();

        let a_src = std::fs::read_to_string(&a_path).unwrap();
        let diags = check_program_with_file(
            &{
                let lexer = crate::lexer::Lexer::new(&a_src);
                let tokens: Vec<_> = lexer.collect();
                let mut parser = crate::parser::Parser::new(tokens);
                parser.parse().unwrap()
            },
            &a_src,
            a_path.to_str().unwrap(),
        );

        // Should not panic/crash — no errors (cycle is a warning, not error)
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "Circular imports should not crash or produce errors: {:?}",
            errors
        );

        // Should emit a warning about the circular import with the cycle chain
        let warnings: Vec<_> = diags
            .iter()
            .filter(|d| {
                d.severity == Severity::Warning && d.message.contains("Circular import detected")
            })
            .collect();
        assert!(
            !warnings.is_empty(),
            "Circular imports should produce a warning diagnostic"
        );

        // Verify the cycle chain shows both files
        let msg = &warnings[0].message;
        assert!(
            msg.contains("→"),
            "Cycle warning should show chain with →, got: {}",
            msg
        );
        assert!(
            msg.contains("a.tnt") && msg.contains("b.tnt"),
            "Cycle warning should name both files, got: {}",
            msg
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_circular_import_three_file_cycle() {
        use std::io::Write;
        // Create a three-file cycle: a → b → c → a
        let dir = std::env::temp_dir().join("ntnt_test_circular_three");
        let _ = std::fs::create_dir_all(&dir);

        let a_path = dir.join("a.tnt");
        let b_path = dir.join("b.tnt");
        let c_path = dir.join("c.tnt");

        let mut fa = std::fs::File::create(&a_path).unwrap();
        writeln!(fa, "import {{ cfn }} from \"./c\"").unwrap();
        writeln!(fa, "fn afn(x: Int) -> Int {{ return x + 1 }}").unwrap();

        let mut fb = std::fs::File::create(&b_path).unwrap();
        writeln!(fb, "import {{ afn }} from \"./a\"").unwrap();
        writeln!(fb, "fn bfn(x: Int) -> Int {{ return x + 2 }}").unwrap();

        let mut fc = std::fs::File::create(&c_path).unwrap();
        writeln!(fc, "import {{ bfn }} from \"./b\"").unwrap();
        writeln!(fc, "fn cfn(x: Int) -> Int {{ return x + 3 }}").unwrap();

        let a_src = std::fs::read_to_string(&a_path).unwrap();
        let diags = check_program_with_file(
            &{
                let lexer = crate::lexer::Lexer::new(&a_src);
                let tokens: Vec<_> = lexer.collect();
                let mut parser = crate::parser::Parser::new(tokens);
                parser.parse().unwrap()
            },
            &a_src,
            a_path.to_str().unwrap(),
        );

        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "Three-file circular import should not produce errors: {:?}",
            errors
        );

        let warnings: Vec<_> = diags
            .iter()
            .filter(|d| {
                d.severity == Severity::Warning && d.message.contains("Circular import detected")
            })
            .collect();
        assert!(
            !warnings.is_empty(),
            "Three-file circular import should produce a cycle warning"
        );

        // The cycle chain should include all three files
        let msg = &warnings[0].message;
        assert!(
            msg.contains("a.tnt"),
            "Cycle should reference a.tnt, got: {}",
            msg
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Phase 7: Strict mode enhancements ───────────────────────

    #[test]
    fn test_strict_no_warning_interpolate_int_string() {
        let warnings = check_strict_warnings(
            r##"
            let x: Int = 42
            let s: String = "hello"
            let msg = "x is #{x} and s is #{s}"
            "##,
        );
        let interp_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("Interpolating"))
            .collect();
        assert!(
            interp_warnings.is_empty(),
            "Interpolating Int/String should not warn: {:?}",
            interp_warnings
        );
    }

    #[test]
    fn test_strict_warns_interpolate_array() {
        let warnings = check_strict_warnings(
            r##"
            let arr = [1, 2, 3]
            let msg = "arr is #{arr}"
            "##,
        );
        let interp_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("Interpolating"))
            .collect();
        assert!(
            !interp_warnings.is_empty(),
            "Interpolating Array in strict mode should warn: {:?}",
            warnings
        );
    }

    #[test]
    fn test_strict_warns_float_to_int() {
        let warnings = check_strict_warnings(
            r#"
            let x: Int = 3.14
            "#,
        );
        let coercion_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("Float to Int"))
            .collect();
        assert!(
            !coercion_warnings.is_empty(),
            "Float to Int assignment in strict mode should warn: {:?}",
            warnings
        );
    }

    #[test]
    fn test_non_strict_no_interpolation_or_coercion_warnings() {
        // Normal mode should NOT produce these warnings
        let warnings = check_warnings(
            r#"
            let arr = [1, 2, 3]
            let msg = "arr is {arr}"
            let x: Int = 3.14
            "#,
        );
        let strict_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("Interpolating") || w.message.contains("Float to Int"))
            .collect();
        assert!(
            strict_warnings.is_empty(),
            "Non-strict mode should not produce interpolation or coercion warnings: {:?}",
            strict_warnings
        );
    }

    // ── Phase A: Bidirectional closure parameter inference ─────────

    #[test]
    fn test_bidir_filter_infers_lambda_param_int() {
        // filter(Array<Int>, fn(x) { ... }) → x should be inferred as Int
        let diags = check(
            r#"
            let nums: Array<Int> = [1, 2, 3]
            let pos = filter(nums, fn(x) { x > 0 })
            let result: Array<Int> = pos
            "#,
        );
        assert!(
            diags.is_empty(),
            "filter lambda param should be inferred as Int: {:?}",
            diags
        );
    }

    #[test]
    fn test_bidir_transform_infers_lambda_param_string() {
        // transform(Array<String>, fn(s) { len(s) }) → s inferred as String, result Array<Int>
        let diags = check(
            r#"
            let words: Array<String> = ["hello", "world"]
            let lengths = transform(words, fn(s) { len(s) })
            let result: Array<Int> = lengths
            "#,
        );
        assert!(
            diags.is_empty(),
            "transform lambda should infer String param and Int return: {:?}",
            diags
        );
    }

    #[test]
    fn test_bidir_sort_by_infers_both_params() {
        // sort_by is a known HOF in get_callback_expected_types:
        // sort_by(Array<T>, fn(T, T) -> Int) → both params inferred as T
        // We test this by defining a sort_by with typed params, which exercises the
        // user-defined-function path since it's registered by collect_declaration.
        let source = r#"
            fn sort_by(arr: Array<Int>, cmp: Any) -> Array<Int> {
                return arr
            }
            let nums: Array<Int> = [3, 1, 2]
            let sorted = sort_by(nums, fn(a, b) { a - b })
            let result: Array<Int> = sorted
        "#;
        let lexer = crate::lexer::Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = crate::parser::Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let mut ctx = TypeContext::new(source);
        ctx.register_builtins();

        for stmt in &ast.statements {
            ctx.collect_declaration(stmt);
        }
        for stmt in &ast.statements {
            ctx.check_statement(stmt);
        }

        let errors: Vec<_> = ctx
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "sort_by lambda should not produce errors: {:?}",
            errors
        );
    }

    #[test]
    fn test_bidir_reduce_infers_accumulator_and_element() {
        // reduce is a known HOF in get_callback_expected_types:
        // reduce(Array<T>, init: U, fn(U, T) -> U) → params inferred
        let source = r#"
            fn reduce(arr: Array<Int>, init: Int, f: Any) -> Int {
                return init
            }
            let nums: Array<Int> = [1, 2, 3]
            let total: Int = reduce(nums, 0, fn(acc, x) { acc + x })
        "#;
        let lexer = crate::lexer::Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = crate::parser::Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let mut ctx = TypeContext::new(source);
        ctx.register_builtins();

        for stmt in &ast.statements {
            ctx.collect_declaration(stmt);
        }
        for stmt in &ast.statements {
            ctx.check_statement(stmt);
        }

        let errors: Vec<_> = ctx
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "reduce lambda should not produce errors: {:?}",
            errors
        );
    }

    #[test]
    fn test_bidir_explicit_annotation_overrides_inference() {
        // Lambda with explicit type annotation should keep it even when inference disagrees
        let errs = check_errors(
            r#"
            let nums: Array<Int> = [1, 2, 3]
            filter(nums, fn(x: String) { len(x) > 0 })
            "#,
        );
        // The explicit String annotation should be kept, not overridden by Int
        // This means x:String is fine — the filter predicate param type won't cause an error
        // because filter's second param is Any in the signature
        assert!(
            errs.is_empty(),
            "Explicit annotation should override inference: {:?}",
            errs
        );
    }

    #[test]
    fn test_bidir_user_defined_fn_with_function_param() {
        // User-defined function with Function parameter type → callback params inferred
        // We register the function sig manually since the parser doesn't support
        // (Int) -> String type syntax in annotations.
        let source = r#"
            let result: String = apply(42, fn(n) { str(n) })
        "#;
        let lexer = crate::lexer::Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = crate::parser::Parser::new(tokens);
        let ast = parser.parse().unwrap();
        let mut ctx = TypeContext::new(source);
        ctx.register_builtins();

        // Register `apply(Int, (Int) -> String) -> String` manually
        ctx.builtin_sigs.insert(
            "apply".to_string(),
            FunctionSig {
                params: vec![
                    ("value".to_string(), Type::Int),
                    (
                        "transform".to_string(),
                        Type::Function {
                            params: vec![Type::Int],
                            return_type: Box::new(Type::String),
                        },
                    ),
                ],
                return_type: Type::String,
                variadic: false,
                required_params: 2,
                type_params: vec![],
            },
        );

        for stmt in &ast.statements {
            ctx.collect_declaration(stmt);
        }
        for stmt in &ast.statements {
            ctx.check_statement(stmt);
        }

        let errors: Vec<_> = ctx
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "Function param type should guide lambda inference: {:?}",
            errors
        );
    }

    #[test]
    fn test_bidir_untyped_array_no_false_inference() {
        // Untyped array (Any) → lambda params should remain Any, no false inference
        let diags = check(
            r#"
            let items = [1, 2, 3]
            let mapped = transform(items, fn(x) { x + 1 })
            "#,
        );
        // Should produce no errors - gradual typing means x is inferred from Array<Int>
        assert!(
            diags.is_empty(),
            "Typed array should still allow inference: {:?}",
            diags
        );
    }

    #[test]
    fn test_bidir_fully_untyped_no_diagnostics() {
        // Completely untyped code produces zero diagnostics (gradual typing contract)
        let diags = check(
            r#"
            let items = [1, 2, 3]
            let result = filter(items, fn(x) { x > 0 })
            let mapped = transform(items, fn(x) { x * 2 })
            "#,
        );
        assert!(
            diags.is_empty(),
            "Fully untyped code should produce zero diagnostics: {:?}",
            diags
        );
    }

    // ── Phase B: Cross-file struct/enum propagation ────────────────
    // Note: Cross-file tests require actual file system access, so we test
    // the FileExports struct and the in-memory propagation paths.

    #[test]
    fn test_file_exports_default() {
        let exports = FileExports::default();
        assert!(exports.functions.is_empty());
        assert!(exports.structs.is_empty());
        assert!(exports.enums.is_empty());
        assert!(exports.type_aliases.is_empty());
    }

    #[test]
    fn test_cross_file_struct_available_after_registration() {
        // Simulate cross-file import: register a struct directly and check field access
        let source = r#"
            let u = User { name: "Alice" }
            let n: String = u.name
        "#;
        let lexer = crate::lexer::Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = crate::parser::Parser::new(tokens);
        let ast = parser.parse().unwrap();

        let mut ctx = TypeContext::new(source);
        ctx.register_builtins();

        // Manually register a "User" struct (simulating cross-file import)
        ctx.structs
            .insert("User".to_string(), vec![("name".to_string(), Type::String)]);

        for stmt in &ast.statements {
            ctx.collect_declaration(stmt);
        }
        for stmt in &ast.statements {
            ctx.check_statement(stmt);
        }

        let errors: Vec<_> = ctx
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "Imported struct fields should be accessible: {:?}",
            errors
        );
    }

    #[test]
    fn test_cross_file_struct_as_type_annotation() {
        // After registering a struct, it should be usable as a type annotation
        let source = r#"
            let u: User = User { name: "Alice" }
        "#;
        let lexer = crate::lexer::Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = crate::parser::Parser::new(tokens);
        let ast = parser.parse().unwrap();

        let mut ctx = TypeContext::new(source);
        ctx.register_builtins();
        ctx.structs
            .insert("User".to_string(), vec![("name".to_string(), Type::String)]);

        for stmt in &ast.statements {
            ctx.collect_declaration(stmt);
        }
        for stmt in &ast.statements {
            ctx.check_statement(stmt);
        }

        let errors: Vec<_> = ctx
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "Struct type annotation should work: {:?}",
            errors
        );
    }

    #[test]
    fn test_cross_file_enum_match() {
        // After registering an enum, match arms should work
        let source = r#"
            let c = Color::Red
        "#;
        let lexer = crate::lexer::Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = crate::parser::Parser::new(tokens);
        let ast = parser.parse().unwrap();

        let mut ctx = TypeContext::new(source);
        ctx.register_builtins();
        ctx.enums.insert(
            "Color".to_string(),
            vec![
                ("Red".to_string(), None),
                ("Green".to_string(), None),
                ("Blue".to_string(), None),
            ],
        );

        for stmt in &ast.statements {
            ctx.collect_declaration(stmt);
        }
        for stmt in &ast.statements {
            ctx.check_statement(stmt);
        }

        let errors: Vec<_> = ctx
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "Imported enum should be usable: {:?}",
            errors
        );
    }

    #[test]
    fn test_cross_file_type_alias() {
        // Type alias from imported file resolves to underlying type
        let source = r#"
            let x: UserId = 42
        "#;
        let lexer = crate::lexer::Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = crate::parser::Parser::new(tokens);
        let ast = parser.parse().unwrap();

        let mut ctx = TypeContext::new(source);
        ctx.register_builtins();
        ctx.type_aliases.insert("UserId".to_string(), Type::Int);

        for stmt in &ast.statements {
            ctx.collect_declaration(stmt);
        }
        for stmt in &ast.statements {
            ctx.check_statement(stmt);
        }

        let errors: Vec<_> = ctx
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "Type alias should resolve to Int: {:?}",
            errors
        );
    }

    #[test]
    fn test_cross_file_mixed_imports() {
        // Import mix of functions + structs from same file (simulated)
        let source = r#"
            let u = User { name: "Alice" }
            let g = greet("Bob")
        "#;
        let lexer = crate::lexer::Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = crate::parser::Parser::new(tokens);
        let ast = parser.parse().unwrap();

        let mut ctx = TypeContext::new(source);
        ctx.register_builtins();

        // Simulate importing a struct and a function from the same file
        ctx.structs
            .insert("User".to_string(), vec![("name".to_string(), Type::String)]);
        ctx.builtin_sigs.insert(
            "greet".to_string(),
            FunctionSig {
                params: vec![("name".to_string(), Type::String)],
                return_type: Type::String,
                variadic: false,
                required_params: 1,
                type_params: vec![],
            },
        );

        for stmt in &ast.statements {
            ctx.collect_declaration(stmt);
        }
        for stmt in &ast.statements {
            ctx.check_statement(stmt);
        }

        let errors: Vec<_> = ctx
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "Mixed imports should all resolve: {:?}",
            errors
        );
    }

    // ── Phase C: ? operator return type validation ─────────────────

    #[test]
    fn test_try_result_in_result_function_no_warning() {
        // ? on Result inside function returning Result → no warning
        let warnings = check_warnings(
            r#"
            fn parse(s: String) -> Result<Int, String> {
                return Ok(42)
            }
            fn process(input: String) -> Result<String, String> {
                let n = parse(input)?
                return Ok(str(n))
            }
            "#,
        );
        let try_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("? on Result"))
            .collect();
        assert!(
            try_warnings.is_empty(),
            "? on Result in Result-returning function should not warn: {:?}",
            try_warnings
        );
    }

    #[test]
    fn test_try_result_in_int_function_warns() {
        // ? on Result inside function returning Int → warning
        let warnings = check_warnings(
            r#"
            fn parse(s: String) -> Result<Int, String> {
                return Ok(42)
            }
            fn process(input: String) -> Int {
                let n = parse(input)?
                return n
            }
            "#,
        );
        let try_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("? on Result"))
            .collect();
        assert!(
            !try_warnings.is_empty(),
            "? on Result in Int-returning function should warn: {:?}",
            warnings
        );
    }

    #[test]
    fn test_try_optional_in_optional_function_no_warning() {
        // ? on Optional inside function returning Optional → no warning
        let warnings = check_warnings(
            r#"
            fn find_user(id: Int) -> Option<String> {
                return Some("Alice")
            }
            fn get_name(id: Int) -> Option<String> {
                let name = find_user(id)?
                return Some(name)
            }
            "#,
        );
        let try_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("? on Optional"))
            .collect();
        assert!(
            try_warnings.is_empty(),
            "? on Optional in Optional-returning function should not warn: {:?}",
            try_warnings
        );
    }

    #[test]
    fn test_try_optional_in_string_function_warns() {
        // ? on Optional inside function returning String → warning
        let warnings = check_warnings(
            r#"
            fn find_user(id: Int) -> Option<String> {
                return Some("Alice")
            }
            fn get_name(id: Int) -> String {
                let name = find_user(id)?
                return name
            }
            "#,
        );
        let try_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("? on Optional"))
            .collect();
        assert!(
            !try_warnings.is_empty(),
            "? on Optional in String-returning function should warn: {:?}",
            warnings
        );
    }

    #[test]
    fn test_try_in_untyped_function_no_warning() {
        // ? in function with no return annotation → no warning (gradual typing)
        let warnings = check_warnings(
            r#"
            fn parse(s: String) -> Result<Int, String> {
                return Ok(42)
            }
            fn process(input: String) {
                let n = parse(input)?
                print(n)
            }
            "#,
        );
        let try_warnings: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("? on Result") || w.message.contains("? on Optional"))
            .collect();
        assert!(
            try_warnings.is_empty(),
            "? in untyped function should not warn: {:?}",
            try_warnings
        );
    }

    // ── Default parameter value tests ──

    #[test]
    fn test_default_param_arity_accepts_omitted() {
        let errors = check_errors(
            r#"
            fn greet(name: String = "World") -> String {
                return "Hello, {name}!"
            }
            greet()
            greet("Alice")
            "#,
        );
        assert!(
            errors.is_empty(),
            "Should accept 0 or 1 args when default is provided: {:?}",
            errors
        );
    }

    #[test]
    fn test_default_param_arity_rejects_too_many() {
        let errors = check_errors(
            r#"
            fn greet(name: String = "World") -> String {
                return "Hello, {name}!"
            }
            greet("A", "B")
            "#,
        );
        assert!(
            !errors.is_empty(),
            "Should reject 2 args for 1-param function"
        );
    }

    #[test]
    fn test_default_param_type_inferred_from_default() {
        let errors = check_errors(
            r#"
            fn add(a: Int, b = 10) -> Int {
                return a + b
            }
            add(5)
            add(5, "hello")
            "#,
        );
        assert!(
            !errors.is_empty(),
            "Should catch type error when string passed for int-defaulted param: {:?}",
            errors
        );
    }

    #[test]
    fn test_default_param_mixed_required_optional() {
        let errors = check_errors(
            r#"
            fn paginate(items: String, page: Int = 1, per_page: Int = 25) -> String {
                return "{items}:{page}:{per_page}"
            }
            paginate("users")
            paginate("users", 2)
            paginate("users", 2, 10)
            "#,
        );
        assert!(
            errors.is_empty(),
            "Should accept 1-3 args with defaults: {:?}",
            errors
        );
    }

    // ── Recursive type aliases (DD-009 Phase 7.2) ───────────────

    #[test]
    fn test_recursive_type_alias_typechecks() {
        // A recursive type alias should not produce errors — self-references
        // resolve to Type::Named("JsonValue") via the placeholder mechanism.
        let errors = check_errors(
            r#"
            type JsonValue = String | Int | Float | Bool | [JsonValue] | Map<String, JsonValue>
            fn process(v: JsonValue) -> String {
                return str(v)
            }
            process("hello")
            process(42)
            process(3.14)
            process(true)
            "#,
        );
        assert!(
            errors.is_empty(),
            "Recursive type alias should type-check without errors: {:?}",
            errors
        );
    }

    #[test]
    fn test_recursive_type_alias_no_infinite_loop() {
        // Ensure collecting a recursive alias doesn't hang/panic
        let diags = check(
            r#"
            type Tree = String | [Tree]
            let x: Tree = "leaf"
            "#,
        );
        // We only care that this completes (no panic), diagnostics may vary
        let _ = diags;
    }

    // ── Generic struct support (DD-009 Phase 7.4) ────────────────

    #[test]
    fn test_generic_struct_declaration() {
        // Declaring a generic struct should not produce errors
        let errors = check_errors(
            r#"
            struct Pair<A, B> {
                first: A,
                second: B,
            }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Generic struct declaration should not produce errors: {:?}",
            errors
        );
    }

    #[test]
    fn test_generic_struct_construction() {
        // Constructing a generic struct with concrete types should not error
        let errors = check_errors(
            r#"
            struct Pair<A, B> {
                first: A,
                second: B,
            }
            let p = Pair { first: 42, second: "hello" }
            "#,
        );
        assert!(
            errors.is_empty(),
            "Generic struct construction should not produce errors: {:?}",
            errors
        );
    }

    #[test]
    fn test_generic_struct_field_inference() {
        // Field access on a generic struct should return the inferred concrete type
        let errors = check_errors(
            r#"
            struct Pair<A, B> {
                first: A,
                second: B,
            }
            let p = Pair { first: 42, second: "hello" }
            let x: Int = p.first
            let y: String = p.second
            "#,
        );
        assert!(
            errors.is_empty(),
            "Generic struct field access should infer concrete types: {:?}",
            errors
        );
    }

    #[test]
    fn test_generic_struct_field_type_mismatch() {
        // Annotating a generic struct field with the wrong concrete type should error
        let errors = check_errors(
            r#"
            struct Pair<A, B> {
                first: A,
                second: B,
            }
            let p = Pair { first: 42, second: "hello" }
            let x: String = p.first
            "#,
        );
        assert!(
            !errors.is_empty(),
            "Assigning Int field to String variable should produce a type error: {:?}",
            errors
        );
    }
}
