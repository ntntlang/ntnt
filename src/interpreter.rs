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
    
    /// Struct instance
    Struct {
        name: String,
        fields: HashMap<String, Value>,
    },
    
    /// Function value with contract
    Function {
        name: String,
        params: Vec<Parameter>,
        body: Block,
        closure: Rc<RefCell<Environment>>,
        contract: Option<FunctionContract>,
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
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Unit => false,
            Value::Int(0) => false,
            Value::Float(f) if *f == 0.0 => false,
            Value::String(s) if s.is_empty() => false,
            Value::Array(a) if a.is_empty() => false,
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
            Value::Struct { name, .. } => name,
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
            Value::Struct { name, fields } => {
                let field_strs: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v))
                    .collect();
                write!(f, "{} {{ {} }}", name, field_strs.join(", "))
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

/// The Intent interpreter
pub struct Interpreter {
    environment: Rc<RefCell<Environment>>,
    contracts: ContractChecker,
    /// Struct type definitions
    structs: HashMap<String, Vec<Field>>,
    /// Struct invariants
    struct_invariants: HashMap<String, Vec<Expression>>,
    /// Old values for current function call (used in postconditions)
    current_old_values: Option<OldValues>,
    /// Current function's result value (used in postconditions)
    current_result: Option<Value>,
}

impl Interpreter {
    pub fn new() -> Self {
        let env = Rc::new(RefCell::new(Environment::new()));
        let mut interpreter = Interpreter {
            environment: env,
            contracts: ContractChecker::new(),
            structs: HashMap::new(),
            struct_invariants: HashMap::new(),
            current_old_values: None,
            current_result: None,
        };
        interpreter.define_builtins();
        interpreter
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
                func: |args| {
                    match &args[0] {
                        Value::String(s) => Ok(Value::Int(s.len() as i64)),
                        Value::Array(a) => Ok(Value::Int(a.len() as i64)),
                        _ => Err(IntentError::TypeError(
                            "len() requires a string or array".to_string(),
                        )),
                    }
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
                func: |args| {
                    match &args[0] {
                        Value::Int(n) => Ok(Value::Int(*n)),
                        Value::Float(f) => Ok(Value::Int(*f as i64)),
                        Value::String(s) => s
                            .parse::<i64>()
                            .map(Value::Int)
                            .map_err(|_| IntentError::TypeError("Cannot parse as int".to_string())),
                        Value::Bool(b) => Ok(Value::Int(if *b { 1 } else { 0 })),
                        _ => Err(IntentError::TypeError("Cannot convert to int".to_string())),
                    }
                },
            },
        );

        // Float conversion
        self.environment.borrow_mut().define(
            "float".to_string(),
            Value::NativeFunction {
                name: "float".to_string(),
                arity: 1,
                func: |args| {
                    match &args[0] {
                        Value::Int(n) => Ok(Value::Float(*n as f64)),
                        Value::Float(f) => Ok(Value::Float(*f)),
                        Value::String(s) => s
                            .parse::<f64>()
                            .map(Value::Float)
                            .map_err(|_| IntentError::TypeError("Cannot parse as float".to_string())),
                        _ => Err(IntentError::TypeError("Cannot convert to float".to_string())),
                    }
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
                        Err(IntentError::TypeError("push() requires an array".to_string()))
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
                        Err(IntentError::ContractViolation("Assertion failed".to_string()))
                    }
                },
            },
        );
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
            } => {
                let val = if let Some(expr) = value {
                    self.eval_expression(expr)?
                } else {
                    Value::Unit
                };
                self.environment.borrow_mut().define(name.clone(), val);
                Ok(Value::Unit)
            }

            Statement::Function {
                name,
                params,
                return_type: _,
                contract,
                body,
                attributes: _,
            } => {
                // Convert AST Contract to FunctionContract with expressions
                let func_contract = contract.as_ref().map(|c| {
                    FunctionContract {
                        requires: c.requires.clone(),
                        ensures: c.ensures.clone(),
                    }
                });

                let func = Value::Function {
                    name: name.clone(),
                    params: params.clone(),
                    body: body.clone(),
                    closure: Rc::clone(&self.environment),
                    contract: func_contract,
                };
                self.environment.borrow_mut().define(name.clone(), func);
                Ok(Value::Unit)
            }

            Statement::Struct {
                name,
                fields,
                attributes: _,
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
                methods,
                invariants,
            } => {
                // Store invariants for this type
                if !invariants.is_empty() {
                    self.struct_invariants.insert(type_name.clone(), invariants.clone());
                }
                
                for method in methods {
                    self.eval_statement(method)?;
                }
                Ok(Value::Unit)
            }

            Statement::Enum { .. } => {
                // TODO: Implement enum support
                Ok(Value::Unit)
            }

            Statement::Protocol { .. } => {
                // TODO: Implement protocol support
                Ok(Value::Unit)
            }

            Statement::Intent { description: _, target } => {
                self.eval_statement(target)
            }
        }
    }

    fn eval_block(&mut self, block: &Block) -> Result<Value> {
        let previous = Rc::clone(&self.environment);
        self.environment = Rc::new(RefCell::new(Environment::with_parent(Rc::clone(&previous))));

        let mut result = Value::Unit;
        for stmt in &block.statements {
            result = self.eval_statement(stmt)?;
            // Propagate control flow
            match result {
                Value::Return(_) | Value::Break | Value::Continue => break,
                _ => {}
            }
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

            Expression::Identifier(name) => {
                self.environment
                    .borrow()
                    .get(name)
                    .ok_or_else(|| IntentError::UndefinedVariable(name.clone()))
            }

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
                let vals: Result<Vec<Value>> = elements
                    .iter()
                    .map(|e| self.eval_expression(e))
                    .collect();
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
                        arr.get(index).cloned().ok_or_else(|| {
                            IntentError::IndexOutOfBounds {
                                index: i,
                                length: arr.len(),
                            }
                        })
                    }
                    (Value::String(s), Value::Int(i)) => {
                        let index = if i < 0 {
                            (s.len() as i64 + i) as usize
                        } else {
                            i as usize
                        };
                        s.chars()
                            .nth(index)
                            .map(|c| Value::String(c.to_string()))
                            .ok_or_else(|| IntentError::IndexOutOfBounds {
                                index: i,
                                length: s.len(),
                            })
                    }
                    _ => Err(IntentError::TypeError("Invalid index operation".to_string())),
                }
            }

            Expression::FieldAccess { object, field } => {
                let obj = self.eval_expression(object)?;
                match obj {
                    Value::Struct { fields, .. } => {
                        fields.get(field).cloned().ok_or_else(|| {
                            IntentError::RuntimeError(format!("Unknown field: {}", field))
                        })
                    }
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

            Expression::Assign { target, value } => {
                let val = self.eval_expression(value)?;
                match target.as_ref() {
                    Expression::Identifier(name) => {
                        if self.environment.borrow_mut().set(name, val.clone()) {
                            // After assignment, check if this is a struct and verify invariants
                            if let Value::Struct { name: struct_name, .. } = &val {
                                self.check_struct_invariants(struct_name, &val)?;
                            }
                            Ok(val)
                        } else {
                            Err(IntentError::UndefinedVariable(name.clone()))
                        }
                    }
                    Expression::FieldAccess { object, field } => {
                        // Handle field assignment (e.g., obj.field = value)
                        if let Expression::Identifier(var_name) = object.as_ref() {
                            // Get the current struct
                            let current = self.environment.borrow().get(var_name)
                                .ok_or_else(|| IntentError::UndefinedVariable(var_name.clone()))?;
                            
                            if let Value::Struct { name: struct_name, mut fields } = current {
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
                                    Err(IntentError::RuntimeError(
                                        format!("Unknown field '{}' on struct '{}'", field, struct_name)
                                    ))
                                }
                            } else {
                                Err(IntentError::RuntimeError(
                                    format!("Cannot assign field on non-struct value")
                                ))
                            }
                        } else {
                            Err(IntentError::RuntimeError(
                                "Cannot assign to complex field access".to_string()
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

            Expression::Lambda { params, body } => {
                Ok(Value::Function {
                    name: "<lambda>".to_string(),
                    params: params.clone(),
                    body: Block {
                        statements: vec![Statement::Return(Some(body.as_ref().clone()))],
                    },
                    closure: Rc::clone(&self.environment),
                    contract: None,
                })
            }

            Expression::MethodCall { object, method, arguments } => {
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
                    Err(IntentError::UndefinedFunction(method.clone()))
                }
            }

            Expression::Match { .. } => {
                // TODO: Implement pattern matching
                Err(IntentError::RuntimeError(
                    "Match expressions not yet implemented".to_string(),
                ))
            }

            Expression::Await(_) | Expression::Try(_) => {
                // TODO: Implement async/try
                Err(IntentError::RuntimeError(
                    "Async/Try not yet implemented".to_string(),
                ))
            }
        }
    }

    fn call_function(&mut self, callee: Value, args: Vec<Value>) -> Result<Value> {
        match callee {
            Value::Function {
                name,
                params,
                body,
                closure,
                contract,
            } => {
                if args.len() != params.len() {
                    return Err(IntentError::ArityMismatch {
                        expected: params.len(),
                        got: args.len(),
                    });
                }

                // Create new environment with closure as parent
                let func_env = Rc::new(RefCell::new(Environment::with_parent(closure)));

                // Bind parameters
                for (param, arg) in params.iter().zip(args.iter()) {
                    func_env.borrow_mut().define(param.name.clone(), arg.clone());
                }

                // Save current environment and switch to function's environment
                let previous = Rc::clone(&self.environment);
                self.environment = Rc::clone(&func_env);

                // Check preconditions BEFORE execution
                if let Some(ref func_contract) = contract {
                    for req_expr in &func_contract.requires {
                        let condition_str = Self::format_expression(req_expr);
                        let result = self.eval_expression(req_expr)?;
                        if !result.is_truthy() {
                            self.environment = previous;
                            return Err(IntentError::ContractViolation(
                                format!("Precondition failed in '{}': {}", name, condition_str)
                            ));
                        }
                        self.contracts.check_precondition(&condition_str, true, None)?;
                    }

                    // Capture old values for postconditions containing old()
                    self.current_old_values = Some(self.capture_old_values(&func_contract.ensures)?);
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

                // Store result for postcondition evaluation
                self.current_result = Some(result.clone());
                
                // Bind 'result' in environment for postcondition evaluation
                self.environment.borrow_mut().define("result".to_string(), result.clone());

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
                            return Err(IntentError::ContractViolation(
                                format!("Postcondition failed in '{}': {}", name, condition_str)
                            ));
                        }
                        self.contracts.check_postcondition(&condition_str, true, None)?;
                    }
                }

                // Clear contract evaluation state
                self.current_old_values = None;
                self.current_result = None;
                
                // Restore environment
                self.environment = previous;

                Ok(result)
            }

            Value::NativeFunction { name: _, arity, func } => {
                if args.len() != arity && arity != 0 {
                    return Err(IntentError::ArityMismatch {
                        expected: arity,
                        got: args.len(),
                    });
                }
                func(&args)
            }

            _ => Err(IntentError::TypeError(
                "Can only call functions".to_string(),
            )),
        }
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
            Expression::Call { function, arguments } => {
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
    fn value_to_stored(&self, value: &Value) -> StoredValue {
        match value {
            Value::Int(n) => StoredValue::Int(*n),
            Value::Float(f) => StoredValue::Float(*f),
            Value::Bool(b) => StoredValue::Bool(*b),
            Value::String(s) => StoredValue::String(s.clone()),
            Value::Array(arr) => StoredValue::Array(
                arr.iter().map(|v| self.value_to_stored(v)).collect()
            ),
            Value::Unit => StoredValue::Unit,
            _ => StoredValue::Unit, // Functions and other complex types stored as Unit
        }
    }
    
    /// Convert a StoredValue back to a runtime Value
    fn stored_to_value(&self, stored: &StoredValue) -> Value {
        match stored {
            StoredValue::Int(n) => Value::Int(*n),
            StoredValue::Float(f) => Value::Float(*f),
            StoredValue::Bool(b) => Value::Bool(*b),
            StoredValue::String(s) => Value::String(s.clone()),
            StoredValue::Array(arr) => Value::Array(
                arr.iter().map(|v| self.stored_to_value(v)).collect()
            ),
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
            Expression::Binary { left, operator, right } => {
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
                };
                format!("{} {} {}", Self::format_expression(left), op_str, Self::format_expression(right))
            }
            Expression::Unary { operator, operand } => {
                let op_str = match operator {
                    UnaryOp::Neg => "-",
                    UnaryOp::Not => "!",
                };
                format!("{}{}", op_str, Self::format_expression(operand))
            }
            Expression::Call { function, arguments } => {
                let func_str = Self::format_expression(function);
                let args_str: Vec<String> = arguments.iter().map(Self::format_expression).collect();
                format!("{}({})", func_str, args_str.join(", "))
            }
            Expression::FieldAccess { object, field } => {
                format!("{}.{}", Self::format_expression(object), field)
            }
            Expression::Index { object, index } => {
                format!("{}[{}]", Self::format_expression(object), Self::format_expression(index))
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
            inv_env.borrow_mut().define(field_name.clone(), field_val.clone());
        }
        inv_env.borrow_mut().define("self".to_string(), struct_val.clone());
        
        self.environment = inv_env;
        
        // Check each invariant
        for inv_expr in &invariants {
            let condition_str = Self::format_expression(inv_expr);
            let result = self.eval_expression(inv_expr)?;
            
            if !result.is_truthy() {
                self.environment = previous;
                return Err(IntentError::ContractViolation(
                    format!("Invariant violated for '{}': {}", struct_name, condition_str)
                ));
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
            (BinaryOp::Pow, Value::Int(a), Value::Int(b)) => {
                Ok(Value::Int(a.pow(b as u32)))
            }

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
            (BinaryOp::Add, Value::String(a), b) => {
                Ok(Value::String(format!("{}{}", a, b)))
            }
            (BinaryOp::Add, a, Value::String(b)) => {
                Ok(Value::String(format!("{}{}", a, b)))
            }

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
        let result = eval(
            "let x = 0; while x < 5 { x = x + 1; } x"
        ).unwrap();
        assert!(matches!(result, Value::Int(5)));
    }

    #[test]
    fn test_contract_precondition_passes() {
        // Precondition passes when b != 0
        let result = eval(r#"
            fn divide(a, b) requires b != 0 { return a / b; }
            divide(10, 2)
        "#).unwrap();
        assert!(matches!(result, Value::Int(5)));
    }

    #[test]
    fn test_contract_precondition_fails() {
        // Precondition fails when b == 0
        let result = eval(r#"
            fn divide(a, b) requires b != 0 { return a / b; }
            divide(10, 0)
        "#);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Precondition failed"));
    }

    #[test]
    fn test_contract_postcondition_passes() {
        // Postcondition passes when result >= 0
        let result = eval(r#"
            fn absolute(x) ensures result >= 0 { 
                if x < 0 { return -x; } 
                return x; 
            }
            absolute(-5)
        "#).unwrap();
        assert!(matches!(result, Value::Int(5)));
    }

    #[test]
    fn test_contract_postcondition_fails() {
        // Postcondition fails intentionally
        let result = eval(r#"
            fn bad_absolute(x) ensures result > 100 { 
                if x < 0 { return -x; } 
                return x; 
            }
            bad_absolute(5)
        "#);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Postcondition failed"));
    }

    #[test]
    fn test_contract_with_result() {
        // Use result keyword in postcondition
        let result = eval(r#"
            fn double(x) ensures result == x * 2 { 
                return x * 2; 
            }
            double(7)
        "#).unwrap();
        assert!(matches!(result, Value::Int(14)));
    }

    #[test]
    fn test_contract_with_old() {
        // Use old() to capture pre-execution value
        let result = eval(r#"
            fn increment(x) ensures result == old(x) + 1 { 
                return x + 1; 
            }
            increment(10)
        "#).unwrap();
        assert!(matches!(result, Value::Int(11)));
    }

    #[test]
    fn test_multiple_contracts() {
        // Multiple requires and ensures
        let result = eval(r#"
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
        "#).unwrap();
        assert!(matches!(result, Value::Int(10)));
    }

    #[test]
    fn test_struct_literal() {
        // Basic struct literal creation
        let result = eval(r#"
            struct Point {
                x: Int,
                y: Int
            }
            let p = Point { x: 10, y: 20 };
            p.x + p.y
        "#).unwrap();
        assert!(matches!(result, Value::Int(30)));
    }

    #[test]
    fn test_struct_invariant_passes() {
        // Struct invariant passes on construction
        let result = eval(r#"
            struct Counter {
                value: Int
            }
            impl Counter {
                invariant self.value >= 0
            }
            let c = Counter { value: 5 };
            c.value
        "#).unwrap();
        assert!(matches!(result, Value::Int(5)));
    }

    #[test]
    fn test_struct_invariant_fails() {
        // Struct invariant fails on construction
        let result = eval(r#"
            struct Counter {
                value: Int
            }
            impl Counter {
                invariant self.value >= 0
            }
            let c = Counter { value: -1 };
            c.value
        "#);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Invariant violated"));
    }
}
