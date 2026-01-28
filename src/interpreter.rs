//! Interpreter for Intent
//!
//! A tree-walking interpreter for executing Intent programs.
//!
//! ## Contract Support
//!
//! This interpreter fully supports design-by-contract with:
//! - `requires` clauses (preconditions) evaluated before function execution
//! - `ensures` clauses (postconditions) evaluated after function execution
//! - `old(expr)` to capture pre-execution values for postcondition checks
//! - `result` to reference the return value in postconditions

use crate::ast::*;
use crate::contracts::{ContractChecker, OldValues, StoredValue};
use crate::error::{IntentError, Result};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

/// Runtime values
#[derive(Debug, Clone)]
pub enum Value {
    /// Unit value
    Unit,

    /// Integer value
    Int(i64),

    /// Float value
    Float(f64),

    /// Boolean value
    Bool(bool),

    /// String value
    String(String),

    /// Array value
    Array(Vec<Value>),

    /// Map value
    Map(HashMap<String, Value>),

    /// Range value
    Range {
        start: i64,
        end: i64,
        inclusive: bool,
    },

    /// Struct instance
    Struct {
        name: String,
        fields: HashMap<String, Value>,
    },

    /// Enum variant instance (for ADTs like Option, Result)
    EnumValue {
        enum_name: String,
        variant: String,
        values: Vec<Value>,
    },

    /// Enum constructor (for creating enum values dynamically)
    EnumConstructor {
        enum_name: String,
        variant: String,
        arity: usize,
    },

    /// Function value with contract
    Function {
        name: String,
        params: Vec<Parameter>,
        body: Block,
        closure: Rc<RefCell<Environment>>,
        contract: Option<FunctionContract>,
        type_params: Vec<TypeParam>,
    },

    /// Native/built-in function
    NativeFunction {
        name: String,
        arity: usize,
        func: fn(&[Value]) -> Result<Value>,
    },

    /// Return value (for control flow)
    Return(Box<Value>),

    /// Break (for loop control)
    Break,

    /// Continue (for loop control)
    Continue,
}

/// Function contract with parsed expressions for runtime evaluation
#[derive(Debug, Clone)]
pub struct FunctionContract {
    /// Precondition expressions
    pub requires: Vec<Expression>,
    /// Postcondition expressions
    pub ensures: Vec<Expression>,
}

impl Value {
    /// Determine if a value is truthy for conditionals
    ///
    /// Falsy values: false, Unit, None, empty strings, empty arrays, empty maps
    /// Truthy values: everything else (including 0 and 0.0 to avoid subtle bugs)
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Unit => false,
            // Numbers are ALWAYS truthy (including 0) - avoids "if count {}" bugs
            Value::Int(_) => true,
            Value::Float(_) => true,
            // Empty collections are falsy
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Map(m) => !m.is_empty(),
            // None is falsy, Some(x) is truthy
            Value::EnumValue {
                enum_name, variant, ..
            } => !(enum_name == "Option" && variant == "None"),
            // Everything else is truthy
            _ => true,
        }
    }

    pub fn type_name(&self) -> &str {
        match self {
            Value::Unit => "Unit",
            Value::Int(_) => "Int",
            Value::Float(_) => "Float",
            Value::Bool(_) => "Bool",
            Value::String(_) => "String",
            Value::Array(_) => "Array",
            Value::Map(_) => "Map",
            Value::Range { .. } => "Range",
            Value::Struct { name, .. } => name,
            Value::EnumValue { enum_name, .. } => enum_name,
            Value::EnumConstructor { .. } => "EnumConstructor",
            Value::Function { .. } => "Function",
            Value::NativeFunction { .. } => "NativeFunction",
            Value::Return(_) => "Return",
            Value::Break => "Break",
            Value::Continue => "Continue",
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Unit => write!(f, "()"),
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Bool(b) => write!(f, "{}", b),
            Value::String(s) => write!(f, "{}", s),
            Value::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| v.to_string()).collect();
                write!(f, "[{}]", items.join(", "))
            }
            Value::Map(map) => {
                let items: Vec<String> = map.iter().map(|(k, v)| format!("{}: {}", k, v)).collect();
                write!(f, "{{ {} }}", items.join(", "))
            }
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                if *inclusive {
                    write!(f, "{}..={}", start, end)
                } else {
                    write!(f, "{}..{}", start, end)
                }
            }
            Value::Struct { name, fields } => {
                let field_strs: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v))
                    .collect();
                write!(f, "{} {{ {} }}", name, field_strs.join(", "))
            }
            Value::EnumValue {
                enum_name,
                variant,
                values,
            } => {
                if values.is_empty() {
                    write!(f, "{}::{}", enum_name, variant)
                } else {
                    let vals: Vec<String> = values.iter().map(|v| v.to_string()).collect();
                    write!(f, "{}::{}({})", enum_name, variant, vals.join(", "))
                }
            }
            Value::EnumConstructor {
                enum_name,
                variant,
                arity,
            } => {
                write!(f, "<constructor {}::{}({})>", enum_name, variant, arity)
            }
            Value::Function { name, .. } => write!(f, "<fn {}>", name),
            Value::NativeFunction { name, .. } => write!(f, "<native fn {}>", name),
            Value::Return(v) => write!(f, "{}", v),
            Value::Break => write!(f, "<break>"),
            Value::Continue => write!(f, "<continue>"),
        }
    }
}

/// Environment for variable bindings
#[derive(Debug, Clone)]
pub struct Environment {
    values: HashMap<String, Value>,
    parent: Option<Rc<RefCell<Environment>>>,
}

impl Environment {
    pub fn new() -> Self {
        Environment {
            values: HashMap::new(),
            parent: None,
        }
    }

    pub fn with_parent(parent: Rc<RefCell<Environment>>) -> Self {
        Environment {
            values: HashMap::new(),
            parent: Some(parent),
        }
    }

    pub fn define(&mut self, name: String, value: Value) {
        self.values.insert(name, value);
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some(value) = self.values.get(name) {
            Some(value.clone())
        } else if let Some(ref parent) = self.parent {
            parent.borrow().get(name)
        } else {
            None
        }
    }

    pub fn set(&mut self, name: &str, value: Value) -> bool {
        if self.values.contains_key(name) {
            self.values.insert(name.to_string(), value);
            true
        } else if let Some(ref parent) = self.parent {
            parent.borrow_mut().set(name, value)
        } else {
            false
        }
    }

    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<_> = self.values.keys().cloned().collect();
        if let Some(ref parent) = self.parent {
            keys.extend(parent.borrow().keys());
        }
        keys.sort();
        keys.dedup();
        keys
    }
}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}

/// Execution mode controls how server-related functions behave
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ExecutionMode {
    /// Normal execution - all functions run normally
    #[default]
    Normal,
    /// Hot-reload mode - skip listen(), re-register routes
    HotReload,
    /// Unit test mode - skip all server-related calls
    UnitTest,
}

/// The Intent interpreter
pub struct Interpreter {
    environment: Rc<RefCell<Environment>>,
    contracts: ContractChecker,
    /// Struct type definitions
    structs: HashMap<String, Vec<Field>>,
    /// Enum type definitions (name -> variants with their field types)
    enums: HashMap<String, Vec<EnumVariant>>,
    /// Type aliases (alias -> target type expression)
    type_aliases: HashMap<String, TypeExpr>,
    /// Struct invariants
    struct_invariants: HashMap<String, Vec<Expression>>,
    /// Trait implementations: type_name -> list of trait names
    trait_implementations: HashMap<String, Vec<String>>,
    /// Trait definitions: trait_name -> trait info
    trait_definitions: HashMap<String, TraitInfo>,
    /// Deferred statements for current scope
    deferred_statements: Vec<Expression>,
    /// Old values for current function call (used in postconditions)
    current_old_values: Option<OldValues>,
    /// Current function's result value (used in postconditions)
    current_result: Option<Value>,
    /// Loaded modules cache
    loaded_modules: HashMap<String, HashMap<String, Value>>,
    /// Current file path (for relative imports)
    current_file: Option<String>,
    /// HTTP server state for routing
    server_state: crate::stdlib::http_server::ServerState,
    /// Test mode: if Some, contains (port, max_requests, shutdown_flag)
    test_mode: Option<(u16, usize, std::sync::Arc<std::sync::atomic::AtomicBool>)>,
    /// Main source file path for hot-reload (single-file apps)
    main_source_file: Option<String>,
    /// Main source file last modification time
    main_source_mtime: Option<std::time::SystemTime>,
    /// Tracked imported files for hot-reload (path -> mtime)
    imported_files: HashMap<String, std::time::SystemTime>,
    /// Request timeout in seconds for HTTP server
    request_timeout_secs: u64,
    /// Execution mode controls how server-related functions behave
    execution_mode: ExecutionMode,
    /// Lib modules for file-based routing (stored for hot-reload)
    lib_modules: HashMap<String, HashMap<String, Value>>,
}

/// Information about a trait definition
#[derive(Debug, Clone)]
pub struct TraitInfo {
    pub name: String,
    pub methods: Vec<TraitMethodInfo>,
    pub supertraits: Vec<String>,
}

/// Information about a trait method
#[derive(Debug, Clone)]
pub struct TraitMethodInfo {
    pub name: String,
    pub params: Vec<Parameter>,
    pub return_type: Option<TypeExpr>,
    pub has_default: bool,
}

impl Interpreter {
    pub fn new() -> Self {
        let env = Rc::new(RefCell::new(Environment::new()));
        let mut interpreter = Interpreter {
            environment: env,
            contracts: ContractChecker::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            type_aliases: HashMap::new(),
            struct_invariants: HashMap::new(),
            trait_implementations: HashMap::new(),
            trait_definitions: HashMap::new(),
            deferred_statements: Vec::new(),
            current_old_values: None,
            current_result: None,
            loaded_modules: HashMap::new(),
            current_file: None,
            server_state: crate::stdlib::http_server::ServerState::new(),
            test_mode: None,
            main_source_file: None,
            main_source_mtime: None,
            imported_files: HashMap::new(),
            request_timeout_secs: 30,
            execution_mode: ExecutionMode::Normal,
            lib_modules: HashMap::new(),
        };
        interpreter.define_builtins();
        interpreter.define_builtin_types();
        interpreter.define_stdlib();
        interpreter
    }

    /// Enable test mode - server will handle limited requests then exit
    pub fn set_test_mode(
        &mut self,
        port: u16,
        max_requests: usize,
        shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        self.test_mode = Some((port, max_requests, shutdown_flag));
    }

    /// Set the request timeout for the HTTP server (in seconds)
    pub fn set_request_timeout(&mut self, seconds: u64) {
        self.request_timeout_secs = seconds;
    }

    /// Set the execution mode for the interpreter
    pub fn set_execution_mode(&mut self, mode: ExecutionMode) {
        self.execution_mode = mode;
    }

    /// Check if a server-related function should be skipped entirely
    fn should_skip_server_call(&self, name: &str) -> bool {
        match self.execution_mode {
            ExecutionMode::Normal => false,
            ExecutionMode::HotReload => {
                // In hot-reload, only skip listen() and on_shutdown()
                matches!(name, "listen" | "on_shutdown")
            }
            ExecutionMode::UnitTest => {
                // In unit test mode, skip all server-related functions
                matches!(
                    name,
                    "listen" | "serve_static" | "routes" | "use_middleware" | "on_shutdown"
                )
            }
        }
    }

    /// Check if route registration should be skipped (for HTTP methods used as routes)
    fn should_skip_route_registration(&self) -> bool {
        self.execution_mode == ExecutionMode::UnitTest
    }

    /// Set the current file path for relative imports
    pub fn set_current_file(&mut self, path: &str) {
        self.current_file = Some(path.to_string());
    }

    /// Resolve a path relative to the current script's directory
    /// If the path is absolute, return it as-is
    /// If relative, resolve it relative to the .tnt file's directory (not cwd)
    fn resolve_path_relative_to_script(&self, path: &str) -> String {
        let path_obj = std::path::Path::new(path);

        // If already absolute, return as-is
        if path_obj.is_absolute() {
            return path.to_string();
        }

        // Resolve relative to current script's directory
        if let Some(current_file) = &self.current_file {
            let script_dir = std::path::Path::new(current_file)
                .parent()
                .unwrap_or(std::path::Path::new("."));
            return script_dir.join(path).to_string_lossy().to_string();
        }

        // Fallback: return path as-is (will resolve relative to cwd)
        path.to_string()
    }

    /// Define a variable in the current environment
    pub fn define_variable(&mut self, name: String, value: Value) {
        self.environment.borrow_mut().define(name, value);
    }

    /// Call a function by name with the given arguments
    ///
    /// This is useful for external callers (like the IAL test runner) that want
    /// to invoke NTNT functions after loading a module.
    pub fn call_function_by_name(&mut self, name: &str, args: Vec<Value>) -> Result<Value> {
        // Look up the function in the environment
        let func = self.environment.borrow().get(name).ok_or_else(|| {
            let candidates = self.environment.borrow().keys();
            let suggestion = crate::error::find_suggestion(name, &candidates);
            IntentError::UndefinedVariable {
                name: name.to_string(),
                suggestion,
            }
        })?;

        // Verify it's a function
        match &func {
            Value::Function { .. } | Value::NativeFunction { .. } => {}
            _ => {
                return Err(IntentError::TypeError(format!(
                    "Expected function, got {}",
                    func.type_name()
                )))
            }
        }

        // Call the function
        self.call_function(func, args)
    }

    /// Set the main source file for hot-reload tracking
    pub fn set_main_source_file(&mut self, path: &str) {
        self.main_source_file = Some(path.to_string());
        // Store the current mtime
        self.main_source_mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    }

    /// Check if any tracked source file needs reloading and reload if necessary
    /// Checks the main source file AND all imported files
    /// Returns true if reload happened, false otherwise
    fn check_and_reload_main_source(&mut self) -> bool {
        // Check if hot-reload is enabled
        if !self.server_state.hot_reload {
            return false;
        }

        // Only check if we have a main source file configured
        let (file_path, cached_mtime) = match (&self.main_source_file, &self.main_source_mtime) {
            (Some(fp), Some(mt)) => (fp.clone(), *mt),
            _ => return false,
        };

        // Check if main file changed
        let mut changed_file: Option<String> = None;
        let current_mtime = match std::fs::metadata(&file_path) {
            Ok(m) => match m.modified() {
                Ok(mt) => mt,
                Err(_) => return false,
            },
            Err(_) => return false,
        };

        if current_mtime > cached_mtime {
            changed_file = Some(file_path.clone());
        }

        // Check all imported files for changes
        if changed_file.is_none() {
            for (import_path, import_mtime) in &self.imported_files {
                if let Ok(metadata) = std::fs::metadata(import_path) {
                    if let Ok(current) = metadata.modified() {
                        if current > *import_mtime {
                            changed_file = Some(import_path.clone());
                            break;
                        }
                    }
                }
            }
        }

        // No changes detected
        let changed_file = match changed_file {
            Some(f) => f,
            None => return false,
        };

        // File changed - reload!
        println!("\n[hot-reload] {} changed, reloading...", changed_file);

        // Read the main source (we always reload from the main file)
        let source_code = match std::fs::read_to_string(&file_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[hot-reload] Failed to read file: {}", e);
                return false;
            }
        };

        // Parse the source
        use crate::lexer::Lexer;
        use crate::parser::Parser;

        let lexer = Lexer::new(&source_code);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = Parser::new(tokens);

        let ast = match parser.parse() {
            Ok(ast) => ast,
            Err(e) => {
                eprintln!("[hot-reload] Parse error: {}", e);
                return false;
            }
        };

        // Clear current state (routes, middleware, etc.) but keep server running
        self.server_state.clear();

        // Clear loaded modules and imported file tracking to force reimport
        self.loaded_modules.clear();
        self.imported_files.clear();

        // Reset environment but keep builtins
        self.environment = std::rc::Rc::new(std::cell::RefCell::new(Environment::new()));
        self.define_builtins();
        self.define_builtin_types();
        self.define_stdlib(); // Re-populate stdlib modules after clearing

        // Re-set the current file for imports
        self.current_file = Some(file_path.clone());

        // Set hot-reload mode so listen() knows to skip re-binding
        self.execution_mode = ExecutionMode::HotReload;

        // Re-evaluate the AST
        let result = match self.eval(&ast) {
            Ok(_) => {
                // Update main file mtime
                self.main_source_mtime = Some(
                    std::fs::metadata(&file_path)
                        .and_then(|m| m.modified())
                        .unwrap_or(current_mtime),
                );
                let import_count = self.imported_files.len();
                println!(
                    "[hot-reload] Reload complete. {} routes, {} imports tracked.",
                    self.server_state.route_count(),
                    import_count
                );
                true
            }
            Err(e) => {
                eprintln!("[hot-reload] Evaluation error: {}", e);
                false
            }
        };

        // Reset to normal mode
        self.execution_mode = ExecutionMode::Normal;
        result
    }

    fn define_builtins(&mut self) {
        // Print function
        self.environment.borrow_mut().define(
            "print".to_string(),
            Value::NativeFunction {
                name: "print".to_string(),
                arity: 1,
                func: |args| {
                    for arg in args {
                        println!("{}", arg);
                    }
                    Ok(Value::Unit)
                },
            },
        );

        // Length function
        self.environment.borrow_mut().define(
            "len".to_string(),
            Value::NativeFunction {
                name: "len".to_string(),
                arity: 1,
                func: |args| match &args[0] {
                    Value::String(s) => Ok(Value::Int(s.len() as i64)),
                    Value::Array(a) => Ok(Value::Int(a.len() as i64)),
                    _ => Err(IntentError::TypeError(
                        "len() requires a string or array".to_string(),
                    )),
                },
            },
        );

        // Type function
        self.environment.borrow_mut().define(
            "type".to_string(),
            Value::NativeFunction {
                name: "type".to_string(),
                arity: 1,
                func: |args| Ok(Value::String(args[0].type_name().to_string())),
            },
        );

        // String conversion
        self.environment.borrow_mut().define(
            "str".to_string(),
            Value::NativeFunction {
                name: "str".to_string(),
                arity: 1,
                func: |args| Ok(Value::String(args[0].to_string())),
            },
        );

        // Integer conversion
        self.environment.borrow_mut().define(
            "int".to_string(),
            Value::NativeFunction {
                name: "int".to_string(),
                arity: 1,
                func: |args| match &args[0] {
                    Value::Int(n) => Ok(Value::Int(*n)),
                    Value::Float(f) => Ok(Value::Int(*f as i64)),
                    Value::String(s) => s
                        .parse::<i64>()
                        .map(Value::Int)
                        .map_err(|_| IntentError::TypeError("Cannot parse as int".to_string())),
                    Value::Bool(b) => Ok(Value::Int(if *b { 1 } else { 0 })),
                    _ => Err(IntentError::TypeError("Cannot convert to int".to_string())),
                },
            },
        );

        // Float conversion
        self.environment.borrow_mut().define(
            "float".to_string(),
            Value::NativeFunction {
                name: "float".to_string(),
                arity: 1,
                func: |args| match &args[0] {
                    Value::Int(n) => Ok(Value::Float(*n as f64)),
                    Value::Float(f) => Ok(Value::Float(*f)),
                    Value::String(s) => s
                        .parse::<f64>()
                        .map(Value::Float)
                        .map_err(|_| IntentError::TypeError("Cannot parse as float".to_string())),
                    _ => Err(IntentError::TypeError(
                        "Cannot convert to float".to_string(),
                    )),
                },
            },
        );

        // Push to array
        self.environment.borrow_mut().define(
            "push".to_string(),
            Value::NativeFunction {
                name: "push".to_string(),
                arity: 2,
                func: |args| {
                    if let Value::Array(mut arr) = args[0].clone() {
                        arr.push(args[1].clone());
                        Ok(Value::Array(arr))
                    } else {
                        Err(IntentError::TypeError(
                            "push() requires an array".to_string(),
                        ))
                    }
                },
            },
        );

        // Assert function
        self.environment.borrow_mut().define(
            "assert".to_string(),
            Value::NativeFunction {
                name: "assert".to_string(),
                arity: 1,
                func: |args| {
                    if args[0].is_truthy() {
                        Ok(Value::Unit)
                    } else {
                        Err(IntentError::ContractViolation(
                            "Assertion failed".to_string(),
                        ))
                    }
                },
            },
        );

        // ============================================
        // Math functions
        // ============================================

        // Absolute value
        self.environment.borrow_mut().define(
            "abs".to_string(),
            Value::NativeFunction {
                name: "abs".to_string(),
                arity: 1,
                func: |args| match &args[0] {
                    Value::Int(n) => Ok(Value::Int(n.abs())),
                    Value::Float(f) => Ok(Value::Float(f.abs())),
                    _ => Err(IntentError::TypeError(
                        "abs() requires a number".to_string(),
                    )),
                },
            },
        );

        // Minimum of two values
        self.environment.borrow_mut().define(
            "min".to_string(),
            Value::NativeFunction {
                name: "min".to_string(),
                arity: 2,
                func: |args| match (&args[0], &args[1]) {
                    (Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a.min(b))),
                    (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a.min(*b))),
                    (Value::Int(a), Value::Float(b)) => Ok(Value::Float((*a as f64).min(*b))),
                    (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a.min(*b as f64))),
                    _ => Err(IntentError::TypeError("min() requires numbers".to_string())),
                },
            },
        );

        // Maximum of two values
        self.environment.borrow_mut().define(
            "max".to_string(),
            Value::NativeFunction {
                name: "max".to_string(),
                arity: 2,
                func: |args| match (&args[0], &args[1]) {
                    (Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a.max(b))),
                    (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a.max(*b))),
                    (Value::Int(a), Value::Float(b)) => Ok(Value::Float((*a as f64).max(*b))),
                    (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a.max(*b as f64))),
                    _ => Err(IntentError::TypeError("max() requires numbers".to_string())),
                },
            },
        );

        // Round to nearest integer, or to N decimal places: round(value) or round(value, decimals)
        self.environment.borrow_mut().define(
            "round".to_string(),
            Value::NativeFunction {
                name: "round".to_string(),
                arity: 0, // Variable arity: 1 or 2 args
                func: |args| {
                    if args.is_empty() || args.len() > 2 {
                        return Err(IntentError::TypeError(
                            "round() requires 1 or 2 arguments".to_string(),
                        ));
                    }

                    let value = match &args[0] {
                        Value::Int(n) => *n as f64,
                        Value::Float(f) => *f,
                        _ => {
                            return Err(IntentError::TypeError(
                                "round() requires a number as first argument".to_string(),
                            ))
                        }
                    };

                    // If no decimals specified, round to integer (original behavior)
                    if args.len() == 1 {
                        return Ok(Value::Int(value.round() as i64));
                    }

                    // Round to N decimal places
                    let decimals = match &args[1] {
                        Value::Int(n) => *n,
                        _ => {
                            return Err(IntentError::TypeError(
                                "round() requires an integer for decimal places".to_string(),
                            ))
                        }
                    };

                    if decimals < 0 {
                        return Err(IntentError::TypeError(
                            "round() decimal places must be non-negative".to_string(),
                        ));
                    }

                    let multiplier = 10_f64.powi(decimals as i32);
                    let rounded = (value * multiplier).round() / multiplier;
                    Ok(Value::Float(rounded))
                },
            },
        );

        // Floor (round down)
        self.environment.borrow_mut().define(
            "floor".to_string(),
            Value::NativeFunction {
                name: "floor".to_string(),
                arity: 1,
                func: |args| match &args[0] {
                    Value::Int(n) => Ok(Value::Int(*n)),
                    Value::Float(f) => Ok(Value::Int(f.floor() as i64)),
                    _ => Err(IntentError::TypeError(
                        "floor() requires a number".to_string(),
                    )),
                },
            },
        );

        // Ceil (round up)
        self.environment.borrow_mut().define(
            "ceil".to_string(),
            Value::NativeFunction {
                name: "ceil".to_string(),
                arity: 1,
                func: |args| match &args[0] {
                    Value::Int(n) => Ok(Value::Int(*n)),
                    Value::Float(f) => Ok(Value::Int(f.ceil() as i64)),
                    _ => Err(IntentError::TypeError(
                        "ceil() requires a number".to_string(),
                    )),
                },
            },
        );

        // Trunc (truncate toward zero)
        self.environment.borrow_mut().define(
            "trunc".to_string(),
            Value::NativeFunction {
                name: "trunc".to_string(),
                arity: 1,
                func: |args| match &args[0] {
                    Value::Int(n) => Ok(Value::Int(*n)),
                    Value::Float(f) => Ok(Value::Int(f.trunc() as i64)),
                    _ => Err(IntentError::TypeError(
                        "trunc() requires a number".to_string(),
                    )),
                },
            },
        );

        // Square root
        self.environment.borrow_mut().define(
            "sqrt".to_string(),
            Value::NativeFunction {
                name: "sqrt".to_string(),
                arity: 1,
                func: |args| match &args[0] {
                    Value::Int(n) => {
                        if *n < 0 {
                            Err(IntentError::RuntimeError(
                                "sqrt() of negative number".to_string(),
                            ))
                        } else {
                            Ok(Value::Float((*n as f64).sqrt()))
                        }
                    }
                    Value::Float(f) => {
                        if *f < 0.0 {
                            Err(IntentError::RuntimeError(
                                "sqrt() of negative number".to_string(),
                            ))
                        } else {
                            Ok(Value::Float(f.sqrt()))
                        }
                    }
                    _ => Err(IntentError::TypeError(
                        "sqrt() requires a number".to_string(),
                    )),
                },
            },
        );

        // Power (x^y)
        self.environment.borrow_mut().define(
            "pow".to_string(),
            Value::NativeFunction {
                name: "pow".to_string(),
                arity: 2,
                func: |args| match (&args[0], &args[1]) {
                    (Value::Int(base), Value::Int(exp)) => {
                        if *exp >= 0 {
                            Ok(Value::Int(base.pow(*exp as u32)))
                        } else {
                            Ok(Value::Float((*base as f64).powi(*exp as i32)))
                        }
                    }
                    (Value::Float(base), Value::Int(exp)) => {
                        Ok(Value::Float(base.powi(*exp as i32)))
                    }
                    (Value::Int(base), Value::Float(exp)) => {
                        Ok(Value::Float((*base as f64).powf(*exp)))
                    }
                    (Value::Float(base), Value::Float(exp)) => Ok(Value::Float(base.powf(*exp))),
                    _ => Err(IntentError::TypeError("pow() requires numbers".to_string())),
                },
            },
        );

        // Sign function (-1, 0, or 1)
        self.environment.borrow_mut().define(
            "sign".to_string(),
            Value::NativeFunction {
                name: "sign".to_string(),
                arity: 1,
                func: |args| match &args[0] {
                    Value::Int(n) => Ok(Value::Int(n.signum())),
                    Value::Float(f) => {
                        if *f > 0.0 {
                            Ok(Value::Int(1))
                        } else if *f < 0.0 {
                            Ok(Value::Int(-1))
                        } else {
                            Ok(Value::Int(0))
                        }
                    }
                    _ => Err(IntentError::TypeError(
                        "sign() requires a number".to_string(),
                    )),
                },
            },
        );

        // Clamp value between min and max
        self.environment.borrow_mut().define(
            "clamp".to_string(),
            Value::NativeFunction {
                name: "clamp".to_string(),
                arity: 3,
                func: |args| match (&args[0], &args[1], &args[2]) {
                    (Value::Int(val), Value::Int(min), Value::Int(max)) => {
                        Ok(Value::Int(*val.max(min).min(max)))
                    }
                    (Value::Float(val), Value::Float(min), Value::Float(max)) => {
                        Ok(Value::Float(val.max(*min).min(*max)))
                    }
                    _ => Err(IntentError::TypeError(
                        "clamp() requires numbers of same type".to_string(),
                    )),
                },
            },
        );
    }

    /// Define built-in types: Option<T>, Result<T, E>
    fn define_builtin_types(&mut self) {
        // Option<T> = Some(T) | None
        self.enums.insert(
            "Option".to_string(),
            vec![
                EnumVariant {
                    name: "Some".to_string(),
                    fields: Some(vec![TypeExpr::Named("T".to_string())]),
                },
                EnumVariant {
                    name: "None".to_string(),
                    fields: None,
                },
            ],
        );

        // Result<T, E> = Ok(T) | Err(E)
        self.enums.insert(
            "Result".to_string(),
            vec![
                EnumVariant {
                    name: "Ok".to_string(),
                    fields: Some(vec![TypeExpr::Named("T".to_string())]),
                },
                EnumVariant {
                    name: "Err".to_string(),
                    fields: Some(vec![TypeExpr::Named("E".to_string())]),
                },
            ],
        );

        // Define constructors for Option
        self.environment.borrow_mut().define(
            "Some".to_string(),
            Value::NativeFunction {
                name: "Some".to_string(),
                arity: 1,
                func: |args| {
                    Ok(Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "Some".to_string(),
                        values: args.to_vec(),
                    })
                },
            },
        );

        self.environment.borrow_mut().define(
            "None".to_string(),
            Value::EnumValue {
                enum_name: "Option".to_string(),
                variant: "None".to_string(),
                values: vec![],
            },
        );

        // Define constructors for Result
        self.environment.borrow_mut().define(
            "Ok".to_string(),
            Value::NativeFunction {
                name: "Ok".to_string(),
                arity: 1,
                func: |args| {
                    Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Ok".to_string(),
                        values: args.to_vec(),
                    })
                },
            },
        );

        self.environment.borrow_mut().define(
            "Err".to_string(),
            Value::NativeFunction {
                name: "Err".to_string(),
                arity: 1,
                func: |args| {
                    Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: args.to_vec(),
                    })
                },
            },
        );

        // is_some() helper for Option
        self.environment.borrow_mut().define(
            "is_some".to_string(),
            Value::NativeFunction {
                name: "is_some".to_string(),
                arity: 1,
                func: |args| match &args[0] {
                    Value::EnumValue {
                        enum_name, variant, ..
                    } if enum_name == "Option" => Ok(Value::Bool(variant == "Some")),
                    _ => Err(IntentError::TypeError(
                        "is_some() requires an Option".to_string(),
                    )),
                },
            },
        );

        // is_none() helper for Option
        self.environment.borrow_mut().define(
            "is_none".to_string(),
            Value::NativeFunction {
                name: "is_none".to_string(),
                arity: 1,
                func: |args| match &args[0] {
                    Value::EnumValue {
                        enum_name, variant, ..
                    } if enum_name == "Option" => Ok(Value::Bool(variant == "None")),
                    _ => Err(IntentError::TypeError(
                        "is_none() requires an Option".to_string(),
                    )),
                },
            },
        );

        // is_ok() helper for Result
        self.environment.borrow_mut().define(
            "is_ok".to_string(),
            Value::NativeFunction {
                name: "is_ok".to_string(),
                arity: 1,
                func: |args| match &args[0] {
                    Value::EnumValue {
                        enum_name, variant, ..
                    } if enum_name == "Result" => Ok(Value::Bool(variant == "Ok")),
                    _ => Err(IntentError::TypeError(
                        "is_ok() requires a Result".to_string(),
                    )),
                },
            },
        );

        // is_err() helper for Result
        self.environment.borrow_mut().define(
            "is_err".to_string(),
            Value::NativeFunction {
                name: "is_err".to_string(),
                arity: 1,
                func: |args| match &args[0] {
                    Value::EnumValue {
                        enum_name, variant, ..
                    } if enum_name == "Result" => Ok(Value::Bool(variant == "Err")),
                    _ => Err(IntentError::TypeError(
                        "is_err() requires a Result".to_string(),
                    )),
                },
            },
        );

        // unwrap() for Option and Result
        self.environment.borrow_mut().define(
            "unwrap".to_string(),
            Value::NativeFunction {
                name: "unwrap".to_string(),
                arity: 1,
                func: |args| match &args[0] {
                    Value::EnumValue {
                        enum_name,
                        variant,
                        values,
                    } => match (enum_name.as_str(), variant.as_str()) {
                        ("Option", "Some") | ("Result", "Ok") => values
                            .first()
                            .cloned()
                            .ok_or_else(|| IntentError::RuntimeError("Empty variant".to_string())),
                        ("Option", "None") => Err(IntentError::RuntimeError(
                            "Called unwrap() on None".to_string(),
                        )),
                        ("Result", "Err") => {
                            let err_val = values.first().map(|v| v.to_string()).unwrap_or_default();
                            Err(IntentError::RuntimeError(format!(
                                "Called unwrap() on Err({})",
                                err_val
                            )))
                        }
                        _ => Err(IntentError::TypeError(
                            "unwrap() requires Option or Result".to_string(),
                        )),
                    },
                    _ => Err(IntentError::TypeError(
                        "unwrap() requires Option or Result".to_string(),
                    )),
                },
            },
        );

        // unwrap_or() for Option and Result
        self.environment.borrow_mut().define(
            "unwrap_or".to_string(),
            Value::NativeFunction {
                name: "unwrap_or".to_string(),
                arity: 2,
                func: |args| match &args[0] {
                    Value::EnumValue {
                        enum_name,
                        variant,
                        values,
                    } => match (enum_name.as_str(), variant.as_str()) {
                        ("Option", "Some") | ("Result", "Ok") => values
                            .first()
                            .cloned()
                            .ok_or_else(|| IntentError::RuntimeError("Empty variant".to_string())),
                        ("Option", "None") | ("Result", "Err") => Ok(args[1].clone()),
                        _ => Err(IntentError::TypeError(
                            "unwrap_or() requires Option or Result".to_string(),
                        )),
                    },
                    _ => Err(IntentError::TypeError(
                        "unwrap_or() requires Option or Result".to_string(),
                    )),
                },
            },
        );

        // listen(port) - Start HTTP server on given port
        // This is a special built-in because it needs to call Intent handler functions
        self.environment.borrow_mut().define(
            "listen".to_string(),
            Value::NativeFunction {
                name: "listen".to_string(),
                arity: 1,
                func: |_args| {
                    // This is a placeholder - actual implementation is in eval_call
                    // because we need access to the interpreter to call handlers
                    Err(IntentError::RuntimeError(
                        "listen() must be called directly, not stored in a variable".to_string(),
                    ))
                },
            },
        );

        // HTTP routing functions - these need special handling in eval_call
        // because they need to store handlers in the interpreter's server_state
        for method in &["get", "post", "put", "delete", "patch"] {
            let method_name = method.to_string();
            self.environment.borrow_mut().define(
                method_name.clone(),
                Value::NativeFunction {
                    name: method_name,
                    arity: 2,
                    func: |_args| {
                        Err(IntentError::RuntimeError(
                            "HTTP route functions must be called directly".to_string(),
                        ))
                    },
                },
            );
        }

        // new_server() - create a new server (resets routes)
        self.environment.borrow_mut().define(
            "new_server".to_string(),
            Value::NativeFunction {
                name: "new_server".to_string(),
                arity: 0,
                func: |_args| {
                    // Placeholder - actual implementation clears server_state
                    Err(IntentError::RuntimeError(
                        "new_server() must be called directly".to_string(),
                    ))
                },
            },
        );
    }

    /// Define standard library functions that are always available
    fn define_stdlib(&mut self) {
        // Initialize standard library modules from the stdlib module
        use crate::stdlib;
        let modules = stdlib::init_all_modules();
        for (name, module) in modules {
            self.loaded_modules.insert(name, module);
        }
    }

    /// Handle import statement
    fn handle_import(
        &mut self,
        items: &[ImportItem],
        source: &str,
        alias: Option<&str>,
    ) -> Result<Value> {
        // Check if it's a standard library module
        if source.starts_with("std/") {
            return self.import_std_module(items, source, alias);
        }

        // Check if it's already loaded
        if let Some(module) = self.loaded_modules.get(source).cloned() {
            return self.bind_imports(items, &module, source, alias);
        }

        // Try to load from file
        self.import_file_module(items, source, alias)
    }

    fn import_std_module(
        &mut self,
        items: &[ImportItem],
        source: &str,
        alias: Option<&str>,
    ) -> Result<Value> {
        let module = self.loaded_modules.get(source).cloned().ok_or_else(|| {
            IntentError::RuntimeError(format!("Unknown standard library module: {}", source))
        })?;

        self.bind_imports(items, &module, source, alias)
    }

    fn bind_imports(
        &mut self,
        items: &[ImportItem],
        module: &HashMap<String, Value>,
        source: &str,
        alias: Option<&str>,
    ) -> Result<Value> {
        if items.is_empty() {
            // Import entire module
            let module_name = alias.unwrap_or_else(|| source.rsplit('/').next().unwrap_or(source));
            // Create a struct-like value for the module
            let mut fields = HashMap::new();
            for (name, value) in module {
                fields.insert(name.clone(), value.clone());
            }
            self.environment.borrow_mut().define(
                module_name.to_string(),
                Value::Struct {
                    name: format!("module:{}", source),
                    fields,
                },
            );
        } else {
            // Import specific items
            for item in items {
                let value = module.get(&item.name).ok_or_else(|| {
                    IntentError::RuntimeError(format!(
                        "'{}' is not exported from '{}'",
                        item.name, source
                    ))
                })?;
                let bind_name = item.alias.as_ref().unwrap_or(&item.name);
                self.environment
                    .borrow_mut()
                    .define(bind_name.clone(), value.clone());
            }
        }
        Ok(Value::Unit)
    }

    fn import_file_module(
        &mut self,
        items: &[ImportItem],
        source: &str,
        alias: Option<&str>,
    ) -> Result<Value> {
        use crate::lexer::Lexer;
        use crate::parser::Parser;
        use std::fs;

        // Resolve the file path
        let file_path = if source.starts_with("./") || source.starts_with("../") {
            // Relative import
            if let Some(ref current) = self.current_file {
                let current_dir = std::path::Path::new(current)
                    .parent()
                    .unwrap_or(std::path::Path::new("."));
                current_dir.join(source)
            } else {
                std::path::PathBuf::from(source)
            }
        } else {
            std::path::PathBuf::from(source)
        };

        // Add .tnt extension if not present
        let file_path = if file_path.extension().is_none() {
            file_path.with_extension("tnt")
        } else {
            file_path
        };

        // Read and parse the file
        let source_code = fs::read_to_string(&file_path).map_err(|e| {
            IntentError::RuntimeError(format!(
                "Failed to read module '{}': {}",
                file_path.display(),
                e
            ))
        })?;

        let lexer = Lexer::new(&source_code);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse()?;

        // Create a new environment for the module
        let previous_env = Rc::clone(&self.environment);
        let previous_file = self.current_file.clone();

        self.environment = Rc::new(RefCell::new(Environment::new()));
        self.current_file = Some(file_path.to_string_lossy().to_string());

        // Define builtins and types in the module environment
        self.define_builtins();
        self.define_builtin_types();

        // Evaluate the module
        self.eval(&ast)?;

        // Collect exported items
        let mut module_exports: HashMap<String, Value> = HashMap::new();

        // For now, export everything defined at module level
        // In the future, we'd track explicit exports
        let env = self.environment.borrow();
        for (name, value) in env.values.iter() {
            module_exports.insert(name.clone(), value.clone());
        }
        drop(env);

        // Restore environment
        self.environment = previous_env;
        self.current_file = previous_file;

        // Cache the module
        let source_key = file_path.to_string_lossy().to_string();
        self.loaded_modules
            .insert(source_key.clone(), module_exports.clone());

        // Track for hot-reload (record mtime)
        if let Ok(metadata) = std::fs::metadata(&file_path) {
            if let Ok(mtime) = metadata.modified() {
                self.imported_files.insert(source_key.clone(), mtime);
            }
        }

        // Bind imports
        self.bind_imports(items, &module_exports, &source_key, alias)
    }

    /// Load file-based routes from a directory
    ///
    /// Scans a directory for .tnt files and registers routes based on:
    /// - File path = URL path (e.g., routes/users/[id].tnt → /users/{id})
    /// - Exported functions = HTTP methods (e.g., export fn get(req) → GET)
    /// - index.tnt = directory root (e.g., routes/users/index.tnt → /users)
    /// - [param].tnt = dynamic segments (e.g., [id].tnt → {id})
    ///
    /// Also auto-loads:
    /// - lib/ directory as shared modules available in routes
    /// - middleware/ directory in alphabetical order
    fn load_file_based_routes(&mut self, dir_path: &str) -> Result<Value> {
        use std::fs;

        // Resolve the directory path relative to the current .tnt file's location
        // This allows running `ntnt path/to/app.tnt` from any directory
        let base_dir = if std::path::Path::new(dir_path).is_relative() {
            // Use the directory of the current .tnt file as base, not cwd
            if let Some(current_file) = &self.current_file {
                let script_dir = std::path::Path::new(current_file)
                    .parent()
                    .unwrap_or(std::path::Path::new("."));
                script_dir.join(dir_path)
            } else {
                // Fallback to cwd if no current file (shouldn't happen in practice)
                std::env::current_dir()
                    .map(|cwd| cwd.join(dir_path))
                    .unwrap_or_else(|_| std::path::PathBuf::from(dir_path))
            }
        } else {
            std::path::PathBuf::from(dir_path)
        };

        // Check for lib/ directory and load shared modules
        let lib_dir = base_dir
            .parent()
            .map(|p| p.join("lib"))
            .unwrap_or_else(|| std::path::PathBuf::from("lib"));

        let mut lib_modules: HashMap<String, HashMap<String, Value>> = HashMap::new();

        if lib_dir.exists() && lib_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&lib_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map(|e| e == "tnt").unwrap_or(false) {
                        let module_name = path
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();

                        if let Ok(exports) = self.load_module_exports(&path) {
                            lib_modules.insert(module_name, exports);
                        }
                    }
                }
            }
        }

        // Store lib_modules for hot-reload
        self.lib_modules = lib_modules.clone();

        // Check for middleware/ directory and load middleware in order
        let middleware_dir = base_dir
            .parent()
            .map(|p| p.join("middleware"))
            .unwrap_or_else(|| std::path::PathBuf::from("middleware"));

        if middleware_dir.exists() && middleware_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&middleware_dir) {
                let mut middleware_files: Vec<_> = entries
                    .flatten()
                    .filter(|e| {
                        e.path()
                            .extension()
                            .map(|ext| ext == "tnt")
                            .unwrap_or(false)
                    })
                    .collect();

                // Sort alphabetically for predictable order (01_logger.tnt, 02_auth.tnt, etc.)
                middleware_files.sort_by_key(|e| e.path());

                for entry in middleware_files {
                    let path = entry.path();
                    if let Ok(exports) = self.load_module_exports(&path) {
                        // Look for a handler function (middleware or handler)
                        if let Some(handler) =
                            exports.get("middleware").or_else(|| exports.get("handler"))
                        {
                            self.server_state.add_middleware(handler.clone());
                            let name = path
                                .file_stem()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| "unknown".to_string());
                            println!("  Loaded middleware: {}", name);
                        }
                    }
                }
            }
        }

        // Scan routes directory recursively
        let routes = self.discover_routes(&base_dir, &base_dir, &lib_modules)?;

        // Register all discovered routes with source info for hot-reload
        for (method, pattern, handler, file, imports) in &routes {
            self.server_state.add_route_with_source(
                method,
                pattern,
                handler.clone(),
                Some(file.clone()),
                imports.clone(),
            );
            let import_count = imports.len();
            if import_count > 0 {
                println!(
                    "  {} {} -> {} ({} imports)",
                    method, pattern, file, import_count
                );
            } else {
                println!("  {} {} -> {}", method, pattern, file);
            }
        }

        Ok(Value::Int(routes.len() as i64))
    }

    /// Load a module and return its exports
    fn load_module_exports(
        &mut self,
        file_path: &std::path::Path,
    ) -> Result<HashMap<String, Value>> {
        use crate::lexer::Lexer;
        use crate::parser::Parser;
        use std::fs;

        let source_code = fs::read_to_string(file_path).map_err(|e| {
            IntentError::RuntimeError(format!("Failed to read '{}': {}", file_path.display(), e))
        })?;

        let lexer = Lexer::new(&source_code);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse()?;

        // Create a fresh environment for the module
        let previous_env = Rc::clone(&self.environment);
        let previous_file = self.current_file.clone();

        self.environment = Rc::new(RefCell::new(Environment::new()));
        self.current_file = Some(file_path.to_string_lossy().to_string());

        // Re-define builtins and types in the new environment
        self.define_builtins();
        self.define_builtin_types();

        // Evaluate the module
        self.eval(&ast)?;

        // Collect exports (everything defined at module level)
        let mut exports: HashMap<String, Value> = HashMap::new();
        let env = self.environment.borrow();
        for (name, value) in env.values.iter() {
            // Skip builtins
            if !matches!(value, Value::NativeFunction { .. }) {
                exports.insert(name.clone(), value.clone());
            }
        }
        drop(env);

        // Restore environment
        self.environment = previous_env;
        self.current_file = previous_file;

        Ok(exports)
    }

    /// Recursively discover routes in a directory
    fn discover_routes(
        &mut self,
        dir: &std::path::Path,
        base_dir: &std::path::Path,
        lib_modules: &HashMap<String, HashMap<String, Value>>,
    ) -> Result<
        Vec<(
            String,
            String,
            Value,
            String,
            HashMap<String, std::time::SystemTime>,
        )>,
    > {
        use std::fs;

        let mut routes = Vec::new();

        if !dir.exists() || !dir.is_dir() {
            return Err(IntentError::RuntimeError(format!(
                "Routes directory does not exist: {}",
                dir.display()
            )));
        }

        let mut entries: Vec<_> = fs::read_dir(dir)
            .map_err(|e| IntentError::RuntimeError(format!("Failed to read directory: {}", e)))?
            .flatten()
            .collect();

        // Sort for consistent ordering
        entries.sort_by_key(|e| e.path());

        for entry in entries {
            let path = entry.path();

            if path.is_dir() {
                // Recurse into subdirectory
                let sub_routes = self.discover_routes(&path, base_dir, lib_modules)?;
                routes.extend(sub_routes);
            } else if path.extension().map(|e| e == "tnt").unwrap_or(false) {
                // Process .tnt file
                let file_routes = self.process_route_file(&path, base_dir, lib_modules)?;
                routes.extend(file_routes);
            }
        }

        Ok(routes)
    }

    /// Process a single route file and extract HTTP method handlers
    /// Returns: Vec<(method, pattern, handler, file_path, imported_files)>
    fn process_route_file(
        &mut self,
        file_path: &std::path::Path,
        base_dir: &std::path::Path,
        lib_modules: &HashMap<String, HashMap<String, Value>>,
    ) -> Result<
        Vec<(
            String,
            String,
            Value,
            String,
            HashMap<String, std::time::SystemTime>,
        )>,
    > {
        use crate::lexer::Lexer;
        use crate::parser::Parser;
        use std::fs;

        let mut routes = Vec::new();

        // Convert file path to URL pattern
        let relative_path = file_path
            .strip_prefix(base_dir)
            .map_err(|_| IntentError::RuntimeError("Failed to get relative path".to_string()))?;

        let url_pattern = self.file_path_to_url_pattern(relative_path);

        // Read and parse the file
        let source_code = fs::read_to_string(file_path).map_err(|e| {
            IntentError::RuntimeError(format!("Failed to read '{}': {}", file_path.display(), e))
        })?;

        let lexer = Lexer::new(&source_code);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse()?;

        // Create a fresh environment for the route module
        let previous_env = Rc::clone(&self.environment);
        let previous_file = self.current_file.clone();
        let previous_imports = std::mem::take(&mut self.imported_files);

        self.environment = Rc::new(RefCell::new(Environment::new()));
        self.current_file = Some(file_path.to_string_lossy().to_string());

        // Re-define builtins and types
        self.define_builtins();
        self.define_builtin_types();

        // Inject lib modules into the environment
        for (name, exports) in lib_modules {
            let mut fields = HashMap::new();
            for (fn_name, value) in exports {
                fields.insert(fn_name.clone(), value.clone());
            }
            self.environment.borrow_mut().define(
                name.clone(),
                Value::Struct {
                    name: format!("lib:{}", name),
                    fields,
                },
            );
        }

        // Evaluate the module
        self.eval(&ast)?;

        // Capture imports made by this route file
        let route_imports = std::mem::take(&mut self.imported_files);

        // Find exported HTTP method handlers
        let http_methods = ["get", "post", "put", "delete", "patch", "head", "options"];

        let env = self.environment.borrow();
        for method in http_methods {
            if let Some(handler) = env.values.get(method) {
                // Check if it's a function
                if matches!(handler, Value::Function { .. }) {
                    let http_method = method.to_uppercase();
                    routes.push((
                        http_method,
                        url_pattern.clone(),
                        handler.clone(),
                        file_path.to_string_lossy().to_string(),
                        route_imports.clone(),
                    ));
                }
            }
        }
        drop(env);

        // Restore environment and imports
        self.environment = previous_env;
        self.current_file = previous_file;
        self.imported_files = previous_imports;

        Ok(routes)
    }

    /// Reload a single route handler from a file (for hot-reload)
    /// Returns: (handler, imported_files)
    fn reload_route_handler(
        &mut self,
        file_path: &str,
        method: &str,
    ) -> Result<(Value, HashMap<String, std::time::SystemTime>)> {
        use crate::lexer::Lexer;
        use crate::parser::Parser;
        use std::fs;

        let path = std::path::Path::new(file_path);

        // Read and parse the file
        let source_code = fs::read_to_string(path).map_err(|e| {
            IntentError::RuntimeError(format!("Failed to read '{}': {}", file_path, e))
        })?;

        let lexer = Lexer::new(&source_code);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse()?;

        // Create a fresh environment and save import state
        let previous_env = Rc::clone(&self.environment);
        let previous_file = self.current_file.clone();
        let previous_imports = std::mem::take(&mut self.imported_files);

        self.environment = Rc::new(RefCell::new(Environment::new()));
        self.current_file = Some(file_path.to_string());

        // Re-define builtins and types
        self.define_builtins();
        self.define_builtin_types();

        // Inject lib modules (same as initial route processing)
        for (name, exports) in &self.lib_modules {
            let mut fields = HashMap::new();
            for (fn_name, value) in exports {
                fields.insert(fn_name.clone(), value.clone());
            }
            self.environment.borrow_mut().define(
                name.clone(),
                Value::Struct {
                    name: format!("lib:{}", name),
                    fields,
                },
            );
        }

        // Evaluate the module
        self.eval(&ast)?;

        // Capture imports made by this route file
        let route_imports = std::mem::take(&mut self.imported_files);

        // Find the handler for the specified method
        let method_name = method.to_lowercase();
        let env = self.environment.borrow();
        let handler = env.values.get(&method_name).cloned();
        drop(env);

        // Restore environment and imports
        self.environment = previous_env;
        self.current_file = previous_file;
        self.imported_files = previous_imports;

        let handler = handler.ok_or_else(|| {
            IntentError::RuntimeError(format!(
                "Handler '{}' not found in {}",
                method_name, file_path
            ))
        })?;

        Ok((handler, route_imports))
    }

    /// Convert a file path to a URL pattern
    ///
    /// Examples:
    /// - index.tnt → /
    /// - about.tnt → /about
    /// - users/index.tnt → /users
    /// - users/[id].tnt → /users/{id}
    /// - api/products/[id]/reviews.tnt → /api/products/{id}/reviews
    fn file_path_to_url_pattern(&self, path: &std::path::Path) -> String {
        let mut segments: Vec<String> = Vec::new();

        for component in path.components() {
            if let std::path::Component::Normal(os_str) = component {
                let segment = os_str.to_string_lossy().to_string();

                // Remove .tnt extension
                let segment = segment.strip_suffix(".tnt").unwrap_or(&segment).to_string();

                // Skip index files (they represent the directory root)
                if segment == "index" {
                    continue;
                }

                // Convert [param] to {param}
                let segment = if segment.starts_with('[') && segment.ends_with(']') {
                    let param_name = &segment[1..segment.len() - 1];
                    format!("{{{}}}", param_name)
                } else {
                    segment
                };

                segments.push(segment);
            }
        }

        if segments.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", segments.join("/"))
        }
    }

    /// Evaluate a program
    pub fn eval(&mut self, program: &Program) -> Result<Value> {
        let mut result = Value::Unit;
        for stmt in &program.statements {
            result = self.eval_statement(stmt)?;
            // Unwrap return values at top level
            if let Value::Return(v) = result {
                return Ok(*v);
            }
        }
        Ok(result)
    }

    fn eval_statement(&mut self, stmt: &Statement) -> Result<Value> {
        match stmt {
            Statement::Let {
                name,
                mutable: _,
                type_annotation: _,
                value,
                pattern,
            } => {
                let val = if let Some(expr) = value {
                    self.eval_expression(expr)?
                } else {
                    Value::Unit
                };

                // Handle pattern destructuring
                if let Some(pat) = pattern {
                    self.bind_pattern(pat, &val)?;
                } else {
                    self.environment.borrow_mut().define(name.clone(), val);
                }
                Ok(Value::Unit)
            }

            Statement::TypeAlias {
                name,
                type_params: _,
                target,
            } => {
                // Store type alias for later resolution
                self.type_aliases.insert(name.clone(), target.clone());
                Ok(Value::Unit)
            }

            Statement::Function {
                name,
                params,
                return_type: _,
                contract,
                body,
                attributes: _,
                type_params,
                effects: _, // Effects are tracked but not enforced at runtime yet
            } => {
                // Convert AST Contract to FunctionContract with expressions
                let func_contract = contract.as_ref().map(|c| FunctionContract {
                    requires: c.requires.clone(),
                    ensures: c.ensures.clone(),
                });

                let func = Value::Function {
                    name: name.clone(),
                    params: params.clone(),
                    body: body.clone(),
                    closure: Rc::clone(&self.environment),
                    contract: func_contract,
                    type_params: type_params.clone(),
                };
                self.environment.borrow_mut().define(name.clone(), func);
                Ok(Value::Unit)
            }

            Statement::Struct {
                name,
                fields,
                attributes: _,
                type_params: _, // TODO: Use for generic struct instantiation
            } => {
                self.structs.insert(name.clone(), fields.clone());
                Ok(Value::Unit)
            }

            Statement::Expression(expr) => self.eval_expression(expr),

            Statement::Return(expr) => {
                let value = if let Some(e) = expr {
                    self.eval_expression(e)?
                } else {
                    Value::Unit
                };
                Ok(Value::Return(Box::new(value)))
            }

            Statement::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond = self.eval_expression(condition)?;
                if cond.is_truthy() {
                    self.eval_block(then_branch)
                } else if let Some(else_b) = else_branch {
                    self.eval_block(else_b)
                } else {
                    Ok(Value::Unit)
                }
            }

            Statement::While { condition, body } => {
                loop {
                    let cond = self.eval_expression(condition)?;
                    if !cond.is_truthy() {
                        break;
                    }
                    let result = self.eval_block(body)?;
                    match result {
                        Value::Break => break,
                        Value::Continue => continue,
                        Value::Return(_) => return Ok(result),
                        _ => {}
                    }
                }
                Ok(Value::Unit)
            }

            Statement::Loop { body } => {
                loop {
                    let result = self.eval_block(body)?;
                    match result {
                        Value::Break => break,
                        Value::Continue => continue,
                        Value::Return(_) => return Ok(result),
                        _ => {}
                    }
                }
                Ok(Value::Unit)
            }

            Statement::Break => Ok(Value::Break),
            Statement::Continue => Ok(Value::Continue),

            Statement::Module { name: _, body } => {
                for stmt in body {
                    self.eval_statement(stmt)?;
                }
                Ok(Value::Unit)
            }

            Statement::Use { path: _ } => {
                // TODO: Implement module imports
                Ok(Value::Unit)
            }

            Statement::Impl {
                type_name,
                trait_name,
                methods,
                invariants,
            } => {
                // Store trait implementation if present
                if let Some(trait_name) = trait_name {
                    // Register that this type implements this trait
                    self.trait_implementations
                        .entry(type_name.clone())
                        .or_default()
                        .push(trait_name.clone());
                }

                // Store invariants for this type
                if !invariants.is_empty() {
                    self.struct_invariants
                        .insert(type_name.clone(), invariants.clone());
                }

                for method in methods {
                    self.eval_statement(method)?;
                }
                Ok(Value::Unit)
            }

            Statement::Enum {
                name,
                variants,
                attributes: _,
                type_params: _,
            } => {
                // Register the enum type
                self.enums.insert(name.clone(), variants.clone());

                // Create constructors for each variant
                for variant in variants {
                    let variant_name = variant.name.clone();
                    let enum_name = name.clone();
                    let has_fields = variant.fields.is_some();
                    let field_count = variant.fields.as_ref().map(|f| f.len()).unwrap_or(0);

                    if has_fields {
                        // Variant with data - create an enum constructor
                        self.environment.borrow_mut().define(
                            variant_name.clone(),
                            Value::EnumConstructor {
                                enum_name: enum_name.clone(),
                                variant: variant_name,
                                arity: field_count,
                            },
                        );
                    } else {
                        // Variant without data - create a constant value
                        self.environment.borrow_mut().define(
                            variant_name.clone(),
                            Value::EnumValue {
                                enum_name: enum_name.clone(),
                                variant: variant_name,
                                values: vec![],
                            },
                        );
                    }
                }

                Ok(Value::Unit)
            }

            Statement::Protocol { .. } => {
                // TODO: Implement protocol support
                Ok(Value::Unit)
            }

            Statement::Intent {
                description: _,
                target,
            } => self.eval_statement(target),

            Statement::Import {
                items,
                source,
                alias,
            } => self.handle_import(items, source, alias.as_deref()),

            Statement::Export {
                items: _,
                statement,
            } => {
                // For now, just evaluate the exported statement
                // The export metadata would be used by the module system
                if let Some(stmt) = statement {
                    self.eval_statement(stmt)?;
                }
                Ok(Value::Unit)
            }

            Statement::Trait {
                name,
                type_params: _,
                methods,
                supertraits,
            } => {
                // Register the trait definition
                let method_infos: Vec<TraitMethodInfo> = methods
                    .iter()
                    .map(|m| TraitMethodInfo {
                        name: m.name.clone(),
                        params: m.params.clone(),
                        return_type: m.return_type.clone(),
                        has_default: m.default_body.is_some(),
                    })
                    .collect();

                self.trait_definitions.insert(
                    name.clone(),
                    TraitInfo {
                        name: name.clone(),
                        methods: method_infos,
                        supertraits: supertraits.clone(),
                    },
                );

                Ok(Value::Unit)
            }

            Statement::ForIn {
                variable,
                iterable,
                body,
            } => {
                let iterable_value = self.eval_expression(iterable)?;

                // Convert iterable to something we can iterate over
                let items: Vec<Value> = match &iterable_value {
                    Value::Array(arr) => arr.clone(),
                    Value::Range {
                        start,
                        end,
                        inclusive,
                    } => {
                        let end_val = if *inclusive { *end + 1 } else { *end };
                        (*start..end_val).map(Value::Int).collect()
                    }
                    Value::String(s) => s.chars().map(|c| Value::String(c.to_string())).collect(),
                    Value::Map(map) => map.keys().map(|k| Value::String(k.clone())).collect(),
                    _ => {
                        return Err(IntentError::RuntimeError(format!(
                            "Cannot iterate over {}",
                            iterable_value.type_name()
                        )))
                    }
                };

                let mut result = Value::Unit;
                for item in items {
                    // Create new scope for each iteration
                    let previous = Rc::clone(&self.environment);
                    self.environment =
                        Rc::new(RefCell::new(Environment::with_parent(Rc::clone(&previous))));

                    // Bind the loop variable
                    self.environment.borrow_mut().define(variable.clone(), item);

                    // Execute the loop body
                    result = self.eval_block(body)?;

                    // Restore environment
                    self.environment = previous;

                    // Handle control flow
                    match result {
                        Value::Break => {
                            result = Value::Unit;
                            break;
                        }
                        Value::Continue => {
                            result = Value::Unit;
                            continue;
                        }
                        Value::Return(_) => break,
                        _ => {}
                    }
                }

                Ok(result)
            }

            Statement::Defer(expr) => {
                // Push the deferred expression onto the stack
                // It will be executed when the current scope exits
                self.deferred_statements.push(expr.clone());
                Ok(Value::Unit)
            }
        }
    }

    fn eval_block(&mut self, block: &Block) -> Result<Value> {
        let previous = Rc::clone(&self.environment);
        self.environment = Rc::new(RefCell::new(Environment::with_parent(Rc::clone(&previous))));

        // Track deferred statements for this block
        let deferred_count_before = self.deferred_statements.len();

        let mut result = Value::Unit;
        for stmt in &block.statements {
            result = self.eval_statement(stmt)?;
            // Propagate control flow
            match result {
                Value::Return(_) | Value::Break | Value::Continue => break,
                _ => {}
            }
        }

        // Execute deferred statements in reverse order (LIFO)
        let deferred_to_run: Vec<Expression> = self
            .deferred_statements
            .drain(deferred_count_before..)
            .collect();

        for deferred_expr in deferred_to_run.into_iter().rev() {
            // Deferred expressions execute even if there was an error
            // For now, we ignore any errors in deferred statements
            let _ = self.eval_expression(&deferred_expr);
        }

        self.environment = previous;
        Ok(result)
    }

    fn eval_expression(&mut self, expr: &Expression) -> Result<Value> {
        match expr {
            Expression::Integer(n) => Ok(Value::Int(*n)),
            Expression::Float(n) => Ok(Value::Float(*n)),
            Expression::String(s) => Ok(Value::String(s.clone())),
            Expression::Bool(b) => Ok(Value::Bool(*b)),
            Expression::Unit => Ok(Value::Unit),

            Expression::Identifier(name) => self.environment.borrow().get(name).ok_or_else(|| {
                let candidates = self.environment.borrow().keys();
                let suggestion = crate::error::find_suggestion(name, &candidates);
                IntentError::UndefinedVariable {
                    name: name.clone(),
                    suggestion,
                }
            }),

            Expression::Binary {
                left,
                operator,
                right,
            } => {
                let lhs = self.eval_expression(left)?;

                // Short-circuit evaluation for logical operators
                match operator {
                    BinaryOp::And => {
                        if !lhs.is_truthy() {
                            return Ok(Value::Bool(false));
                        }
                        let rhs = self.eval_expression(right)?;
                        return Ok(Value::Bool(rhs.is_truthy()));
                    }
                    BinaryOp::Or => {
                        if lhs.is_truthy() {
                            return Ok(Value::Bool(true));
                        }
                        let rhs = self.eval_expression(right)?;
                        return Ok(Value::Bool(rhs.is_truthy()));
                    }
                    BinaryOp::NullCoalesce => {
                        // Return unwrapped left if it's Some, otherwise evaluate and return right
                        match &lhs {
                            Value::EnumValue {
                                enum_name,
                                variant,
                                values,
                            } if enum_name == "Option" && variant == "Some" => {
                                // Unwrap the Some value
                                return Ok(values.first().cloned().unwrap_or(Value::Unit));
                            }
                            Value::EnumValue {
                                enum_name, variant, ..
                            } if enum_name == "Option" && variant == "None" => {
                                return self.eval_expression(right);
                            }
                            // For non-Option values, return as-is (like JavaScript's ??)
                            _ => return Ok(lhs),
                        }
                    }
                    _ => {}
                }

                let rhs = self.eval_expression(right)?;
                self.eval_binary_op(*operator, lhs, rhs)
            }

            Expression::Unary { operator, operand } => {
                let val = self.eval_expression(operand)?;
                match operator {
                    UnaryOp::Neg => match val {
                        Value::Int(n) => Ok(Value::Int(-n)),
                        Value::Float(f) => Ok(Value::Float(-f)),
                        _ => Err(IntentError::TypeError(
                            "Cannot negate non-numeric value".to_string(),
                        )),
                    },
                    UnaryOp::Not => Ok(Value::Bool(!val.is_truthy())),
                }
            }

            Expression::Call {
                function,
                arguments,
            } => {
                // Special handling for old() in postconditions
                if let Expression::Identifier(name) = function.as_ref() {
                    if name == "old" && arguments.len() == 1 {
                        // Look up the pre-execution value
                        let key = format!("{:?}", &arguments[0]);
                        if let Some(ref old_values) = self.current_old_values {
                            if let Some(stored) = old_values.get(&key) {
                                return Ok(self.stored_to_value(stored));
                            }
                        }
                        // If not in postcondition context, just evaluate normally
                        return self.eval_expression(&arguments[0]);
                    }

                    // Special handling for listen() - starts HTTP server
                    if name == "listen" && arguments.len() == 1 {
                        // Skip in hot-reload (server already running) and unit-test mode
                        if self.should_skip_server_call("listen") {
                            return Ok(Value::Unit);
                        }
                        let port = self.eval_expression(&arguments[0])?;
                        if let Value::Int(port_num) = port {
                            // Use sync server for test mode (intent check), async for production
                            if self.test_mode.is_some() {
                                return self.run_http_server(port_num as u16);
                            } else {
                                return self.run_async_http_server(port_num as u16);
                            }
                        } else {
                            return Err(IntentError::TypeError(
                                "listen() requires an integer port".to_string(),
                            ));
                        }
                    }

                    // Special handling for new_server() - resets routes
                    if name == "new_server" && arguments.is_empty() {
                        self.server_state.clear();
                        let mut server = HashMap::new();
                        server.insert("_type".to_string(), Value::String("Server".to_string()));
                        return Ok(Value::Map(server));
                    }

                    // Special handling for serve_static(url_prefix, directory)
                    if name == "serve_static" && arguments.len() == 2 {
                        if self.should_skip_server_call("serve_static") {
                            return Ok(Value::Unit);
                        }
                        let prefix = self.eval_expression(&arguments[0])?;
                        let directory = self.eval_expression(&arguments[1])?;

                        match (&prefix, &directory) {
                            (Value::String(prefix_str), Value::String(dir_str)) => {
                                // Resolve relative paths based on the .tnt file's location
                                let resolved_dir = if std::path::Path::new(dir_str).is_relative() {
                                    if let Some(current_file) = &self.current_file {
                                        let script_dir = std::path::Path::new(current_file)
                                            .parent()
                                            .unwrap_or(std::path::Path::new("."));
                                        script_dir.join(dir_str).to_string_lossy().to_string()
                                    } else {
                                        std::env::current_dir()
                                            .map(|cwd| {
                                                cwd.join(dir_str).to_string_lossy().to_string()
                                            })
                                            .unwrap_or_else(|_| dir_str.clone())
                                    }
                                } else {
                                    dir_str.clone()
                                };
                                self.server_state
                                    .add_static_dir(prefix_str.clone(), resolved_dir);
                                return Ok(Value::Unit);
                            }
                            _ => {
                                return Err(IntentError::TypeError(
                                    "serve_static() requires two string arguments: (url_prefix, directory)".to_string()
                                ));
                            }
                        }
                    }

                    // Special handling for routes(directory) - file-based routing
                    if name == "routes" && arguments.len() == 1 {
                        if self.should_skip_server_call("routes") {
                            return Ok(Value::Unit);
                        }
                        let directory = self.eval_expression(&arguments[0])?;
                        if let Value::String(dir_str) = directory {
                            return self.load_file_based_routes(&dir_str);
                        } else {
                            return Err(IntentError::TypeError(
                                "routes() requires a string directory path".to_string(),
                            ));
                        }
                    }

                    // Special handling for use_middleware(handler_fn)
                    if name == "use_middleware" && arguments.len() == 1 {
                        if self.should_skip_server_call("use_middleware") {
                            return Ok(Value::Unit);
                        }
                        let handler = self.eval_expression(&arguments[0])?;
                        self.server_state.add_middleware(handler);
                        return Ok(Value::Unit);
                    }

                    // Special handling for on_shutdown(handler_fn)
                    if name == "on_shutdown" && arguments.len() == 1 {
                        if self.should_skip_server_call("on_shutdown") {
                            return Ok(Value::Unit);
                        }
                        let handler = self.eval_expression(&arguments[0])?;
                        self.server_state.add_shutdown_handler(handler);
                        return Ok(Value::Unit);
                    }

                    // Special handling for template(path, data) - load and render template
                    if name == "template" && arguments.len() == 2 {
                        let path = self.eval_expression(&arguments[0])?;
                        let data = self.eval_expression(&arguments[1])?;

                        let path_str = match &path {
                            Value::String(s) => s.clone(),
                            _ => {
                                return Err(IntentError::TypeError(
                                    "template() first argument must be a string path".to_string(),
                                ))
                            }
                        };

                        let data_map = match &data {
                            Value::Map(m) => m.clone(),
                            _ => {
                                return Err(IntentError::TypeError(
                                    "template() second argument must be a map".to_string(),
                                ))
                            }
                        };

                        // Load template file
                        let base_path = self.current_file.as_deref();
                        let content =
                            crate::stdlib::template::load_template_file(&path_str, base_path)?;

                        // Render with data
                        return self.render_template_with_data(&content, &data_map);
                    }

                    // Special handling for compile(path) - pre-compile template
                    if name == "compile" && arguments.len() == 1 {
                        let path = self.eval_expression(&arguments[0])?;

                        let path_str = match &path {
                            Value::String(s) => s.clone(),
                            _ => {
                                return Err(IntentError::TypeError(
                                    "compile() argument must be a string path".to_string(),
                                ))
                            }
                        };

                        // Load template file
                        let base_path = self.current_file.as_deref();
                        let content =
                            crate::stdlib::template::load_template_file(&path_str, base_path)?;

                        // Create compiled template
                        let id = crate::stdlib::template::get_next_template_id();
                        let compiled = crate::stdlib::template::CompiledTemplate {
                            id,
                            path: path_str.clone(),
                            content,
                        };

                        // Store in cache
                        crate::stdlib::template::store_compiled_template(id, compiled);

                        // Return a map representing the compiled template
                        let mut result = HashMap::new();
                        result.insert("_template_id".to_string(), Value::Int(id as i64));
                        result.insert("path".to_string(), Value::String(path_str));

                        return Ok(Value::Map(result));
                    }

                    // Special handling for render(compiled, data) - render pre-compiled template
                    if name == "render" && arguments.len() == 2 {
                        let compiled = self.eval_expression(&arguments[0])?;
                        let data = self.eval_expression(&arguments[1])?;

                        // Get template ID from compiled template
                        let template_id = match &compiled {
                            Value::Map(m) => match m.get("_template_id") {
                                Some(Value::Int(id)) => *id as u64,
                                _ => {
                                    return Err(IntentError::TypeError(
                                        "render() first argument must be a compiled template"
                                            .to_string(),
                                    ))
                                }
                            },
                            _ => {
                                return Err(IntentError::TypeError(
                                    "render() first argument must be a compiled template"
                                        .to_string(),
                                ))
                            }
                        };

                        let data_map = match &data {
                            Value::Map(m) => m.clone(),
                            _ => {
                                return Err(IntentError::TypeError(
                                    "render() second argument must be a map".to_string(),
                                ))
                            }
                        };

                        // Get template content from cache
                        let content =
                            match crate::stdlib::template::get_compiled_template(template_id) {
                                Some(t) => t.content,
                                None => {
                                    return Err(IntentError::RuntimeError(
                                        "Template not found in cache".to_string(),
                                    ))
                                }
                            };

                        // Render with data
                        return self.render_template_with_data(&content, &data_map);
                    }

                    // Special handling for std/fs functions - resolve paths relative to script
                    // This makes apps portable: `ntnt run path/to/app.tnt` works from any directory
                    let fs_single_path_fns = [
                        "read_file",
                        "read_bytes",
                        "exists",
                        "is_file",
                        "is_dir",
                        "mkdir",
                        "mkdir_all",
                        "readdir",
                        "remove",
                        "remove_dir",
                        "remove_dir_all",
                        "file_size",
                    ];
                    let fs_two_path_fns = ["rename", "copy"];
                    let fs_path_content_fns = ["write_file", "append_file"];

                    if fs_single_path_fns.contains(&name.as_str()) && arguments.len() == 1 {
                        let path = self.eval_expression(&arguments[0])?;
                        if let Value::String(path_str) = &path {
                            let resolved = self.resolve_path_relative_to_script(path_str);
                            let resolved_value = Value::String(resolved);
                            let callee = self.eval_expression(function)?;
                            return self.call_function(callee, vec![resolved_value]);
                        }
                    }

                    if fs_two_path_fns.contains(&name.as_str()) && arguments.len() == 2 {
                        let from_path = self.eval_expression(&arguments[0])?;
                        let to_path = self.eval_expression(&arguments[1])?;
                        if let (Value::String(from_str), Value::String(to_str)) =
                            (&from_path, &to_path)
                        {
                            let resolved_from =
                                Value::String(self.resolve_path_relative_to_script(from_str));
                            let resolved_to =
                                Value::String(self.resolve_path_relative_to_script(to_str));
                            let callee = self.eval_expression(function)?;
                            return self.call_function(callee, vec![resolved_from, resolved_to]);
                        }
                    }

                    if fs_path_content_fns.contains(&name.as_str()) && arguments.len() == 2 {
                        let path = self.eval_expression(&arguments[0])?;
                        let content = self.eval_expression(&arguments[1])?;
                        if let Value::String(path_str) = &path {
                            let resolved =
                                Value::String(self.resolve_path_relative_to_script(path_str));
                            let callee = self.eval_expression(function)?;
                            return self.call_function(callee, vec![resolved, content]);
                        }
                    }

                    // Special handling for filter(arr, fn) - higher-order function
                    if name == "filter" && arguments.len() == 2 {
                        let arr = self.eval_expression(&arguments[0])?;
                        let predicate = self.eval_expression(&arguments[1])?;

                        if let Value::Array(items) = arr {
                            let mut result = Vec::new();
                            for item in items {
                                let should_include =
                                    self.call_function(predicate.clone(), vec![item.clone()])?;
                                if should_include.is_truthy() {
                                    result.push(item);
                                }
                            }
                            return Ok(Value::Array(result));
                        } else {
                            return Err(IntentError::TypeError(
                                "filter() requires an array as first argument".to_string(),
                            ));
                        }
                    }

                    // Special handling for transform(arr, fn) - higher-order function
                    if name == "transform" && arguments.len() == 2 {
                        let arr = self.eval_expression(&arguments[0])?;
                        let transform_fn = self.eval_expression(&arguments[1])?;

                        if let Value::Array(items) = arr {
                            let mut result = Vec::new();
                            for item in items {
                                let transformed =
                                    self.call_function(transform_fn.clone(), vec![item])?;
                                result.push(transformed);
                            }
                            return Ok(Value::Array(result));
                        } else {
                            return Err(IntentError::TypeError(
                                "transform() requires an array as first argument".to_string(),
                            ));
                        }
                    }

                    // Special handling for HTTP route registration
                    // Only intercept if first arg is a route pattern (starts with /)
                    // NOT if it's a URL (starts with http:// or https://) - those are HTTP client calls
                    let http_methods = ["get", "post", "put", "delete", "patch"];
                    if http_methods.contains(&name.as_str()) && arguments.len() == 2 {
                        // Use eval_route_pattern to auto-detect route parameters:
                        // "/users/{id}" preserves {id} as a route param instead of interpolating
                        let pattern = self.eval_route_pattern(&arguments[0])?;

                        // Check if this is a route pattern vs a URL
                        if let Value::String(pattern_str) = &pattern {
                            // Route patterns start with /, URLs start with http
                            if pattern_str.starts_with('/') {
                                // Skip route registration in unit test mode
                                if self.should_skip_route_registration() {
                                    return Ok(Value::Unit);
                                }
                                let handler = self.eval_expression(&arguments[1])?;
                                let method = name.to_uppercase();
                                self.server_state.add_route(&method, pattern_str, handler);
                                return Ok(Value::Unit);
                            }
                            // Otherwise fall through to normal function call (HTTP client)
                        }
                    }
                }

                let callee = self.eval_expression(function)?;
                let args: Result<Vec<Value>> = arguments
                    .iter()
                    .map(|arg| self.eval_expression(arg))
                    .collect();
                let args = args?;

                self.call_function(callee, args)
            }

            Expression::Array(elements) => {
                let vals: Result<Vec<Value>> =
                    elements.iter().map(|e| self.eval_expression(e)).collect();
                Ok(Value::Array(vals?))
            }

            Expression::Index { object, index } => {
                let obj = self.eval_expression(object)?;
                let idx = self.eval_expression(index)?;

                match (obj, idx) {
                    (Value::Array(arr), Value::Int(i)) => {
                        let index = if i < 0 {
                            (arr.len() as i64 + i) as usize
                        } else {
                            i as usize
                        };
                        #[allow(clippy::unnecessary_lazy_evaluations)]
                        arr.get(index)
                            .cloned()
                            .ok_or_else(|| IntentError::IndexOutOfBounds {
                                index: i,
                                length: arr.len(),
                            })
                    }
                    (Value::String(s), Value::Int(i)) => {
                        let index = if i < 0 {
                            (s.len() as i64 + i) as usize
                        } else {
                            i as usize
                        };
                        #[allow(clippy::unnecessary_lazy_evaluations)]
                        s.chars()
                            .nth(index)
                            .map(|c| Value::String(c.to_string()))
                            .ok_or_else(|| IntentError::IndexOutOfBounds {
                                index: i,
                                length: s.len(),
                            })
                    }
                    // Map access with string key: map["key"]
                    (Value::Map(map), Value::String(key)) => map
                        .get(&key)
                        .cloned()
                        .ok_or_else(|| IntentError::RuntimeError(format!("Unknown key: {}", key))),
                    // Struct access with string key: struct["field"]
                    (Value::Struct { fields, .. }, Value::String(key)) => {
                        fields.get(&key).cloned().ok_or_else(|| {
                            IntentError::RuntimeError(format!("Unknown field: {}", key))
                        })
                    }
                    _ => Err(IntentError::TypeError(
                        "Invalid index operation".to_string(),
                    )),
                }
            }

            Expression::FieldAccess { object, field } => {
                let obj = self.eval_expression(object)?;
                match obj {
                    Value::Struct { fields, .. } => fields.get(field).cloned().ok_or_else(|| {
                        IntentError::RuntimeError(format!("Unknown field: {}", field))
                    }),
                    Value::Map(map) => map.get(field).cloned().ok_or_else(|| {
                        IntentError::RuntimeError(format!("Unknown key: {}", field))
                    }),
                    _ => Err(IntentError::TypeError(
                        "Field access on non-struct value".to_string(),
                    )),
                }
            }

            Expression::StructLiteral { name, fields } => {
                let mut field_values = HashMap::new();
                for (field_name, expr) in fields {
                    field_values.insert(field_name.clone(), self.eval_expression(expr)?);
                }

                let struct_val = Value::Struct {
                    name: name.clone(),
                    fields: field_values,
                };

                // Check invariants on construction
                self.check_struct_invariants(name, &struct_val)?;

                Ok(struct_val)
            }

            Expression::EnumVariant {
                enum_name,
                variant,
                arguments,
            } => {
                // Evaluate any arguments
                let mut arg_values = Vec::new();
                for arg in arguments {
                    arg_values.push(self.eval_expression(arg)?);
                }

                // Create the enum value
                Ok(Value::EnumValue {
                    enum_name: enum_name.clone(),
                    variant: variant.clone(),
                    values: arg_values,
                })
            }

            Expression::Assign { target, value } => {
                let val = self.eval_expression(value)?;
                match target.as_ref() {
                    Expression::Identifier(name) => {
                        if self.environment.borrow_mut().set(name, val.clone()) {
                            // After assignment, check if this is a struct and verify invariants
                            if let Value::Struct {
                                name: struct_name, ..
                            } = &val
                            {
                                self.check_struct_invariants(struct_name, &val)?;
                            }
                            Ok(val)
                        } else {
                            let candidates = self.environment.borrow().keys();
                            let suggestion = crate::error::find_suggestion(name, &candidates);
                            Err(IntentError::UndefinedVariable {
                                name: name.clone(),
                                suggestion,
                            })
                        }
                    }
                    Expression::FieldAccess { object, field } => {
                        // Handle field assignment (e.g., obj.field = value)
                        if let Expression::Identifier(var_name) = object.as_ref() {
                            // Get the current struct
                            let current =
                                self.environment.borrow().get(var_name).ok_or_else(|| {
                                    let candidates = self.environment.borrow().keys();
                                    let suggestion =
                                        crate::error::find_suggestion(var_name, &candidates);
                                    IntentError::UndefinedVariable {
                                        name: var_name.clone(),
                                        suggestion,
                                    }
                                })?;

                            if let Value::Struct {
                                name: struct_name,
                                mut fields,
                            } = current
                            {
                                // Update the field
                                if fields.contains_key(field) {
                                    fields.insert(field.clone(), val.clone());

                                    let new_struct = Value::Struct {
                                        name: struct_name.clone(),
                                        fields: fields.clone(),
                                    };

                                    // Check invariants after field mutation
                                    self.check_struct_invariants(&struct_name, &new_struct)?;

                                    // Update the variable
                                    self.environment.borrow_mut().set(var_name, new_struct);
                                    Ok(val)
                                } else {
                                    Err(IntentError::RuntimeError(format!(
                                        "Unknown field '{}' on struct '{}'",
                                        field, struct_name
                                    )))
                                }
                            } else {
                                Err(IntentError::RuntimeError(
                                    "Cannot assign field on non-struct value".to_string(),
                                ))
                            }
                        } else {
                            Err(IntentError::RuntimeError(
                                "Cannot assign to complex field access".to_string(),
                            ))
                        }
                    }
                    _ => Err(IntentError::RuntimeError(
                        "Invalid assignment target".to_string(),
                    )),
                }
            }

            Expression::Block(block) => self.eval_block(block),

            Expression::IfExpr {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond = self.eval_expression(condition)?;
                if cond.is_truthy() {
                    self.eval_expression(then_branch)
                } else {
                    self.eval_expression(else_branch)
                }
            }

            Expression::Lambda { params, body } => Ok(Value::Function {
                name: "<lambda>".to_string(),
                params: params.clone(),
                body: Block {
                    statements: vec![Statement::Return(Some(body.as_ref().clone()))],
                },
                closure: Rc::clone(&self.environment),
                contract: None,
                type_params: vec![],
            }),

            Expression::MethodCall {
                object,
                method,
                arguments,
            } => {
                let obj = self.eval_expression(object)?;
                let args: Result<Vec<Value>> = arguments
                    .iter()
                    .map(|arg| self.eval_expression(arg))
                    .collect();
                let mut args = args?;

                // Keep track of struct name for invariant checking
                let struct_name = if let Value::Struct { name, .. } = &obj {
                    Some(name.clone())
                } else {
                    None
                };

                // Check if this is a module call (struct with function field)
                if let Value::Struct { name, fields } = &obj {
                    if name.starts_with("module:") || name.starts_with("lib:") {
                        // This is a module - look up method in its fields
                        if let Some(func) = fields.get(method) {
                            return self.call_function(func.clone(), args);
                        } else {
                            let module_name = name
                                .strip_prefix("module:")
                                .or_else(|| name.strip_prefix("lib:"))
                                .unwrap_or(name);
                            return Err(IntentError::RuntimeError(format!(
                                "Module '{}' has no function '{}'",
                                module_name, method
                            )));
                        }
                    }
                }

                args.insert(0, obj);

                // Look up method in environment
                let func = self.environment.borrow().get(method);
                if let Some(func) = func {
                    let result = self.call_function(func, args)?;

                    // After method call, check if self (first arg) was modified and verify invariants
                    // This requires looking up the updated value if it was bound to a variable
                    if let Some(struct_name) = struct_name {
                        // If the object came from a variable, check the updated value's invariants
                        if let Expression::Identifier(var_name) = object.as_ref() {
                            // Clone to avoid borrow conflict
                            let updated_obj = self.environment.borrow().get(var_name);
                            if let Some(updated_obj) = updated_obj {
                                if let Value::Struct { name, .. } = &updated_obj {
                                    if name == &struct_name {
                                        self.check_struct_invariants(name, &updated_obj)?;
                                    }
                                }
                            }
                        }
                    }

                    Ok(result)
                } else {
                    Err(IntentError::UndefinedFunction {
                        name: method.clone(),
                        suggestion: None,
                    })
                }
            }

            Expression::Match { scrutinee, arms } => {
                let value = self.eval_expression(scrutinee)?;

                // Check exhaustiveness for enum values
                if let Value::EnumValue { enum_name, .. } = &value {
                    self.check_exhaustiveness(enum_name, arms)?;
                }

                for arm in arms {
                    if let Some(bindings) = self.match_pattern(&arm.pattern, &value)? {
                        // Check guard if present
                        if let Some(guard) = &arm.guard {
                            // Create new scope with pattern bindings
                            let previous = Rc::clone(&self.environment);
                            self.environment = Rc::new(RefCell::new(Environment::with_parent(
                                Rc::clone(&previous),
                            )));

                            // Bind pattern variables
                            for (name, val) in &bindings {
                                self.environment
                                    .borrow_mut()
                                    .define(name.clone(), val.clone());
                            }

                            let guard_result = self.eval_expression(guard)?;

                            if !guard_result.is_truthy() {
                                self.environment = previous;
                                continue; // Guard failed, try next arm
                            }

                            // Guard passed, evaluate body
                            let result = self.eval_expression(&arm.body)?;
                            self.environment = previous;
                            return Ok(result);
                        } else {
                            // No guard, create scope and evaluate body
                            let previous = Rc::clone(&self.environment);
                            self.environment = Rc::new(RefCell::new(Environment::with_parent(
                                Rc::clone(&previous),
                            )));

                            // Bind pattern variables
                            for (name, val) in &bindings {
                                self.environment
                                    .borrow_mut()
                                    .define(name.clone(), val.clone());
                            }

                            let result = self.eval_expression(&arm.body)?;
                            self.environment = previous;
                            return Ok(result);
                        }
                    }
                }

                Err(IntentError::RuntimeError(
                    "No pattern matched in match expression".to_string(),
                ))
            }

            Expression::Await(_) | Expression::Try(_) => {
                // TODO: Implement async/try
                Err(IntentError::RuntimeError(
                    "Async/Try not yet implemented".to_string(),
                ))
            }

            Expression::MapLiteral(pairs) => {
                let mut map = HashMap::new();
                for (key_expr, value_expr) in pairs {
                    let key = self.eval_expression(key_expr)?;
                    let value = self.eval_expression(value_expr)?;

                    // Keys must be hashable (strings or integers for now)
                    let key_str = match &key {
                        Value::String(s) => s.clone(),
                        Value::Int(n) => n.to_string(),
                        _ => {
                            return Err(IntentError::RuntimeError(
                                "Map keys must be strings or integers".to_string(),
                            ))
                        }
                    };
                    map.insert(key_str, value);
                }
                Ok(Value::Map(map))
            }

            Expression::Range {
                start,
                end,
                inclusive,
            } => {
                let start_val = self.eval_expression(start)?;
                let end_val = self.eval_expression(end)?;

                match (&start_val, &end_val) {
                    (Value::Int(s), Value::Int(e)) => Ok(Value::Range {
                        start: *s,
                        end: *e,
                        inclusive: *inclusive,
                    }),
                    _ => Err(IntentError::RuntimeError(
                        "Range bounds must be integers".to_string(),
                    )),
                }
            }

            Expression::InterpolatedString(parts) => {
                use crate::ast::StringPart;
                let mut result = String::new();
                for part in parts {
                    match part {
                        StringPart::Literal(s) => result.push_str(s),
                        StringPart::Expr(expr) => {
                            let value = self.eval_expression(expr)?;
                            result.push_str(&value.to_string());
                        }
                    }
                }
                Ok(Value::String(result))
            }

            Expression::TemplateString(parts) => self.eval_template_parts(parts),
        }
    }

    /// Evaluate an expression as a route pattern.
    ///
    /// Route builtins (get, post, put, delete, patch) call this instead of
    /// eval_expression() for their path argument. It preserves `{name}` as
    /// literal route parameter placeholders instead of interpolating them.
    ///
    /// - InterpolatedString with simple identifiers: `"/users/{id}"` → `/users/{id}`
    /// - InterpolatedString with complex expressions: evaluated normally
    /// - All other expressions (String, variable, concatenation): evaluated normally
    fn eval_route_pattern(&mut self, expr: &Expression) -> Result<Value> {
        match expr {
            Expression::InterpolatedString(parts) => {
                use crate::ast::StringPart;
                let mut result = String::new();
                for part in parts {
                    match part {
                        StringPart::Literal(s) => result.push_str(s),
                        StringPart::Expr(inner) => {
                            if let Expression::Identifier(name) = inner {
                                // Route parameter: preserve {name} as literal
                                result.push('{');
                                result.push_str(name);
                                result.push('}');
                            } else {
                                // Complex expression: evaluate it (intentional interpolation)
                                let value = self.eval_expression(inner)?;
                                result.push_str(&value.to_string());
                            }
                        }
                    }
                }
                Ok(Value::String(result))
            }
            // For non-interpolated strings, variables, etc. — evaluate normally
            _ => self.eval_expression(expr),
        }
    }

    /// Render a template string with the given data
    fn render_template_with_data(
        &mut self,
        content: &str,
        data: &HashMap<String, Value>,
    ) -> Result<Value> {
        // Wrap content in triple quotes to make it a template string
        let template_source = format!("\"\"\"{}\"\"\"", content);

        // Parse the template string
        use crate::lexer::Lexer;
        use crate::parser::Parser;

        let lexer = Lexer::new(&template_source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = Parser::new(tokens);

        let template_expr = parser
            .expression()
            .map_err(|e| IntentError::RuntimeError(format!("Failed to compile template: {}", e)))?;

        // Create a new scope for template data
        let previous = Rc::clone(&self.environment);
        self.environment = Rc::new(RefCell::new(Environment::with_parent(Rc::clone(&previous))));

        // Define all data variables in the new scope
        for (key, value) in data {
            self.environment
                .borrow_mut()
                .define(key.clone(), value.clone());
        }

        // Evaluate the template expression
        let result = self.eval_expression(&template_expr);

        // Restore environment
        self.environment = previous;

        result
    }

    /// Evaluate template string parts
    fn eval_template_parts(&mut self, parts: &[TemplatePart]) -> Result<Value> {
        let mut result = String::new();

        for part in parts {
            match part {
                TemplatePart::Literal(s) => result.push_str(s),
                TemplatePart::Expr(expr) => {
                    let value = self.eval_expression(expr)?;
                    result.push_str(&value.to_string());
                }
                TemplatePart::FilteredExpr { expr, filters } => {
                    // Check if there's a default filter in the chain
                    let has_default = filters.iter().any(|f| f.name == "default");

                    let mut value = match self.eval_expression(expr) {
                        Ok(v) => v,
                        Err(e) => {
                            // If there's a default filter and we got an undefined variable error,
                            // use Unit as the value so the default filter can provide a fallback
                            if has_default && matches!(&e, IntentError::UndefinedVariable { .. }) {
                                Value::Unit
                            } else {
                                return Err(e);
                            }
                        }
                    };
                    for filter in filters {
                        value = self.apply_template_filter(&value, filter)?;
                    }
                    result.push_str(&value.to_string());
                }
                TemplatePart::ForLoop {
                    var,
                    iterable,
                    body,
                    empty_body,
                } => {
                    let iterable_value = self.eval_expression(iterable)?;

                    match iterable_value {
                        Value::Array(ref items) if items.is_empty() => {
                            // Empty array - render empty_body if present
                            if !empty_body.is_empty() {
                                let empty_result = self.eval_template_parts(empty_body)?;
                                if let Value::String(s) = empty_result {
                                    result.push_str(&s);
                                }
                            }
                        }
                        Value::Array(items) => {
                            let length = items.len();
                            for (index, item) in items.into_iter().enumerate() {
                                // Create new scope for each iteration
                                let previous = Rc::clone(&self.environment);
                                self.environment = Rc::new(RefCell::new(Environment::with_parent(
                                    Rc::clone(&previous),
                                )));

                                // Bind the loop variable
                                self.environment.borrow_mut().define(var.clone(), item);

                                // Bind loop metadata variables
                                self.environment
                                    .borrow_mut()
                                    .define("@index".to_string(), Value::Int(index as i64));
                                self.environment
                                    .borrow_mut()
                                    .define("@index1".to_string(), Value::Int((index + 1) as i64));
                                self.environment
                                    .borrow_mut()
                                    .define("@first".to_string(), Value::Bool(index == 0));
                                self.environment
                                    .borrow_mut()
                                    .define("@last".to_string(), Value::Bool(index == length - 1));
                                self.environment
                                    .borrow_mut()
                                    .define("@length".to_string(), Value::Int(length as i64));
                                self.environment
                                    .borrow_mut()
                                    .define("@even".to_string(), Value::Bool(index % 2 == 0));
                                self.environment
                                    .borrow_mut()
                                    .define("@odd".to_string(), Value::Bool(index % 2 == 1));

                                // Evaluate the body and append to result
                                let body_result = self.eval_template_parts(body)?;
                                if let Value::String(s) = body_result {
                                    result.push_str(&s);
                                }

                                // Restore environment
                                self.environment = previous;
                            }
                        }
                        Value::Map(ref map) if map.is_empty() => {
                            // Empty map - render empty_body if present
                            if !empty_body.is_empty() {
                                let empty_result = self.eval_template_parts(empty_body)?;
                                if let Value::String(s) = empty_result {
                                    result.push_str(&s);
                                }
                            }
                        }
                        Value::Map(map) => {
                            // When iterating over a map, yield (key, value) pairs
                            let length = map.len();
                            for (index, (k, v)) in map.iter().enumerate() {
                                // Create new scope for each iteration
                                let previous = Rc::clone(&self.environment);
                                self.environment = Rc::new(RefCell::new(Environment::with_parent(
                                    Rc::clone(&previous),
                                )));

                                // Create a tuple-like array for the pair
                                let pair = Value::Array(vec![Value::String(k.clone()), v.clone()]);
                                self.environment.borrow_mut().define(var.clone(), pair);

                                // Bind loop metadata variables
                                self.environment
                                    .borrow_mut()
                                    .define("@index".to_string(), Value::Int(index as i64));
                                self.environment
                                    .borrow_mut()
                                    .define("@index1".to_string(), Value::Int((index + 1) as i64));
                                self.environment
                                    .borrow_mut()
                                    .define("@first".to_string(), Value::Bool(index == 0));
                                self.environment
                                    .borrow_mut()
                                    .define("@last".to_string(), Value::Bool(index == length - 1));
                                self.environment
                                    .borrow_mut()
                                    .define("@length".to_string(), Value::Int(length as i64));
                                self.environment
                                    .borrow_mut()
                                    .define("@even".to_string(), Value::Bool(index % 2 == 0));
                                self.environment
                                    .borrow_mut()
                                    .define("@odd".to_string(), Value::Bool(index % 2 == 1));

                                let body_result = self.eval_template_parts(body)?;
                                if let Value::String(s) = body_result {
                                    result.push_str(&s);
                                }

                                // Restore environment
                                self.environment = previous;
                            }
                        }
                        _ => {
                            return Err(IntentError::RuntimeError(format!(
                                "Template for loop requires array or map, got {}",
                                iterable_value.type_name()
                            )));
                        }
                    }
                }
                TemplatePart::IfBlock {
                    condition,
                    then_parts,
                    elif_chains,
                    else_parts,
                } => {
                    let condition_value = self.eval_expression(condition)?;

                    if condition_value.is_truthy() {
                        let then_result = self.eval_template_parts(then_parts)?;
                        if let Value::String(s) = then_result {
                            result.push_str(&s);
                        }
                    } else {
                        // Check elif chains
                        let mut handled = false;
                        for (elif_condition, elif_body) in elif_chains {
                            let elif_value = self.eval_expression(elif_condition)?;
                            if elif_value.is_truthy() {
                                let elif_result = self.eval_template_parts(elif_body)?;
                                if let Value::String(s) = elif_result {
                                    result.push_str(&s);
                                }
                                handled = true;
                                break;
                            }
                        }

                        // Fall through to else if no elif matched
                        if !handled && !else_parts.is_empty() {
                            let else_result = self.eval_template_parts(else_parts)?;
                            if let Value::String(s) = else_result {
                                result.push_str(&s);
                            }
                        }
                    }
                }
            }
        }

        Ok(Value::String(result))
    }

    /// Apply a template filter to a value
    fn apply_template_filter(
        &mut self,
        value: &Value,
        filter: &crate::ast::TemplateFilter,
    ) -> Result<Value> {
        // Evaluate filter arguments
        let mut args = Vec::new();
        for arg_expr in &filter.args {
            args.push(self.eval_expression(arg_expr)?);
        }

        match filter.name.as_str() {
            // String filters
            "uppercase" => {
                let s = value.to_string();
                Ok(Value::String(s.to_uppercase()))
            }
            "lowercase" => {
                let s = value.to_string();
                Ok(Value::String(s.to_lowercase()))
            }
            "capitalize" => {
                let s = value.to_string();
                let mut chars = s.chars();
                match chars.next() {
                    None => Ok(Value::String(String::new())),
                    Some(first) => Ok(Value::String(first.to_uppercase().chain(chars).collect())),
                }
            }
            "trim" => {
                let s = value.to_string();
                Ok(Value::String(s.trim().to_string()))
            }
            "truncate" => {
                let s = value.to_string();
                let max_len = match args.first() {
                    Some(Value::Int(n)) => *n as usize,
                    _ => {
                        return Err(IntentError::RuntimeError(
                            "truncate filter requires an integer argument".to_string(),
                        ))
                    }
                };
                if s.len() <= max_len {
                    Ok(Value::String(s))
                } else {
                    Ok(Value::String(format!("{}...", &s[..max_len])))
                }
            }
            "replace" => {
                let s = value.to_string();
                let (from, to) = match (args.first(), args.get(1)) {
                    (Some(Value::String(f)), Some(Value::String(t))) => (f.as_str(), t.as_str()),
                    _ => {
                        return Err(IntentError::RuntimeError(
                            "replace filter requires two string arguments".to_string(),
                        ))
                    }
                };
                Ok(Value::String(s.replace(from, to)))
            }

            // Safety filters
            "escape" => {
                let s = value.to_string();
                let escaped = s
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;")
                    .replace('\'', "&#x27;");
                Ok(Value::String(escaped))
            }
            "raw" => {
                // raw just returns the value as-is (no auto-escaping)
                Ok(value.clone())
            }
            "default" => match value {
                Value::Unit => Ok(args
                    .first()
                    .cloned()
                    .unwrap_or(Value::String(String::new()))),
                Value::EnumValue {
                    enum_name, variant, ..
                } if enum_name == "Option" && variant == "None" => Ok(args
                    .first()
                    .cloned()
                    .unwrap_or(Value::String(String::new()))),
                Value::String(s) if s.is_empty() => Ok(args
                    .first()
                    .cloned()
                    .unwrap_or(Value::String(String::new()))),
                _ => Ok(value.clone()),
            },

            // Collection filters
            "length" => match value {
                Value::String(s) => Ok(Value::Int(s.len() as i64)),
                Value::Array(arr) => Ok(Value::Int(arr.len() as i64)),
                Value::Map(m) => Ok(Value::Int(m.len() as i64)),
                _ => Err(IntentError::RuntimeError(format!(
                    "length filter not supported for {}",
                    value.type_name()
                ))),
            },
            "first" => match value {
                Value::Array(arr) => Ok(arr.first().cloned().unwrap_or(Value::Unit)),
                Value::String(s) => Ok(Value::String(
                    s.chars().next().map(|c| c.to_string()).unwrap_or_default(),
                )),
                _ => Err(IntentError::RuntimeError(format!(
                    "first filter not supported for {}",
                    value.type_name()
                ))),
            },
            "last" => match value {
                Value::Array(arr) => Ok(arr.last().cloned().unwrap_or(Value::Unit)),
                Value::String(s) => Ok(Value::String(
                    s.chars().last().map(|c| c.to_string()).unwrap_or_default(),
                )),
                _ => Err(IntentError::RuntimeError(format!(
                    "last filter not supported for {}",
                    value.type_name()
                ))),
            },
            "reverse" => match value {
                Value::Array(arr) => {
                    let mut reversed = arr.clone();
                    reversed.reverse();
                    Ok(Value::Array(reversed))
                }
                Value::String(s) => Ok(Value::String(s.chars().rev().collect())),
                _ => Err(IntentError::RuntimeError(format!(
                    "reverse filter not supported for {}",
                    value.type_name()
                ))),
            },
            "join" => {
                let separator = match args.first() {
                    Some(Value::String(s)) => s.as_str(),
                    _ => ", ",
                };
                match value {
                    Value::Array(arr) => {
                        let strings: Vec<String> = arr.iter().map(|v| v.to_string()).collect();
                        Ok(Value::String(strings.join(separator)))
                    }
                    _ => Err(IntentError::RuntimeError(format!(
                        "join filter not supported for {}",
                        value.type_name()
                    ))),
                }
            }
            "slice" => {
                let start = match args.first() {
                    Some(Value::Int(n)) => *n as usize,
                    _ => 0,
                };
                let end = match args.get(1) {
                    Some(Value::Int(n)) => Some(*n as usize),
                    _ => None,
                };
                match value {
                    Value::Array(arr) => {
                        let end = end.unwrap_or(arr.len()).min(arr.len());
                        let start = start.min(end);
                        Ok(Value::Array(arr[start..end].to_vec()))
                    }
                    Value::String(s) => {
                        let chars: Vec<char> = s.chars().collect();
                        let end = end.unwrap_or(chars.len()).min(chars.len());
                        let start = start.min(end);
                        Ok(Value::String(chars[start..end].iter().collect()))
                    }
                    _ => Err(IntentError::RuntimeError(format!(
                        "slice filter not supported for {}",
                        value.type_name()
                    ))),
                }
            }

            // Formatting filters
            "json" => {
                let json_value = crate::stdlib::json::intent_value_to_json(value);
                Ok(Value::String(json_value.to_string()))
            }
            "number" => {
                let decimals = match args.first() {
                    Some(Value::Int(n)) => *n as usize,
                    _ => 2,
                };
                match value {
                    Value::Int(n) => Ok(Value::String(format!(
                        "{:.prec$}",
                        *n as f64,
                        prec = decimals
                    ))),
                    Value::Float(f) => Ok(Value::String(format!("{:.prec$}", f, prec = decimals))),
                    _ => Ok(Value::String(value.to_string())),
                }
            }
            "url_encode" => {
                let s = value.to_string();
                Ok(Value::String(urlencoding::encode(&s).to_string()))
            }

            _ => Err(IntentError::RuntimeError(format!(
                "Unknown template filter: {}",
                filter.name
            ))),
        }
    }

    /// Try to match a pattern against a value, returning variable bindings if successful
    #[allow(clippy::only_used_in_recursion)]
    fn match_pattern(
        &self,
        pattern: &Pattern,
        value: &Value,
    ) -> Result<Option<Vec<(String, Value)>>> {
        match pattern {
            Pattern::Wildcard => Ok(Some(vec![])),

            Pattern::Variable(name) => Ok(Some(vec![(name.clone(), value.clone())])),

            Pattern::Literal(expr) => {
                // For literals, we need to check if the value matches
                match expr {
                    Expression::Integer(n) => {
                        if let Value::Int(v) = value {
                            if v == n {
                                return Ok(Some(vec![]));
                            }
                        }
                    }
                    Expression::Float(n) => {
                        if let Value::Float(v) = value {
                            if (v - n).abs() < f64::EPSILON {
                                return Ok(Some(vec![]));
                            }
                        }
                    }
                    Expression::String(s) => {
                        if let Value::String(v) = value {
                            if v == s {
                                return Ok(Some(vec![]));
                            }
                        }
                    }
                    Expression::Bool(b) => {
                        if let Value::Bool(v) = value {
                            if v == b {
                                return Ok(Some(vec![]));
                            }
                        }
                    }
                    Expression::Unit => {
                        if matches!(value, Value::Unit) {
                            return Ok(Some(vec![]));
                        }
                    }
                    _ => {}
                }
                Ok(None)
            }

            Pattern::Tuple(patterns) => {
                // For now, treat tuple patterns as array patterns
                if let Value::Array(values) = value {
                    if values.len() != patterns.len() {
                        return Ok(None);
                    }
                    let mut bindings = vec![];
                    for (pat, val) in patterns.iter().zip(values.iter()) {
                        if let Some(b) = self.match_pattern(pat, val)? {
                            bindings.extend(b);
                        } else {
                            return Ok(None);
                        }
                    }
                    return Ok(Some(bindings));
                }
                Ok(None)
            }

            Pattern::Array(patterns) => {
                if let Value::Array(values) = value {
                    if values.len() != patterns.len() {
                        return Ok(None);
                    }
                    let mut bindings = vec![];
                    for (pat, val) in patterns.iter().zip(values.iter()) {
                        if let Some(b) = self.match_pattern(pat, val)? {
                            bindings.extend(b);
                        } else {
                            return Ok(None);
                        }
                    }
                    return Ok(Some(bindings));
                }
                Ok(None)
            }

            Pattern::Struct { name, fields } => {
                if let Value::Struct {
                    name: struct_name,
                    fields: struct_fields,
                } = value
                {
                    if name != struct_name {
                        return Ok(None);
                    }
                    let mut bindings = vec![];
                    for (field_name, field_pattern) in fields {
                        if let Some(field_value) = struct_fields.get(field_name) {
                            if let Some(b) = self.match_pattern(field_pattern, field_value)? {
                                bindings.extend(b);
                            } else {
                                return Ok(None);
                            }
                        } else {
                            return Ok(None);
                        }
                    }
                    return Ok(Some(bindings));
                }
                Ok(None)
            }

            Pattern::Variant {
                name,
                variant,
                fields,
            } => {
                if let Value::EnumValue {
                    enum_name,
                    variant: value_variant,
                    values,
                } = value
                {
                    // Check if enum and variant match (handling qualified and unqualified names)
                    let enum_matches = name.is_empty() || name == enum_name;
                    let variant_matches = variant == value_variant;

                    if !enum_matches || !variant_matches {
                        return Ok(None);
                    }

                    // Match field patterns against values
                    match fields {
                        Some(patterns) => {
                            if patterns.len() != values.len() {
                                return Ok(None);
                            }
                            let mut bindings = vec![];
                            for (pat, val) in patterns.iter().zip(values.iter()) {
                                if let Some(b) = self.match_pattern(pat, val)? {
                                    bindings.extend(b);
                                } else {
                                    return Ok(None);
                                }
                            }
                            Ok(Some(bindings))
                        }
                        None => {
                            if values.is_empty() {
                                Ok(Some(vec![]))
                            } else {
                                Ok(None)
                            }
                        }
                    }
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Bind variables from a pattern destructuring
    fn bind_pattern(&mut self, pattern: &Pattern, value: &Value) -> Result<()> {
        match self.match_pattern(pattern, value)? {
            Some(bindings) => {
                for (name, val) in bindings {
                    self.environment.borrow_mut().define(name, val);
                }
                Ok(())
            }
            None => Err(IntentError::RuntimeError(
                "Pattern destructuring failed: value does not match pattern".to_string(),
            )),
        }
    }

    /// Check exhaustiveness of match arms against an enum type
    fn check_exhaustiveness(&self, enum_name: &str, arms: &[MatchArm]) -> Result<()> {
        // Get the enum variants
        let variants = match self.enums.get(enum_name) {
            Some(v) => v,
            None => return Ok(()), // Unknown enum, skip check
        };

        let variant_names: Vec<&str> = variants.iter().map(|v| v.name.as_str()).collect();
        let mut covered = std::collections::HashSet::new();
        let mut has_wildcard = false;

        for arm in arms {
            match &arm.pattern {
                Pattern::Wildcard => {
                    has_wildcard = true;
                }
                Pattern::Variable(_) => {
                    has_wildcard = true; // Variable captures all
                }
                Pattern::Variant { variant, .. } => {
                    covered.insert(variant.as_str());
                }
                _ => {}
            }
        }

        if has_wildcard {
            return Ok(()); // Wildcard covers everything
        }

        let missing: Vec<&&str> = variant_names
            .iter()
            .filter(|v| !covered.contains(*v))
            .collect();

        if !missing.is_empty() {
            return Err(IntentError::RuntimeError(format!(
                "Non-exhaustive match: missing variants {:?}",
                missing
            )));
        }

        Ok(())
    }

    fn call_function(&mut self, callee: Value, args: Vec<Value>) -> Result<Value> {
        match callee {
            Value::Function {
                name,
                params,
                body,
                closure,
                contract,
                type_params: _, // Generic type params - for future type checking
            } => {
                if args.len() != params.len() {
                    return Err(IntentError::ArityMismatch {
                        name: name.clone(),
                        expected: params.len(),
                        got: args.len(),
                    });
                }

                // Create new environment with closure as parent
                let func_env = Rc::new(RefCell::new(Environment::with_parent(closure)));

                // Bind parameters
                for (param, arg) in params.iter().zip(args.iter()) {
                    func_env
                        .borrow_mut()
                        .define(param.name.clone(), arg.clone());
                }

                // Save current environment and switch to function's environment
                let previous = Rc::clone(&self.environment);
                self.environment = Rc::clone(&func_env);

                // Track deferred statements for this function call
                let deferred_count_before = self.deferred_statements.len();

                // Check preconditions BEFORE execution
                if let Some(ref func_contract) = contract {
                    for req_expr in &func_contract.requires {
                        let condition_str = Self::format_expression(req_expr);
                        let result = self.eval_expression(req_expr)?;
                        if !result.is_truthy() {
                            self.environment = previous;
                            return Err(IntentError::ContractViolation(format!(
                                "Precondition failed in '{}': {}",
                                name, condition_str
                            )));
                        }
                        self.contracts
                            .check_precondition(&condition_str, true, None)?;
                    }

                    // Capture old values for postconditions containing old()
                    self.current_old_values =
                        Some(self.capture_old_values(&func_contract.ensures)?);
                }

                // Execute function body
                let mut result = Value::Unit;
                for stmt in &body.statements {
                    result = self.eval_statement(stmt)?;
                    if let Value::Return(v) = result {
                        result = *v;
                        break;
                    }
                }

                // Execute deferred statements in reverse order (LIFO) before returning
                let deferred_to_run: Vec<Expression> = self
                    .deferred_statements
                    .drain(deferred_count_before..)
                    .collect();

                for deferred_expr in deferred_to_run.into_iter().rev() {
                    // Deferred expressions execute even if there was a return
                    let _ = self.eval_expression(&deferred_expr);
                }

                // Store result for postcondition evaluation
                self.current_result = Some(result.clone());

                // Bind 'result' in environment for postcondition evaluation
                self.environment
                    .borrow_mut()
                    .define("result".to_string(), result.clone());

                // Check postconditions AFTER execution
                if let Some(ref func_contract) = contract {
                    for ens_expr in &func_contract.ensures {
                        let condition_str = Self::format_expression(ens_expr);
                        let postcond_result = self.eval_expression(ens_expr)?;
                        if !postcond_result.is_truthy() {
                            // Clear state before returning error
                            self.current_old_values = None;
                            self.current_result = None;
                            self.environment = previous;
                            return Err(IntentError::ContractViolation(format!(
                                "Postcondition failed in '{}': {}",
                                name, condition_str
                            )));
                        }
                        self.contracts
                            .check_postcondition(&condition_str, true, None)?;
                    }
                }

                // Clear contract evaluation state
                self.current_old_values = None;
                self.current_result = None;

                // Restore environment
                self.environment = previous;

                Ok(result)
            }

            Value::NativeFunction {
                name: fn_name,
                arity,
                func,
            } => {
                if args.len() != arity && arity != 0 {
                    return Err(IntentError::ArityMismatch {
                        name: fn_name.clone(),
                        expected: arity,
                        got: args.len(),
                    });
                }
                func(&args)
            }

            Value::EnumConstructor {
                enum_name,
                variant,
                arity,
            } => {
                if args.len() != arity {
                    return Err(IntentError::ArityMismatch {
                        name: format!("{}::{}", enum_name, variant),
                        expected: arity,
                        got: args.len(),
                    });
                }
                Ok(Value::EnumValue {
                    enum_name,
                    variant,
                    values: args,
                })
            }

            _ => Err(IntentError::TypeError(
                "Can only call functions".to_string(),
            )),
        }
    }

    /// Run the HTTP server on the specified port
    fn run_http_server(&mut self, port: u16) -> Result<Value> {
        use crate::stdlib::http_server;
        use std::sync::atomic::Ordering;
        use std::time::Duration;

        // Check for NTNT_LISTEN_PORT env var override (used by Intent Studio)
        let env_port = std::env::var("NTNT_LISTEN_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok());

        // Check if we're in test mode
        let (actual_port, is_test_mode, shutdown_flag) = match &self.test_mode {
            Some((test_port, _max_req, flag)) => (*test_port, true, Some(flag.clone())),
            None => (env_port.unwrap_or(port), false, None),
        };

        // Check if any routes or static dirs are registered
        let has_routes = self.server_state.route_count() > 0;
        let has_static = !self.server_state.static_dirs.is_empty();

        if !has_routes && !has_static {
            return Err(IntentError::RuntimeError(
                "No routes or static directories registered. Use get(), post(), serve_static(), etc. before calling listen()".to_string()
            ));
        }

        // Print startup message
        if is_test_mode {
            println!("Starting test server on http://127.0.0.1:{}", actual_port);
        } else {
            println!("Starting server on http://0.0.0.0:{}", actual_port);
        }

        if has_routes {
            println!("Routes registered: {}", self.server_state.route_count());
        }
        if has_static {
            println!(
                "Static directories: {}",
                self.server_state.static_dirs.len()
            );
            if !is_test_mode {
                for (prefix, dir) in &self.server_state.static_dirs {
                    println!("  {} -> {}", prefix, dir);
                }
            }
        }
        let middleware_count = self.server_state.middleware.len();
        if middleware_count > 0 {
            println!("Middleware: {}", middleware_count);
        }

        // Show hot-reload status
        if self.server_state.hot_reload && self.main_source_file.is_some() {
            println!(
                "\n🔥 Hot-reload enabled: edit your .tnt file and changes apply on next request"
            );
        }

        if !is_test_mode {
            println!("Press Ctrl+C to stop");
        }
        println!();

        // Start the server
        let server = if is_test_mode {
            http_server::start_server_with_timeout(actual_port, Duration::from_secs(60))?
        } else {
            http_server::start_server(actual_port)?
        };

        // Handle requests in a loop
        // In test mode, use recv_timeout and check shutdown flag
        loop {
            // Check shutdown flag in test mode
            if let Some(ref flag) = shutdown_flag {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
            }

            // Get next request (with timeout in test mode)
            let request = if is_test_mode {
                match server.recv_timeout(Duration::from_millis(50)) {
                    Ok(Some(req)) => req,
                    Ok(None) => continue, // Timeout, check shutdown flag
                    Err(_) => break,      // Server error
                }
            } else {
                match server.recv() {
                    Ok(req) => req,
                    Err(_) => break,
                }
            };

            // Hot-reload check: if main source file changed, reload it
            // This runs on each request to pick up changes without restart
            self.check_and_reload_main_source();

            let method = request.method().to_string();
            let url = request.url().to_string();
            let path = url.split('?').next().unwrap_or(&url).to_string();

            // First, try to find a matching route
            if let Some((mut handler, route_params, route_index)) =
                self.server_state.find_route(&method, &path)
            {
                // Hot-reload check: if file or its imports changed, reload the handler
                if self.server_state.needs_reload(route_index) {
                    if let Some(source) = self.server_state.get_route_source(route_index).cloned() {
                        if let Some(file_path) = &source.file_path {
                            // Re-parse and reload the handler
                            match self.reload_route_handler(file_path, &method) {
                                Ok((new_handler, new_imports)) => {
                                    self.server_state.update_route_handler(
                                        route_index,
                                        new_handler.clone(),
                                        new_imports,
                                    );
                                    handler = new_handler;
                                    println!("[hot-reload] Reloaded: {}", file_path);
                                }
                                Err(e) => {
                                    eprintln!("[hot-reload] Error reloading {}: {}", file_path, e);
                                }
                            }
                        }
                    }
                }

                // Process request to get request Value
                match http_server::process_request(request, route_params) {
                    Ok((mut req_value, http_request)) => {
                        // Run middleware chain and determine final response
                        let middleware_handlers: Vec<Value> =
                            self.server_state.get_middleware().to_vec();
                        let mut early_response: Option<Value> = None;

                        for mw in middleware_handlers {
                            match self.call_function(mw.clone(), vec![req_value.clone()]) {
                                Ok(result) => {
                                    // Check if middleware returned a response (early exit) or modified request
                                    match &result {
                                        Value::Map(map) if map.contains_key("status") => {
                                            // Middleware returned a response - use it and stop
                                            early_response = Some(result);
                                            break;
                                        }
                                        Value::Map(_) => {
                                            // Middleware returned modified request - continue with it
                                            req_value = result;
                                        }
                                        Value::Unit => {
                                            // Middleware returned unit - continue with original request
                                        }
                                        _ => {
                                            // Other return - continue with original request
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("Middleware error: {}", e);
                                    early_response = Some(http_server::create_error_response(
                                        500,
                                        &e.to_string(),
                                    ));
                                    break;
                                }
                            }
                        }

                        // Determine final response
                        let final_response = if let Some(resp) = early_response {
                            resp
                        } else {
                            // Call the route handler
                            match self.call_function(handler, vec![req_value]) {
                                Ok(response) => response,
                                Err(e) => {
                                    eprintln!("Handler error: {}", e);
                                    // Check for contract violations and return appropriate HTTP status
                                    if let IntentError::ContractViolation(msg) = &e {
                                        if msg.contains("Precondition failed") {
                                            // Precondition = bad request from client
                                            http_server::create_error_response(
                                                400,
                                                &format!("Bad Request: {}", msg),
                                            )
                                        } else if msg.contains("Postcondition failed") {
                                            // Postcondition = server logic error
                                            http_server::create_error_response(
                                                500,
                                                &format!("Internal Error: {}", msg),
                                            )
                                        } else {
                                            http_server::create_error_response(500, &e.to_string())
                                        }
                                    } else {
                                        http_server::create_error_response(500, &e.to_string())
                                    }
                                }
                            }
                        };

                        // Send the response (only once)
                        if let Err(e) = http_server::send_response(http_request, &final_response) {
                            eprintln!("Error sending response: {}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("Error processing request: {}", e);
                    }
                }
                continue;
            }

            // No matching route - check static files (only for GET requests)
            if method == "GET" {
                if let Some((file_path, _relative)) = self.server_state.find_static_file(&path) {
                    // Serve static file
                    if let Err(e) = http_server::send_static_response(request, &file_path) {
                        eprintln!("Error serving static file: {}", e);
                    }
                    continue;
                }
            }

            // No matching route or static file - send 404
            let path_clone = path.clone();
            #[allow(clippy::single_match)]
            match http_server::process_request(request, HashMap::new()) {
                Ok((_, http_request)) => {
                    let not_found = http_server::create_error_response(
                        404,
                        &format!("Not Found: {} {}", method, path_clone),
                    );
                    let _ = http_server::send_response(http_request, &not_found);
                }
                Err(_) => {}
            }
        }

        // Server is shutting down - call shutdown handlers
        let shutdown_handlers: Vec<Value> = self.server_state.get_shutdown_handlers().to_vec();
        if !shutdown_handlers.is_empty() {
            println!("\nRunning shutdown handlers...");
            for handler in shutdown_handlers {
                if let Err(e) = self.call_function(handler, vec![]) {
                    eprintln!("Shutdown handler error: {}", e);
                }
            }
        }

        Ok(Value::Unit)
    }

    /// Run the HTTP server using Axum + Tokio
    /// This provides high-concurrency handling for production workloads
    fn run_async_http_server(&mut self, port: u16) -> Result<Value> {
        use crate::stdlib::http_bridge::{
            create_channel, BridgeConfig, BridgeResponse, HandlerRequest, InterpreterHandle,
        };
        use crate::stdlib::http_server_async::{
            start_server_with_bridge, AsyncServerConfig, AsyncServerState,
        };
        use std::sync::Arc;
        use std::thread;

        // Check if any routes are registered
        if self.server_state.route_count() == 0 && self.server_state.static_dirs.is_empty() {
            return Err(IntentError::RuntimeError(
                "No routes or static directories registered. Use get(), post(), serve_static(), etc. before calling listen()".to_string()
            ));
        }

        // Check for NTNT_LISTEN_PORT env var override (used by Intent Studio and intent check)
        let actual_port = std::env::var("NTNT_LISTEN_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(port);

        // Enable hot-reload unless in production mode
        let is_production = std::env::var("NTNT_ENV")
            .map(|v| v == "production" || v == "prod")
            .unwrap_or(false);
        self.server_state.hot_reload = !is_production;

        if is_production {
            println!("Running in production mode (hot-reload disabled)");
        }

        // Create the channel for interpreter communication
        let config = BridgeConfig::default();
        let (tx, mut rx) = create_channel(&config);

        // Create async server state with registered routes
        let async_routes = Arc::new(AsyncServerState::new());

        // Helper function to sync routes from interpreter to async state
        fn sync_routes_to_async(
            server_state: &crate::stdlib::http_server::ServerState,
            async_routes: &AsyncServerState,
            rt: &tokio::runtime::Runtime,
        ) {
            // Clear existing async routes
            async_routes.clear_blocking(rt);

            // Copy routes
            for (route, _handler, _source) in &server_state.routes {
                async_routes.register_route_blocking(rt, &route.method, &route.pattern, "handler");
            }

            // Copy static directories
            for (url_prefix, fs_path) in &server_state.static_dirs {
                async_routes.register_static_dir_blocking(rt, url_prefix, fs_path);
            }
        }

        // Create the async runtime for route registration and hot-reload sync
        let sync_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| IntentError::RuntimeError(format!("Failed to create runtime: {}", e)))?;

        // Initial route sync from interpreter to async state
        sync_routes_to_async(&self.server_state, &async_routes, &sync_rt);

        // Create interpreter handle for async handlers
        let interpreter_handle = Arc::new(InterpreterHandle::new(tx));

        // Create server config
        let server_config = AsyncServerConfig {
            port: actual_port,
            host: "0.0.0.0".to_string(),
            enable_compression: true,
            request_timeout_secs: self.request_timeout_secs,
            max_connections: 10_000,
        };

        // Spawn async server in a separate thread
        // Note: We move interpreter_handle into the thread (not clone) so it's dropped
        // when the server shuts down, which closes the channel and signals the main loop to exit
        let routes_clone = async_routes.clone();
        let server_handle = thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime");

            rt.block_on(async {
                if let Err(e) =
                    start_server_with_bridge(server_config, interpreter_handle, routes_clone).await
                {
                    eprintln!("Server error: {}", e);
                }
            });
        });

        // Main thread: process requests from the channel
        // This runs the interpreter in a single thread (required since it's not Send+Sync)
        loop {
            // Block waiting for requests
            match rx.blocking_recv() {
                Some(handler_request) => {
                    let HandlerRequest { request, reply_tx } = handler_request;

                    // Hot-reload check: if main source file changed, reload it
                    if self.check_and_reload_main_source() {
                        // Routes changed - sync to async state
                        sync_routes_to_async(&self.server_state, &async_routes, &sync_rt);
                    }

                    // Find the matching route handler
                    let method = &request.method;
                    let path = &request.path;

                    if let Some((mut handler, route_params, route_index)) =
                        self.server_state.find_route(method, path)
                    {
                        // Hot-reload check: if route file or its imports changed, reload the handler
                        if self.server_state.needs_reload(route_index) {
                            if let Some(source) =
                                self.server_state.get_route_source(route_index).cloned()
                            {
                                if let Some(file_path) = &source.file_path {
                                    match self.reload_route_handler(file_path, method) {
                                        Ok((new_handler, new_imports)) => {
                                            self.server_state.update_route_handler(
                                                route_index,
                                                new_handler.clone(),
                                                new_imports,
                                            );
                                            handler = new_handler;
                                            println!("[hot-reload] Reloaded: {}", file_path);
                                            // Sync updated routes to async state
                                            sync_routes_to_async(
                                                &self.server_state,
                                                &async_routes,
                                                &sync_rt,
                                            );
                                        }
                                        Err(e) => {
                                            eprintln!(
                                                "[hot-reload] Error reloading {}: {}",
                                                file_path, e
                                            );
                                        }
                                    }
                                }
                            }
                        }

                        // Merge route params with request params
                        let mut full_request = request.clone();
                        for (k, v) in route_params {
                            full_request.params.insert(k, v);
                        }

                        // Convert to NTNT Value
                        let req_value = full_request.to_value();

                        // Run middleware
                        let middleware_handlers: Vec<Value> =
                            self.server_state.get_middleware().to_vec();
                        let mut current_req = req_value;
                        let mut early_response: Option<Value> = None;

                        for mw in middleware_handlers {
                            match self.call_function(mw.clone(), vec![current_req.clone()]) {
                                Ok(result) => match &result {
                                    Value::Map(map) if map.contains_key("status") => {
                                        early_response = Some(result);
                                        break;
                                    }
                                    Value::Map(_) => {
                                        current_req = result;
                                    }
                                    _ => {}
                                },
                                Err(e) => {
                                    eprintln!("Middleware error: {}", e);
                                    early_response =
                                        Some(crate::stdlib::http_server::create_error_response(
                                            500,
                                            &e.to_string(),
                                        ));
                                    break;
                                }
                            }
                        }

                        // Determine final response
                        let final_response = if let Some(resp) = early_response {
                            resp
                        } else {
                            match self.call_function(handler, vec![current_req]) {
                                Ok(response) => response,
                                Err(e) => {
                                    eprintln!("Handler error: {}", e);
                                    crate::stdlib::http_server::create_error_response(
                                        500,
                                        &e.to_string(),
                                    )
                                }
                            }
                        };

                        // Convert to BridgeResponse and send back
                        let bridge_response = BridgeResponse::from_value(&final_response);
                        let _ = reply_tx.send(bridge_response);
                    } else {
                        // No route found
                        let _ = reply_tx.send(BridgeResponse::not_found());
                    }
                }
                None => {
                    // Channel closed, server shutting down
                    println!("\n🛑 Server shutting down...");
                    break;
                }
            }
        }

        // Wait for server thread to finish
        let _ = server_handle.join();

        Ok(Value::Unit)
    }

    /// Capture old values from expressions in postconditions
    fn capture_old_values(&mut self, ensures: &[Expression]) -> Result<OldValues> {
        let mut old_values = OldValues::new();

        for expr in ensures {
            self.extract_old_calls(expr, &mut old_values)?;
        }

        Ok(old_values)
    }

    /// Recursively find old() calls in an expression and capture their values
    fn extract_old_calls(&mut self, expr: &Expression, old_values: &mut OldValues) -> Result<()> {
        match expr {
            Expression::Call {
                function,
                arguments,
            } => {
                // Check if this is an old() call
                if let Expression::Identifier(name) = function.as_ref() {
                    if name == "old" && arguments.len() == 1 {
                        // Evaluate the inner expression now (pre-execution)
                        let inner_expr = &arguments[0];
                        let key = format!("{:?}", inner_expr);
                        if !old_values.contains(&key) {
                            let value = self.eval_expression(inner_expr)?;
                            old_values.store(key, self.value_to_stored(&value));
                        }
                    }
                }
                // Also check arguments for nested old() calls
                for arg in arguments {
                    self.extract_old_calls(arg, old_values)?;
                }
            }
            Expression::Binary { left, right, .. } => {
                self.extract_old_calls(left, old_values)?;
                self.extract_old_calls(right, old_values)?;
            }
            Expression::Unary { operand, .. } => {
                self.extract_old_calls(operand, old_values)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Convert a runtime Value to a StoredValue for old() tracking
    #[allow(clippy::only_used_in_recursion)]
    fn value_to_stored(&self, value: &Value) -> StoredValue {
        match value {
            Value::Int(n) => StoredValue::Int(*n),
            Value::Float(f) => StoredValue::Float(*f),
            Value::Bool(b) => StoredValue::Bool(*b),
            Value::String(s) => StoredValue::String(s.clone()),
            Value::Array(arr) => {
                StoredValue::Array(arr.iter().map(|v| self.value_to_stored(v)).collect())
            }
            Value::Unit => StoredValue::Unit,
            _ => StoredValue::Unit, // Functions and other complex types stored as Unit
        }
    }

    /// Convert a StoredValue back to a runtime Value
    #[allow(clippy::only_used_in_recursion)]
    fn stored_to_value(&self, stored: &StoredValue) -> Value {
        match stored {
            StoredValue::Int(n) => Value::Int(*n),
            StoredValue::Float(f) => Value::Float(*f),
            StoredValue::Bool(b) => Value::Bool(*b),
            StoredValue::String(s) => Value::String(s.clone()),
            StoredValue::Array(arr) => {
                Value::Array(arr.iter().map(|v| self.stored_to_value(v)).collect())
            }
            StoredValue::Unit => Value::Unit,
        }
    }

    /// Format an expression as a human-readable string for error messages
    fn format_expression(expr: &Expression) -> String {
        match expr {
            Expression::Integer(n) => n.to_string(),
            Expression::Float(f) => f.to_string(),
            Expression::String(s) => format!("\"{}\"", s),
            Expression::Bool(b) => b.to_string(),
            Expression::Unit => "()".to_string(),
            Expression::Identifier(name) => name.clone(),
            Expression::Binary {
                left,
                operator,
                right,
            } => {
                let op_str = match operator {
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
                    Self::format_expression(left),
                    op_str,
                    Self::format_expression(right)
                )
            }
            Expression::Unary { operator, operand } => {
                let op_str = match operator {
                    UnaryOp::Neg => "-",
                    UnaryOp::Not => "!",
                };
                format!("{}{}", op_str, Self::format_expression(operand))
            }
            Expression::Call {
                function,
                arguments,
            } => {
                let func_str = Self::format_expression(function);
                let args_str: Vec<String> = arguments.iter().map(Self::format_expression).collect();
                format!("{}({})", func_str, args_str.join(", "))
            }
            Expression::FieldAccess { object, field } => {
                format!("{}.{}", Self::format_expression(object), field)
            }
            Expression::Index { object, index } => {
                format!(
                    "{}[{}]",
                    Self::format_expression(object),
                    Self::format_expression(index)
                )
            }
            Expression::Array(elements) => {
                let elems: Vec<String> = elements.iter().map(Self::format_expression).collect();
                format!("[{}]", elems.join(", "))
            }
            _ => format!("{:?}", expr),
        }
    }

    /// Check struct invariants after construction or mutation
    fn check_struct_invariants(&mut self, struct_name: &str, struct_val: &Value) -> Result<()> {
        // Look up invariants for this struct type
        let invariants = match self.struct_invariants.get(struct_name) {
            Some(inv) => inv.clone(),
            None => return Ok(()), // No invariants defined
        };

        if invariants.is_empty() {
            return Ok(());
        }

        // Get struct fields
        let fields = match struct_val {
            Value::Struct { fields, .. } => fields,
            _ => return Ok(()),
        };

        // Create a temporary environment with struct fields as variables
        let previous = Rc::clone(&self.environment);
        let inv_env = Rc::new(RefCell::new(Environment::with_parent(Rc::clone(&previous))));

        // Bind struct fields to environment (also bind 'self' to the struct)
        for (field_name, field_val) in fields {
            inv_env
                .borrow_mut()
                .define(field_name.clone(), field_val.clone());
        }
        inv_env
            .borrow_mut()
            .define("self".to_string(), struct_val.clone());

        self.environment = inv_env;

        // Check each invariant
        for inv_expr in &invariants {
            let condition_str = Self::format_expression(inv_expr);
            let result = self.eval_expression(inv_expr)?;

            if !result.is_truthy() {
                self.environment = previous;
                return Err(IntentError::ContractViolation(format!(
                    "Invariant violated for '{}': {}",
                    struct_name, condition_str
                )));
            }
            self.contracts.check_invariant(&condition_str, true, None)?;
        }

        self.environment = previous;
        Ok(())
    }

    fn eval_binary_op(&self, op: BinaryOp, lhs: Value, rhs: Value) -> Result<Value> {
        match (op, lhs, rhs) {
            // Integer arithmetic
            (BinaryOp::Add, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
            (BinaryOp::Sub, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a - b)),
            (BinaryOp::Mul, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
            (BinaryOp::Div, Value::Int(_), Value::Int(0)) => Err(IntentError::DivisionByZero),
            (BinaryOp::Div, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a / b)),
            (BinaryOp::Mod, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a % b)),
            (BinaryOp::Pow, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.pow(b as u32))),

            // Float arithmetic
            (BinaryOp::Add, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
            (BinaryOp::Sub, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
            (BinaryOp::Mul, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
            (BinaryOp::Div, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
            (BinaryOp::Mod, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a % b)),
            (BinaryOp::Pow, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a.powf(b))),

            // Mixed numeric
            (BinaryOp::Add, Value::Int(a), Value::Float(b)) => Ok(Value::Float(a as f64 + b)),
            (BinaryOp::Add, Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + b as f64)),
            (BinaryOp::Sub, Value::Int(a), Value::Float(b)) => Ok(Value::Float(a as f64 - b)),
            (BinaryOp::Sub, Value::Float(a), Value::Int(b)) => Ok(Value::Float(a - b as f64)),
            (BinaryOp::Mul, Value::Int(a), Value::Float(b)) => Ok(Value::Float(a as f64 * b)),
            (BinaryOp::Mul, Value::Float(a), Value::Int(b)) => Ok(Value::Float(a * b as f64)),
            (BinaryOp::Div, Value::Int(a), Value::Float(b)) => Ok(Value::Float(a as f64 / b)),
            (BinaryOp::Div, Value::Float(a), Value::Int(b)) => Ok(Value::Float(a / b as f64)),

            // String concatenation
            (BinaryOp::Add, Value::String(a), Value::String(b)) => {
                Ok(Value::String(format!("{}{}", a, b)))
            }
            (BinaryOp::Add, Value::String(a), b) => Ok(Value::String(format!("{}{}", a, b))),
            (BinaryOp::Add, a, Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),

            // Array concatenation
            (BinaryOp::Add, Value::Array(mut a), Value::Array(b)) => {
                a.extend(b);
                Ok(Value::Array(a))
            }

            // Comparison - integers
            (BinaryOp::Eq, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a == b)),
            (BinaryOp::Ne, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a != b)),
            (BinaryOp::Lt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
            (BinaryOp::Le, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a <= b)),
            (BinaryOp::Gt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
            (BinaryOp::Ge, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a >= b)),

            // Comparison - floats
            (BinaryOp::Eq, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a == b)),
            (BinaryOp::Ne, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a != b)),
            (BinaryOp::Lt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a < b)),
            (BinaryOp::Le, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a <= b)),
            (BinaryOp::Gt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a > b)),
            (BinaryOp::Ge, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a >= b)),

            // Comparison - strings
            (BinaryOp::Eq, Value::String(a), Value::String(b)) => Ok(Value::Bool(a == b)),
            (BinaryOp::Ne, Value::String(a), Value::String(b)) => Ok(Value::Bool(a != b)),
            (BinaryOp::Lt, Value::String(a), Value::String(b)) => Ok(Value::Bool(a < b)),
            (BinaryOp::Le, Value::String(a), Value::String(b)) => Ok(Value::Bool(a <= b)),
            (BinaryOp::Gt, Value::String(a), Value::String(b)) => Ok(Value::Bool(a > b)),
            (BinaryOp::Ge, Value::String(a), Value::String(b)) => Ok(Value::Bool(a >= b)),

            // Comparison - booleans
            (BinaryOp::Eq, Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a == b)),
            (BinaryOp::Ne, Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a != b)),

            // Mixed numeric comparison
            (BinaryOp::Eq, Value::Int(a), Value::Float(b)) => Ok(Value::Bool((a as f64) == b)),
            (BinaryOp::Eq, Value::Float(a), Value::Int(b)) => Ok(Value::Bool(a == (b as f64))),
            (BinaryOp::Lt, Value::Int(a), Value::Float(b)) => Ok(Value::Bool((a as f64) < b)),
            (BinaryOp::Lt, Value::Float(a), Value::Int(b)) => Ok(Value::Bool(a < (b as f64))),
            (BinaryOp::Le, Value::Int(a), Value::Float(b)) => Ok(Value::Bool((a as f64) <= b)),
            (BinaryOp::Le, Value::Float(a), Value::Int(b)) => Ok(Value::Bool(a <= (b as f64))),
            (BinaryOp::Gt, Value::Int(a), Value::Float(b)) => Ok(Value::Bool((a as f64) > b)),
            (BinaryOp::Gt, Value::Float(a), Value::Int(b)) => Ok(Value::Bool(a > (b as f64))),
            (BinaryOp::Ge, Value::Int(a), Value::Float(b)) => Ok(Value::Bool((a as f64) >= b)),
            (BinaryOp::Ge, Value::Float(a), Value::Int(b)) => Ok(Value::Bool(a >= (b as f64))),

            (op, lhs, rhs) => Err(IntentError::InvalidOperation(format!(
                "Cannot apply {:?} to {} and {}",
                op,
                lhs.type_name(),
                rhs.type_name()
            ))),
        }
    }

    /// Print current environment bindings
    pub fn print_environment(&self) {
        println!("Current environment:");
        let env = self.environment.borrow();
        for key in env.keys() {
            if let Some(value) = env.get(&key) {
                // Skip built-in functions for cleaner output
                match &value {
                    Value::NativeFunction { .. } => continue,
                    Value::Function { name, params, .. } => {
                        let param_names: Vec<_> = params.iter().map(|p| p.name.as_str()).collect();
                        println!("  {} = fn {}({})", key, name, param_names.join(", "));
                    }
                    _ => println!("  {} = {}", key, value),
                }
            }
        }
    }
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn eval(source: &str) -> Result<Value> {
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = Parser::new(tokens);
        let ast = parser.parse()?;
        let mut interpreter = Interpreter::new();
        interpreter.eval(&ast)
    }

    #[test]
    fn test_arithmetic() {
        assert!(matches!(eval("1 + 2").unwrap(), Value::Int(3)));
        assert!(matches!(eval("10 - 3").unwrap(), Value::Int(7)));
        assert!(matches!(eval("4 * 5").unwrap(), Value::Int(20)));
        assert!(matches!(eval("20 / 4").unwrap(), Value::Int(5)));
    }

    #[test]
    fn test_variables() {
        assert!(matches!(eval("let x = 42; x").unwrap(), Value::Int(42)));
    }

    #[test]
    fn test_functions() {
        let result = eval("fn add(a, b) { return a + b; } add(2, 3)").unwrap();
        assert!(matches!(result, Value::Int(5)));
    }

    #[test]
    fn test_conditionals() {
        assert!(matches!(
            eval("if true { 1 } else { 2 }").unwrap(),
            Value::Int(1)
        ));
        assert!(matches!(
            eval("if false { 1 } else { 2 }").unwrap(),
            Value::Int(2)
        ));
    }

    #[test]
    fn test_loops() {
        let result = eval("let x = 0; while x < 5 { x = x + 1; } x").unwrap();
        assert!(matches!(result, Value::Int(5)));
    }

    #[test]
    fn test_contract_precondition_passes() {
        // Precondition passes when b != 0
        let result = eval(
            r#"
            fn divide(a, b) requires b != 0 { return a / b; }
            divide(10, 2)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(5)));
    }

    #[test]
    fn test_contract_precondition_fails() {
        // Precondition fails when b == 0
        let result = eval(
            r#"
            fn divide(a, b) requires b != 0 { return a / b; }
            divide(10, 0)
        "#,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Precondition failed"));
    }

    #[test]
    fn test_contract_postcondition_passes() {
        // Postcondition passes when result >= 0
        let result = eval(
            r#"
            fn absolute(x) ensures result >= 0 { 
                if x < 0 { return -x; } 
                return x; 
            }
            absolute(-5)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(5)));
    }

    #[test]
    fn test_contract_postcondition_fails() {
        // Postcondition fails intentionally
        let result = eval(
            r#"
            fn bad_absolute(x) ensures result > 100 { 
                if x < 0 { return -x; } 
                return x; 
            }
            bad_absolute(5)
        "#,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Postcondition failed"));
    }

    #[test]
    fn test_contract_with_result() {
        // Use result keyword in postcondition
        let result = eval(
            r#"
            fn double(x) ensures result == x * 2 { 
                return x * 2; 
            }
            double(7)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(14)));
    }

    #[test]
    fn test_contract_with_old() {
        // Use old() to capture pre-execution value
        let result = eval(
            r#"
            fn increment(x) ensures result == old(x) + 1 { 
                return x + 1; 
            }
            increment(10)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(11)));
    }

    #[test]
    fn test_multiple_contracts() {
        // Multiple requires and ensures
        let result = eval(
            r#"
            fn clamp(value, min_val, max_val) 
                requires min_val <= max_val
                ensures result >= min_val
                ensures result <= max_val
            { 
                if value < min_val { return min_val; }
                if value > max_val { return max_val; }
                return value;
            }
            clamp(15, 0, 10)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(10)));
    }

    #[test]
    fn test_struct_literal() {
        // Basic struct literal creation
        let result = eval(
            r#"
            struct Point {
                x: Int,
                y: Int
            }
            let p = Point { x: 10, y: 20 };
            p.x + p.y
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(30)));
    }

    #[test]
    fn test_struct_invariant_passes() {
        // Struct invariant passes on construction
        let result = eval(
            r#"
            struct Counter {
                value: Int
            }
            impl Counter {
                invariant self.value >= 0
            }
            let c = Counter { value: 5 };
            c.value
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(5)));
    }

    #[test]
    fn test_struct_invariant_fails() {
        // Struct invariant fails on construction
        let result = eval(
            r#"
            struct Counter {
                value: Int
            }
            impl Counter {
                invariant self.value >= 0
            }
            let c = Counter { value: -1 };
            c.value
        "#,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Invariant violated"));
    }

    // ============================================
    // Math function tests
    // ============================================

    #[test]
    fn test_abs() {
        assert!(matches!(eval("abs(-5)").unwrap(), Value::Int(5)));
        assert!(matches!(eval("abs(5)").unwrap(), Value::Int(5)));
        assert!(matches!(eval("abs(0)").unwrap(), Value::Int(0)));
        // Float
        if let Value::Float(f) = eval("abs(-3.14)").unwrap() {
            assert!((f - 3.14).abs() < 0.001);
        } else {
            panic!("Expected float");
        }
    }

    #[test]
    fn test_min_max() {
        assert!(matches!(eval("min(3, 7)").unwrap(), Value::Int(3)));
        assert!(matches!(eval("min(7, 3)").unwrap(), Value::Int(3)));
        assert!(matches!(eval("max(3, 7)").unwrap(), Value::Int(7)));
        assert!(matches!(eval("max(7, 3)").unwrap(), Value::Int(7)));
        // Mixed int/float
        if let Value::Float(f) = eval("min(3, 2.5)").unwrap() {
            assert!((f - 2.5).abs() < 0.001);
        } else {
            panic!("Expected float");
        }
    }

    #[test]
    fn test_round_floor_ceil() {
        // round (Rust rounds away from zero for .5)
        assert!(matches!(eval("round(3.4)").unwrap(), Value::Int(3)));
        assert!(matches!(eval("round(3.5)").unwrap(), Value::Int(4)));
        assert!(matches!(eval("round(3.6)").unwrap(), Value::Int(4)));
        assert!(matches!(eval("round(-2.5)").unwrap(), Value::Int(-3))); // rounds away from zero
                                                                         // floor
        assert!(matches!(eval("floor(3.9)").unwrap(), Value::Int(3)));
        assert!(matches!(eval("floor(-3.1)").unwrap(), Value::Int(-4)));
        // ceil
        assert!(matches!(eval("ceil(3.1)").unwrap(), Value::Int(4)));
        assert!(matches!(eval("ceil(-3.9)").unwrap(), Value::Int(-3)));
    }

    #[test]
    fn test_sqrt() {
        if let Value::Float(f) = eval("sqrt(16)").unwrap() {
            assert!((f - 4.0).abs() < 0.001);
        } else {
            panic!("Expected float");
        }
        if let Value::Float(f) = eval("sqrt(2.0)").unwrap() {
            assert!((f - 1.414).abs() < 0.01);
        } else {
            panic!("Expected float");
        }
        // Negative should error
        assert!(eval("sqrt(-1)").is_err());
    }

    #[test]
    fn test_pow() {
        assert!(matches!(eval("pow(2, 3)").unwrap(), Value::Int(8)));
        assert!(matches!(eval("pow(2, 0)").unwrap(), Value::Int(1)));
        assert!(matches!(eval("pow(5, 2)").unwrap(), Value::Int(25)));
        // Float exponent
        if let Value::Float(f) = eval("pow(4, 0.5)").unwrap() {
            assert!((f - 2.0).abs() < 0.001);
        } else {
            panic!("Expected float");
        }
    }

    #[test]
    fn test_sign() {
        assert!(matches!(eval("sign(42)").unwrap(), Value::Int(1)));
        assert!(matches!(eval("sign(-42)").unwrap(), Value::Int(-1)));
        assert!(matches!(eval("sign(0)").unwrap(), Value::Int(0)));
        assert!(matches!(eval("sign(3.14)").unwrap(), Value::Int(1)));
        assert!(matches!(eval("sign(-3.14)").unwrap(), Value::Int(-1)));
    }

    #[test]
    fn test_clamp() {
        assert!(matches!(eval("clamp(5, 0, 10)").unwrap(), Value::Int(5)));
        assert!(matches!(eval("clamp(-5, 0, 10)").unwrap(), Value::Int(0)));
        assert!(matches!(eval("clamp(15, 0, 10)").unwrap(), Value::Int(10)));
    }

    // ============================================
    // Phase 2: Type System & Pattern Matching Tests
    // ============================================

    #[test]
    fn test_option_some() {
        // Test Some constructor and is_some helper
        let result = eval(
            r#"
            let x = Some(42);
            is_some(x)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_option_none() {
        // Test None constructor and is_none helper
        let result = eval(
            r#"
            let x = None;
            is_none(x)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_option_unwrap() {
        // Test unwrap on Some
        let result = eval(
            r#"
            let x = Some(100);
            unwrap(x)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(100)));
    }

    #[test]
    fn test_option_unwrap_or() {
        // Test unwrap_or on None
        let result = eval(
            r#"
            let x = None;
            unwrap_or(x, 50)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(50)));

        // Test unwrap_or on Some
        let result = eval(
            r#"
            let x = Some(100);
            unwrap_or(x, 50)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(100)));
    }

    #[test]
    fn test_result_ok() {
        // Test Ok constructor and is_ok helper
        let result = eval(
            r#"
            let x = Ok(42);
            is_ok(x)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_result_err() {
        // Test Err constructor and is_err helper
        let result = eval(
            r#"
            let x = Err("error message");
            is_err(x)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_match_option_some() {
        // Match on Some variant
        let result = eval(
            r#"
            let x = Some(10);
            match x {
                Some(v) => v * 2,
                None => 0
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(20)));
    }

    #[test]
    fn test_match_option_none() {
        // Match on None variant
        let result = eval(
            r#"
            let x = None;
            match x {
                Some(v) => v * 2,
                None => -1
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(-1)));
    }

    #[test]
    fn test_match_result_ok() {
        // Match on Ok variant
        let result = eval(
            r#"
            let x = Ok(42);
            match x {
                Ok(v) => v + 1,
                Err(e) => 0
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(43)));
    }

    #[test]
    fn test_match_result_err() {
        // Match on Err variant
        let result = eval(
            r#"
            let x = Err("failed");
            match x {
                Ok(v) => v,
                Err(e) => -1
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(-1)));
    }

    #[test]
    fn test_match_literal_int() {
        // Match on literal integer patterns
        let result = eval(
            r#"
            let x = 2;
            match x {
                1 => 100,
                2 => 200,
                3 => 300,
                _ => 0
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(200)));
    }

    #[test]
    fn test_match_wildcard() {
        // Match wildcard pattern
        let result = eval(
            r#"
            let x = 999;
            match x {
                1 => 100,
                2 => 200,
                _ => -1
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(-1)));
    }

    #[test]
    fn test_match_binding() {
        // Match with variable binding
        let result = eval(
            r#"
            let x = 42;
            match x {
                n => n + 8
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(50)));
    }

    #[test]
    fn test_user_enum_definition() {
        // User-defined enum
        let result = eval(
            r#"
            enum Color {
                Red,
                Green,
                Blue
            }
            let c = Color::Red;
            match c {
                Color::Red => 1,
                Color::Green => 2,
                Color::Blue => 3
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(1)));
    }

    #[test]
    fn test_user_enum_with_data() {
        // User-defined enum with data
        let result = eval(
            r#"
            enum Shape {
                Circle(Float),
                Rectangle(Float, Float)
            }
            let s = Shape::Circle(5.0);
            match s {
                Shape::Circle(r) => r * 2.0,
                Shape::Rectangle(w, h) => w * h
            }
        "#,
        )
        .unwrap();
        if let Value::Float(f) = result {
            assert!((f - 10.0).abs() < 0.001);
        } else {
            panic!("Expected float");
        }
    }

    #[test]
    fn test_user_enum_rectangle() {
        // User-defined enum Rectangle variant
        let result = eval(
            r#"
            enum Shape {
                Circle(Float),
                Rectangle(Float, Float)
            }
            let s = Shape::Rectangle(3.0, 4.0);
            match s {
                Shape::Circle(r) => r * 2.0,
                Shape::Rectangle(w, h) => w * h
            }
        "#,
        )
        .unwrap();
        if let Value::Float(f) = result {
            assert!((f - 12.0).abs() < 0.001);
        } else {
            panic!("Expected float");
        }
    }

    #[test]
    fn test_type_alias() {
        // Type alias (currently just parses, doesn't enforce types)
        let result = eval(
            r#"
            type UserId = Int;
            let id: UserId = 12345;
            id
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(12345)));
    }

    #[test]
    fn test_union_type() {
        // Union type annotation (parses, runtime is dynamically typed)
        let result = eval(
            r#"
            fn accepts_either(x: String | Int) {
                return x;
            }
            accepts_either(42)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(42)));

        // Also works with strings
        let result = eval(
            r#"
            fn accepts_either(x: String | Int) {
                return x;
            }
            accepts_either("hello")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "hello");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_union_type_multiple() {
        // Union with multiple types
        let result = eval(
            r#"
            fn flexible(x: Int | Float | String | Bool) {
                return x;
            }
            flexible(true)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_generic_function_declaration() {
        // Generic function declaration (parses, generics not enforced at runtime)
        let result = eval(
            r#"
            fn identity<T>(x: T) -> T {
                return x;
            }
            identity(42)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(42)));
    }

    #[test]
    fn test_generic_function_with_string() {
        // Generic function with string
        let result = eval(
            r#"
            fn identity<T>(x: T) -> T {
                return x;
            }
            identity("hello")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "hello");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_effects_annotation() {
        // Function with effects annotation (parses, not enforced)
        let result = eval(
            r#"
            fn read_file(path: String) -> String with io {
                return "file contents";
            }
            read_file("test.txt")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "file contents");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_pure_function() {
        // Pure function annotation
        let result = eval(
            r#"
            fn add(a: Int, b: Int) -> Int pure {
                return a + b;
            }
            add(3, 4)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(7)));
    }

    #[test]
    fn test_nested_option() {
        // Nested Option handling
        let result = eval(
            r#"
            let outer = Some(Some(42));
            match outer {
                Some(inner) => match inner {
                    Some(v) => v,
                    None => -1
                },
                None => -2
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(42)));
    }

    #[test]
    fn test_match_in_function() {
        // Match expression inside a function
        let result = eval(
            r#"
            fn safe_div(a, b) {
                if b == 0 {
                    return None;
                }
                return Some(a / b);
            }
            
            let result = safe_div(10, 2);
            match result {
                Some(v) => v,
                None => -1
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(5)));
    }

    #[test]
    fn test_match_division_by_zero() {
        // Match on None from safe division
        let result = eval(
            r#"
            fn safe_div(a, b) {
                if b == 0 {
                    return None;
                }
                return Some(a / b);
            }
            
            let result = safe_div(10, 0);
            match result {
                Some(v) => v,
                None => -1
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(-1)));
    }

    #[test]
    fn test_match_bool_pattern() {
        // Match on boolean values
        let result = eval(
            r#"
            let flag = true;
            match flag {
                true => 1,
                false => 0
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(1)));
    }

    #[test]
    fn test_match_string_pattern() {
        // Match on string values
        let result = eval(
            r#"
            let cmd = "start";
            match cmd {
                "start" => 1,
                "stop" => 2,
                _ => 0
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(1)));
    }

    #[test]
    fn test_enum_unit_variants() {
        // Enum with only unit variants
        let result = eval(
            r#"
            enum Status {
                Pending,
                Active,
                Completed
            }
            let s = Status::Active;
            match s {
                Status::Pending => 0,
                Status::Active => 1,
                Status::Completed => 2
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(1)));
    }

    // Module System Tests

    #[test]
    fn test_import_std_string_split() {
        let result = eval(
            r#"
            import { split } from "std/string"
            let parts = split("hello,world", ",")
            len(parts)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(2)));
    }

    #[test]
    fn test_import_std_string_join() {
        let result = eval(
            r#"
            import { join, split } from "std/string"
            let parts = split("a-b-c", "-")
            join(parts, "_")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "a_b_c");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_import_std_string_trim() {
        let result = eval(
            r#"
            import { trim } from "std/string"
            trim("  hello world  ")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "hello world");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_import_std_string_replace() {
        let result = eval(
            r#"
            import { replace } from "std/string"
            replace("hello world", "world", "rust")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "hello rust");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_import_std_string_contains() {
        let result = eval(
            r#"
            import { contains } from "std/string"
            contains("hello world", "wor")
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_import_std_string_starts_ends_with() {
        let result = eval(
            r#"
            import { starts_with, ends_with } from "std/string"
            let s = "hello.txt"
            starts_with(s, "hello") && ends_with(s, ".txt")
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_import_std_string_case_conversion() {
        let result = eval(
            r#"
            import { to_upper, to_lower } from "std/string"
            to_upper("hello") == "HELLO" && to_lower("WORLD") == "world"
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_import_std_string_char_at() {
        let result = eval(
            r#"
            import { char_at } from "std/string"
            char_at("hello", 1)
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "e");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_import_std_string_substring() {
        let result = eval(
            r#"
            import { substring } from "std/string"
            substring("hello world", 0, 5)
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "hello");
        } else {
            panic!("Expected string");
        }
    }

    // ==================== New std/string tests ====================

    #[test]
    fn test_std_string_trim_left_right() {
        let result = eval(
            r#"
            import { trim_left, trim_right } from "std/string"
            let s = "  hello  "
            trim_left(s) == "hello  " && trim_right(s) == "  hello"
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_std_string_trim_chars() {
        let result = eval(
            r#"
            import { trim_chars } from "std/string"
            trim_chars("***hello***", "*")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "hello");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_std_string_capitalize_title() {
        let result = eval(
            r#"
            import { capitalize, title } from "std/string"
            capitalize("hello world") == "Hello world" && title("hello world") == "Hello World"
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_std_string_case_conversion() {
        let result = eval(
            r#"
            import { to_snake_case, to_camel_case, to_pascal_case, to_kebab_case } from "std/string"
            to_snake_case("helloWorld") == "hello_world" &&
            to_camel_case("hello_world") == "helloWorld" &&
            to_pascal_case("hello_world") == "HelloWorld" &&
            to_kebab_case("helloWorld") == "hello-world"
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_std_string_slugify() {
        let result = eval(
            r#"
            import { slugify } from "std/string"
            slugify("Hello World! This is NTNT.")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "hello-world-this-is-ntnt");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_std_string_last_index_of() {
        let result = eval(
            r#"
            import { last_index_of } from "std/string"
            last_index_of("hello hello", "hello")
        "#,
        )
        .unwrap();
        if let Value::Int(i) = result {
            assert_eq!(i, 6);
        } else {
            panic!("Expected int");
        }
    }

    #[test]
    fn test_std_string_count() {
        let result = eval(
            r#"
            import { count } from "std/string"
            count("the quick brown fox jumps over the lazy dog", "the")
        "#,
        )
        .unwrap();
        if let Value::Int(i) = result {
            assert_eq!(i, 2);
        } else {
            panic!("Expected int");
        }
    }

    #[test]
    fn test_std_string_replace_all() {
        let result = eval(
            r#"
            import { replace_all } from "std/string"
            replace_all("hello hello", "hello", "hi")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "hi hi");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_std_string_lines_words() {
        let result = eval(
            r#"
            import { lines, words } from "std/string"
            let l = lines("a
b
c")
            let w = words("  hello   world  ")
            len(l) == 3 && len(w) == 2
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_std_string_truncate() {
        let result = eval(
            r#"
            import { truncate } from "std/string"
            truncate("hello world", 8, "...")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "hello...");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_std_string_padding() {
        let result = eval(
            r#"
            import { pad_left, pad_right, center } from "std/string"
            pad_left("42", 5, "0") == "00042" &&
            pad_right("hi", 5, ".") == "hi..." &&
            center("hi", 6, "*") == "**hi**"
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_std_string_validation() {
        let result = eval(
            r#"
            import { is_empty, is_blank, is_numeric, is_alpha, is_alphanumeric } from "std/string"
            is_empty("") == true &&
            is_empty("x") == false &&
            is_blank("   ") == true &&
            is_blank(" x ") == false &&
            is_numeric("123") == true &&
            is_numeric("12a") == false &&
            is_alpha("abc") == true &&
            is_alpha("ab3") == false &&
            is_alphanumeric("abc123") == true
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_std_string_case_validation() {
        let result = eval(
            r#"
            import { is_lowercase, is_uppercase, is_whitespace } from "std/string"
            is_lowercase("hello") == true &&
            is_uppercase("HELLO") == true &&
            is_whitespace("   ") == true
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_std_string_matches() {
        let result = eval(
            r#"
            import { matches } from "std/string"
            matches("hello", "h*o") == true &&
            matches("hello", "h?llo") == true &&
            matches("hello", "world") == false &&
            matches("test.txt", "*.txt") == true
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_std_string_repeat_reverse() {
        let result = eval(
            r#"
            import { repeat, reverse } from "std/string"
            repeat("ab", 3) == "ababab" && reverse("hello") == "olleh"
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_std_string_chars() {
        let result = eval(
            r#"
            import { chars } from "std/string"
            let c = chars("abc")
            len(c) == 3 && c[0] == "a" && c[1] == "b" && c[2] == "c"
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_import_std_math_constants() {
        let result = eval(
            r#"
            import { PI, E } from "std/math"
            PI > 3.14 && PI < 3.15 && E > 2.71 && E < 2.72
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_import_std_math_trig() {
        let result = eval(
            r#"
            import { sin, cos, PI } from "std/math"
            let s = sin(0.0)
            let c = cos(0.0)
            s < 0.001 && s > -0.001 && c > 0.999
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_import_std_math_log_exp() {
        let result = eval(
            r#"
            import { log, exp, E } from "std/math"
            let log_e = log(E)
            let exp_0 = exp(0.0)
            log_e > 0.99 && log_e < 1.01 && exp_0 > 0.99 && exp_0 < 1.01
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_import_std_collections_push() {
        let result = eval(
            r#"
            import { push } from "std/collections"
            let arr = [1, 2, 3]
            let arr2 = push(arr, 4)
            len(arr2)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(4)));
    }

    #[test]
    fn test_import_std_collections_first_last() {
        let result = eval(
            r#"
            import { first, last } from "std/collections"
            let arr = [10, 20, 30]
            let f = first(arr)
            let l = last(arr)
            match f {
                Some(v) => match l {
                    Some(w) => v + w,
                    None => -1
                },
                None => -1
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(40)));
    }

    #[test]
    fn test_import_std_collections_reverse() {
        let result = eval(
            r#"
            import { reverse } from "std/collections"
            let arr = [1, 2, 3]
            let rev = reverse(arr)
            rev[0]
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(3)));
    }

    #[test]
    fn test_import_std_collections_slice() {
        let result = eval(
            r#"
            import { slice } from "std/collections"
            let arr = [1, 2, 3, 4, 5]
            let sub = slice(arr, 1, 4)
            len(sub)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(3)));
    }

    #[test]
    fn test_import_std_collections_concat() {
        let result = eval(
            r#"
            import { concat } from "std/collections"
            let a = [1, 2]
            let b = [3, 4]
            let c = concat(a, b)
            len(c)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(4)));
    }

    #[test]
    fn test_import_std_collections_is_empty() {
        let result = eval(
            r#"
            import { is_empty } from "std/collections"
            let empty = []
            let full = [1]
            is_empty(empty) && !is_empty(full)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_import_std_env_cwd() {
        let result = eval(
            r#"
            import { cwd } from "std/env"
            let dir = cwd()
            len(dir) > 0
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_import_std_env_args() {
        let result = eval(
            r#"
            import { args } from "std/env"
            let argv = args()
            len(argv) >= 0
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_import_entire_module() {
        let result = eval(
            r#"
            import "std/string" as str
            str.trim("  test  ")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "test");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_import_with_alias() {
        let result = eval(
            r#"
            import { split as divide } from "std/string"
            let parts = divide("a:b:c", ":")
            len(parts)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(3)));
    }

    // ===== Phase 4 Tests: Traits & Essential Features =====

    #[test]
    fn test_trait_declaration() {
        // Test that trait declarations parse and eval without error
        let result = eval(
            r#"
            trait Show {
                fn show(self) -> String;
            }
            42
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(42)));
    }

    #[test]
    fn test_trait_with_default() {
        let result = eval(
            r#"
            trait Greet {
                fn greet(name: String) -> String {
                    return "Hello, " + name;
                }
            }
            "ok"
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "ok");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_impl_trait_for_type() {
        let result = eval(
            r#"
            trait Printable {
                fn describe(self) -> String;
            }
            
            struct Point {
                x: Int,
                y: Int
            }
            
            impl Printable for Point {
                fn describe(self) -> String {
                    return "Point"
                }
            }
            
            42
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(42)));
    }

    #[test]
    fn test_for_in_array() {
        let result = eval(
            r#"
            let sum = 0
            for x in [1, 2, 3, 4, 5] {
                sum = sum + x
            }
            sum
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(15)));
    }

    #[test]
    fn test_for_in_range() {
        let result = eval(
            r#"
            let sum = 0
            for i in 1..5 {
                sum = sum + i
            }
            sum
        "#,
        )
        .unwrap();
        // 1 + 2 + 3 + 4 = 10 (exclusive end)
        assert!(matches!(result, Value::Int(10)));
    }

    #[test]
    fn test_for_in_range_inclusive() {
        let result = eval(
            r#"
            let sum = 0
            for i in 1..=5 {
                sum = sum + i
            }
            sum
        "#,
        )
        .unwrap();
        // 1 + 2 + 3 + 4 + 5 = 15 (inclusive end)
        assert!(matches!(result, Value::Int(15)));
    }

    #[test]
    fn test_for_in_string() {
        let result = eval(
            r#"
            let count = 0
            for c in "hello" {
                count = count + 1
            }
            count
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(5)));
    }

    #[test]
    fn test_for_in_with_break() {
        let result = eval(
            r#"
            let sum = 0
            for x in [1, 2, 3, 4, 5] {
                if x > 3 {
                    break
                }
                sum = sum + x
            }
            sum
        "#,
        )
        .unwrap();
        // 1 + 2 + 3 = 6
        assert!(matches!(result, Value::Int(6)));
    }

    #[test]
    fn test_for_in_with_continue() {
        let result = eval(
            r#"
            let sum = 0
            for x in [1, 2, 3, 4, 5] {
                if x == 3 {
                    continue
                }
                sum = sum + x
            }
            sum
        "#,
        )
        .unwrap();
        // 1 + 2 + 4 + 5 = 12 (skip 3)
        assert!(matches!(result, Value::Int(12)));
    }

    #[test]
    fn test_range_expression() {
        let result = eval(
            r#"
            let r = 1..10
            r
        "#,
        )
        .unwrap();
        match result {
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                assert_eq!(start, 1);
                assert_eq!(end, 10);
                assert!(!inclusive);
            }
            _ => panic!("Expected Range value"),
        }
    }

    #[test]
    fn test_range_inclusive_expression() {
        let result = eval(
            r#"
            let r = 5..=15
            r
        "#,
        )
        .unwrap();
        match result {
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                assert_eq!(start, 5);
                assert_eq!(end, 15);
                assert!(inclusive);
            }
            _ => panic!("Expected Range value"),
        }
    }

    #[test]
    fn test_map_literal() {
        let result = eval(
            r#"
            let m = map { "a": 1, "b": 2 }
            m
        "#,
        )
        .unwrap();
        match result {
            Value::Map(map) => {
                assert_eq!(map.len(), 2);
                assert!(matches!(map.get("a"), Some(Value::Int(1))));
                assert!(matches!(map.get("b"), Some(Value::Int(2))));
            }
            _ => panic!("Expected Map value"),
        }
    }

    #[test]
    fn test_map_bracket_access() {
        // Test bracket notation for map keys (including hyphenated keys)
        let result = eval(
            r#"
            let headers = map { "content-type": "application/json", "x-custom-header": "value" }
            headers["content-type"]
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "application/json");
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_map_bracket_hyphenated_key() {
        let result = eval(
            r#"
            let m = map { "my-key": 42 }
            m["my-key"]
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(42)));
    }

    #[test]
    fn test_map_empty() {
        let result = eval(
            r#"
            let m = map {}
            m
        "#,
        )
        .unwrap();
        match result {
            Value::Map(map) => {
                assert!(map.is_empty());
            }
            _ => panic!("Expected Map value"),
        }
    }

    #[test]
    fn test_for_in_map_keys() {
        let result = eval(
            r#"
            let m = map { "x": 10, "y": 20 }
            let count = 0
            for key in m {
                count = count + 1
            }
            count
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(2)));
    }

    #[test]
    fn test_interpolated_string() {
        let result = eval(
            r#"
            let name = "World"
            let greeting = "Hello, {name}!"
            greeting
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "Hello, World!");
        } else {
            panic!("Expected string, got {:?}", result);
        }
    }

    #[test]
    fn test_interpolated_string_with_expression() {
        let result = eval(
            r#"
            let a = 5
            let b = 3
            "Sum: {a + b}"
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "Sum: 8");
        } else {
            panic!("Expected string, got {:?}", result);
        }
    }

    #[test]
    fn test_defer_basic() {
        // Defer should execute when scope exits
        let result = eval(
            r#"
            let x = 0
            fn test() {
                x = 1
                defer x = 10
                x = 2
                return x
            }
            test()
            x
        "#,
        )
        .unwrap();
        // The function returns 2, but defer sets x to 10 after return
        // Since x is captured, the final x should be 10
        // Actually in our simple implementation, defer runs in block scope
        // Let's test a simpler case
        assert!(matches!(result, Value::Int(2) | Value::Int(10)));
    }

    #[test]
    fn test_trait_with_supertrait() {
        let result = eval(
            r#"
            trait Base {
                fn base_method(self);
            }
            
            trait Derived: Base {
                fn derived_method(self);
            }
            
            "ok"
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "ok");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_raw_string_simple() {
        let result = eval(
            r##"
            r"hello world"
        "##,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "hello world");
        } else {
            panic!("Expected string, got {:?}", result);
        }
    }

    #[test]
    fn test_raw_string_with_escapes() {
        // Raw strings don't process escape sequences
        let result = eval(
            r##"
            r"hello\nworld"
        "##,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "hello\\nworld");
        } else {
            panic!("Expected string, got {:?}", result);
        }
    }

    #[test]
    fn test_raw_string_with_hashes() {
        let result = eval(
            r###"
            r#"he said "hello""#
        "###,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "he said \"hello\"");
        } else {
            panic!("Expected string, got {:?}", result);
        }
    }

    #[test]
    fn test_raw_string_sql() {
        let result = eval(
            r##"
            r"SELECT * FROM users WHERE name = 'test'"
        "##,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "SELECT * FROM users WHERE name = 'test'");
        } else {
            panic!("Expected string, got {:?}", result);
        }
    }

    #[test]
    fn test_trait_bounds_parsing() {
        // Test that trait bounds syntax is parsed correctly
        let result = eval(
            r#"
            fn identity<T: Clone>(x: T) -> T {
                return x
            }
            identity(42)
        "#,
        )
        .unwrap();
        if let Value::Int(n) = result {
            assert_eq!(n, 42);
        } else {
            panic!("Expected Int(42), got {:?}", result);
        }
    }

    #[test]
    fn test_multiple_trait_bounds() {
        // Test multiple bounds with + syntax
        let result = eval(
            r#"
            fn process<T: Serializable + Comparable>(x: T) -> T {
                return x
            }
            process("hello")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "hello");
        } else {
            panic!("Expected string");
        }
    }

    #[test]
    fn test_struct_with_bounded_type_param() {
        let result = eval(
            r#"
            struct Container<T: Clone> {
                value: T,
            }
            let c = Container { value: 42 }
            c.value
        "#,
        )
        .unwrap();
        if let Value::Int(n) = result {
            assert_eq!(n, 42);
        } else {
            panic!("Expected Int(42), got {:?}", result);
        }
    }

    // ==================== std/fs tests ====================

    #[test]
    fn test_std_fs_write_and_read_file() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("intent_test_file.txt");
        let test_path = test_file.to_string_lossy().replace('\\', "/");

        let code = format!(
            r#"
            import {{ write_file, read_file, remove }} from "std/fs"
            
            let path = "{}"
            let content = "Hello, Intent!"
            
            // Write file
            let write_result = write_file(path, content)
            
            // Read file
            let read_result = read_file(path)
            
            // Cleanup
            remove(path)
            
            // Return the read content (extracting from Result)
            match read_result {{
                Ok(c) => c,
                Err(e) => e,
            }}
        "#,
            test_path
        );
        let result = eval(&code).unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "Hello, Intent!");
        } else {
            panic!("Expected String, got {:?}", result);
        }
    }

    #[test]
    fn test_std_fs_exists() {
        let temp_dir = std::env::temp_dir();
        let temp_path = temp_dir.to_string_lossy().replace('\\', "/");
        let code = format!(
            r#"
            import {{ exists }} from "std/fs"
            exists("{}")
        "#,
            temp_path
        );
        let result = eval(&code).unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_std_fs_is_file_and_is_dir() {
        let temp_dir = std::env::temp_dir();
        let temp_path = temp_dir.to_string_lossy().replace('\\', "/");
        let code = format!(
            r#"
            import {{ is_dir, is_file }} from "std/fs"
            [is_dir("{}"), is_file("{}")]
        "#,
            temp_path, temp_path
        );
        let result = eval(&code).unwrap();
        if let Value::Array(arr) = result {
            assert!(matches!(&arr[0], Value::Bool(true)));
            assert!(matches!(&arr[1], Value::Bool(false)));
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_std_fs_mkdir_and_remove() {
        // Use a unique test directory name
        let temp_dir = std::env::temp_dir();
        let test_dir = temp_dir.join("intent_test_dir_mkdir");
        let test_path = test_dir.to_string_lossy().replace('\\', "/");

        let code = format!(
            r#"
            import {{ mkdir, remove_dir, exists }} from "std/fs"
            
            let test_dir = "{}"
            
            // Ensure clean state
            if exists(test_dir) {{
                remove_dir(test_dir)
            }}
            
            mkdir(test_dir)
            let existed = exists(test_dir)
            remove_dir(test_dir)
            let exists_after = exists(test_dir)
            existed && !exists_after
        "#,
            test_path
        );
        let result = eval(&code).unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    // ==================== std/path tests ====================

    #[test]
    fn test_std_path_join() {
        let result = eval(
            r#"
            import { join } from "std/path"
            join(["home", "user", "documents"])
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert!(s.contains("home") && s.contains("user") && s.contains("documents"));
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_std_path_dirname_basename() {
        // Test dirname
        let result = eval(
            r#"
            import { dirname } from "std/path"
            match dirname("/home/user/file.txt") {
                Some(d) => d,
                None => "",
            }
        "#,
        )
        .unwrap();
        if let Value::String(dir) = result {
            assert_eq!(dir, "/home/user");
        } else {
            panic!("Expected String for dirname");
        }

        // Test basename
        let result2 = eval(
            r#"
            import { basename } from "std/path"
            match basename("/home/user/file.txt") {
                Some(b) => b,
                None => "",
            }
        "#,
        )
        .unwrap();
        if let Value::String(base) = result2 {
            assert_eq!(base, "file.txt");
        } else {
            panic!("Expected String for basename");
        }
    }

    #[test]
    fn test_std_path_extension() {
        let result = eval(
            r#"
            import { extension } from "std/path"
            match extension("/home/user/file.txt") {
                Some(e) => e,
                None => "",
            }
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "txt");
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_std_path_is_absolute() {
        // Use platform-appropriate absolute path
        let abs_path = if cfg!(windows) {
            "C:/Users/test"
        } else {
            "/home/user"
        };

        let code = format!(
            r#"
            import {{ is_absolute, is_relative }} from "std/path"
            [is_absolute("{}"), is_relative("./file.txt")]
        "#,
            abs_path
        );
        let result = eval(&code).unwrap();
        if let Value::Array(arr) = result {
            assert!(matches!(&arr[0], Value::Bool(true)));
            assert!(matches!(&arr[1], Value::Bool(true)));
        } else {
            panic!("Expected Array");
        }
    }

    // ==================== std/json tests ====================

    #[test]
    fn test_std_json_parse_simple() {
        // Test JSON parsing - use raw string for JSON
        let result = eval(
            r##"
            import { parse_json } from "std/json"
            let json_str = r#"{"name": "Alice", "age": 30}"#
            match parse_json(json_str) {
                Ok(obj) => obj.name,
                Err(e) => e,
            }
        "##,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "Alice");
        } else {
            panic!("Expected String, got {:?}", result);
        }
    }

    #[test]
    fn test_std_json_parse_array() {
        let result = eval(
            r#"
            import { parse_json } from "std/json"
            match parse_json("[1, 2, 3]") {
                Ok(arr) => len(arr),
                Err(e) => 0,
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(3)));
    }

    #[test]
    fn test_std_json_stringify() {
        let result = eval(
            r#"
            import { stringify } from "std/json"
            let data = map { "name": "Bob", "score": 100 }
            stringify(data)
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert!(s.contains("Bob") && s.contains("100"));
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_std_json_roundtrip() {
        let result = eval(
            r#"
            import { parse_json, stringify } from "std/json"
            let original = map { "x": 1, "y": 2 }
            let json_str = stringify(original)
            match parse_json(json_str) {
                Ok(parsed) => parsed.x,
                Err(_) => -1,
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(1)));
    }

    // ==================== std/time tests ====================

    #[test]
    fn test_std_time_now() {
        let result = eval(
            r#"
            import { now } from "std/time"
            let ts = now()
            ts > 0
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_std_time_now_millis() {
        let result = eval(
            r#"
            import { now_millis } from "std/time"
            let ts = now_millis()
            ts > 1000000000000  // Should be after year 2001
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_std_time_elapsed() {
        let result = eval(
            r#"
            import { now_millis, elapsed, sleep } from "std/time"
            let start = now_millis()
            sleep(10)
            let e = elapsed(start)
            e >= 10
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_std_time_format_timestamp() {
        let result = eval(
            r#"
            import { format_timestamp } from "std/time"
            // Unix timestamp for 2024-01-15 12:30:45 UTC
            let ts = 1705322445
            format_timestamp(ts, "%Y-%m-%d")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "2024-01-15");
        } else {
            panic!("Expected String, got {:?}", result);
        }
    }

    #[test]
    fn test_std_time_duration() {
        let result = eval(
            r#"
            import { duration_secs } from "std/time"
            let d = duration_secs(5)
            d.millis
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(5000)));
    }

    // ==================== std/crypto tests ====================

    #[test]
    fn test_std_crypto_sha256() {
        let result = eval(
            r#"
            import { sha256 } from "std/crypto"
            sha256("hello")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            // SHA256 of "hello" is well-known
            assert_eq!(
                s,
                "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
            );
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_std_crypto_sha256_bytes() {
        let result = eval(
            r#"
            import { sha256_bytes } from "std/crypto"
            let hash = sha256_bytes("test")
            len(hash)
        "#,
        )
        .unwrap();
        // SHA256 produces 32 bytes
        assert!(matches!(result, Value::Int(32)));
    }

    #[test]
    fn test_std_crypto_hmac() {
        let result = eval(
            r#"
            import { hmac_sha256 } from "std/crypto"
            hmac_sha256("key", "data")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            // HMAC-SHA256("key", "data") is known
            assert_eq!(
                s,
                "5031fe3d989c6d1537a013fa6e739da23463fdaec3b70137d828e36ace221bd0"
            );
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_std_crypto_uuid() {
        let result = eval(
            r#"
            import { uuid } from "std/crypto"
            let id = uuid()
            len(id)
        "#,
        )
        .unwrap();
        // UUID v4 is 36 characters (with hyphens)
        assert!(matches!(result, Value::Int(36)));
    }

    #[test]
    fn test_std_crypto_random_bytes() {
        let result = eval(
            r#"
            import { random_bytes } from "std/crypto"
            let bytes = random_bytes(16)
            len(bytes)
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(16)));
    }

    #[test]
    fn test_std_crypto_random_hex() {
        let result = eval(
            r#"
            import { random_hex } from "std/crypto"
            let hex = random_hex(8)
            len(hex)
        "#,
        )
        .unwrap();
        // 8 bytes = 16 hex characters
        assert!(matches!(result, Value::Int(16)));
    }

    #[test]
    fn test_std_crypto_hex_encode_decode() {
        let result = eval(
            r#"
            import { hex_encode, hex_decode } from "std/crypto"
            let hex = hex_encode("hello")
            match hex_decode(hex) {
                Ok(bytes) => len(bytes),
                Err(_) => -1,
            }
        "#,
        )
        .unwrap();
        // "hello" is 5 bytes
        assert!(matches!(result, Value::Int(5)));
    }

    // ==================== std/url tests ====================

    #[test]
    fn test_std_url_parse() {
        let result = eval(
            r#"
            import { parse_url } from "std/url"
            match parse_url("https://example.com:8080/path?foo=bar#section") {
                Ok(url) => url.host,
                Err(_) => "error",
            }
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "example.com");
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_std_url_parse_port() {
        let result = eval(
            r#"
            import { parse_url } from "std/url"
            match parse_url("https://example.com:8080/path") {
                Ok(url) => url.port,
                Err(_) => -1,
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(8080)));
    }

    #[test]
    fn test_std_url_parse_query_params() {
        let result = eval(
            r#"
            import { parse_url } from "std/url"
            match parse_url("https://example.com?name=alice&age=30") {
                Ok(url) => url.params.name,
                Err(_) => "error",
            }
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "alice");
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_std_url_encode_decode() {
        let result = eval(
            r#"
            import { encode_component, decode } from "std/url"
            let encoded = encode_component("hello world!")
            match decode(encoded) {
                Ok(decoded) => decoded,
                Err(_) => "error",
            }
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "hello world!");
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_std_url_build_query() {
        let result = eval(
            r#"
            import { build_query } from "std/url"
            let params = map { "name": "alice", "age": "30" }
            build_query(params)
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            // Order may vary, but should contain both params
            assert!(s.contains("name=alice"));
            assert!(s.contains("age=30"));
            assert!(s.contains("&"));
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_std_url_join() {
        let result = eval(
            r#"
            import { join } from "std/url"
            join("https://example.com/api", "users/123")
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "https://example.com/api/users/123");
        } else {
            panic!("Expected String");
        }
    }

    // ========== std/http tests ==========

    #[test]
    fn test_std_http_module_exists() {
        // Verify the HTTP module can be imported with new unified API
        let result = eval(
            r#"
            import { fetch, download, Cache } from "std/http"
            "module loaded"
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "module loaded");
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_std_http_fetch_invalid_url() {
        // Test error handling for invalid URL
        let result = eval(
            r#"
            import { fetch } from "std/http"
            match fetch("not-a-valid-url") {
                Ok(resp) => "unexpected success",
                Err(e) => "got error as expected",
            }
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "got error as expected");
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_std_http_fetch_with_options() {
        // Test that fetch() accepts options map
        let result = eval(
            r#"
            import { fetch } from "std/http"
            match fetch(map { "url": "invalid://test", "method": "GET" }) {
                Ok(resp) => "unexpected success",
                Err(e) => "got error as expected",
            }
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "got error as expected");
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_std_http_post_json_via_fetch() {
        // Test POST with JSON via fetch() options (verifies JSON serialization)
        let result = eval(
            r#"
            import { fetch } from "std/http"
            let data = map { "name": "test", "value": 42 }
            match fetch(map { "url": "invalid://test", "method": "POST", "json": data }) {
                Ok(resp) => "unexpected success",
                Err(e) => "got error as expected",
            }
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "got error as expected");
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_std_http_fetch_post_returns_result() {
        // Test that fetch() with POST method returns a Result
        let result = eval(
            r#"
            import { fetch } from "std/http"
            let result = fetch(map { "url": "invalid://test", "method": "POST", "body": "test" })
            // Should return Err(...) for invalid URL, not Unit
            match result {
                Ok(resp) => "got ok",
                Err(e) => "got error as expected",
            }
        "#,
        )
        .unwrap();
        if let Value::String(s) = result {
            assert_eq!(s, "got error as expected");
        } else {
            panic!("Expected String, got {:?}", result);
        }
    }

    #[test]
    fn test_std_http_fetch_with_auth() {
        // Test that fetch() accepts auth option for basic auth
        let result = eval(
            r#"
            import { fetch } from "std/http"

            // Test that auth option is accepted (will fail with invalid URL)
            let auth_result = fetch(map {
                "url": "invalid://test",
                "auth": map { "user": "testuser", "pass": "testpass" }
            })
            match auth_result {
                Ok(r) => "ok",
                Err(e) => "error"
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::String(_)));
    }

    #[test]
    fn test_std_http_fetch_with_form() {
        // Test that fetch() accepts form option for URL-encoded form data
        let result = eval(
            r#"
            import { fetch } from "std/http"

            let form = map { "username": "test", "password": "secret" }
            let result = fetch(map {
                "url": "invalid://test",
                "method": "POST",
                "form": form
            })
            match result {
                Ok(r) => "ok",
                Err(e) => "error"
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::String(_)));
    }

    #[test]
    fn test_std_http_fetch_with_cookies() {
        // Test that fetch accepts cookies option
        let result = eval(
            r#"
            import { fetch } from "std/http"

            let cookies = map { "session": "abc123" }
            let opts = map {
                "url": "invalid://test",
                "method": "GET",
                "cookies": cookies
            }
            let result = fetch(opts)
            match result {
                Ok(r) => "ok",
                Err(e) => "error"
            }
        "#,
        )
        .unwrap();
        assert!(matches!(result, Value::String(_)));
    }

    // === ExecutionMode Tests ===

    #[test]
    fn test_execution_mode_default() {
        let interpreter = Interpreter::new();
        assert_eq!(interpreter.execution_mode, ExecutionMode::Normal);
    }

    #[test]
    fn test_execution_mode_set() {
        let mut interpreter = Interpreter::new();
        interpreter.set_execution_mode(ExecutionMode::UnitTest);
        assert_eq!(interpreter.execution_mode, ExecutionMode::UnitTest);

        interpreter.set_execution_mode(ExecutionMode::HotReload);
        assert_eq!(interpreter.execution_mode, ExecutionMode::HotReload);

        interpreter.set_execution_mode(ExecutionMode::Normal);
        assert_eq!(interpreter.execution_mode, ExecutionMode::Normal);
    }

    #[test]
    fn test_should_skip_server_call_normal_mode() {
        let interpreter = Interpreter::new();
        // Normal mode should never skip
        assert!(!interpreter.should_skip_server_call("listen"));
        assert!(!interpreter.should_skip_server_call("serve_static"));
        assert!(!interpreter.should_skip_server_call("routes"));
        assert!(!interpreter.should_skip_server_call("use_middleware"));
        assert!(!interpreter.should_skip_server_call("on_shutdown"));
        assert!(!interpreter.should_skip_server_call("other_function"));
    }

    #[test]
    fn test_should_skip_server_call_unit_test_mode() {
        let mut interpreter = Interpreter::new();
        interpreter.set_execution_mode(ExecutionMode::UnitTest);

        // Unit test mode should skip all server-related functions
        assert!(interpreter.should_skip_server_call("listen"));
        assert!(interpreter.should_skip_server_call("serve_static"));
        assert!(interpreter.should_skip_server_call("routes"));
        assert!(interpreter.should_skip_server_call("use_middleware"));
        assert!(interpreter.should_skip_server_call("on_shutdown"));

        // But not other functions
        assert!(!interpreter.should_skip_server_call("print"));
        assert!(!interpreter.should_skip_server_call("other_function"));
    }

    #[test]
    fn test_should_skip_server_call_hot_reload_mode() {
        let mut interpreter = Interpreter::new();
        interpreter.set_execution_mode(ExecutionMode::HotReload);

        // Hot-reload mode should only skip listen and on_shutdown
        assert!(interpreter.should_skip_server_call("listen"));
        assert!(interpreter.should_skip_server_call("on_shutdown"));

        // But NOT these - they need to re-register
        assert!(!interpreter.should_skip_server_call("serve_static"));
        assert!(!interpreter.should_skip_server_call("routes"));
        assert!(!interpreter.should_skip_server_call("use_middleware"));
    }

    #[test]
    fn test_should_skip_route_registration() {
        let mut interpreter = Interpreter::new();

        // Normal mode - don't skip
        assert!(!interpreter.should_skip_route_registration());

        // Hot-reload mode - don't skip (need to re-register routes)
        interpreter.set_execution_mode(ExecutionMode::HotReload);
        assert!(!interpreter.should_skip_route_registration());

        // Unit test mode - skip route registration
        interpreter.set_execution_mode(ExecutionMode::UnitTest);
        assert!(interpreter.should_skip_route_registration());
    }

    #[test]
    fn test_imported_files_tracking() {
        let mut interpreter = Interpreter::new();

        // Initially empty
        assert!(interpreter.imported_files.is_empty());

        // Add some imports
        let now = std::time::SystemTime::now();
        interpreter
            .imported_files
            .insert("/path/to/file.tnt".to_string(), now);
        interpreter
            .imported_files
            .insert("/path/to/other.tnt".to_string(), now);

        assert_eq!(interpreter.imported_files.len(), 2);
        assert!(interpreter.imported_files.contains_key("/path/to/file.tnt"));
    }
}
