//! std/math module - Mathematical functions and constants

use std::collections::HashMap;
use crate::interpreter::Value;
use crate::error::IntentError;

/// Initialize the std/math module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();
    
    // PI constant
    module.insert("PI".to_string(), Value::Float(std::f64::consts::PI));
    
    // E constant
    module.insert("E".to_string(), Value::Float(std::f64::consts::E));
    
    // sin(x) -> Float
    module.insert("sin".to_string(), Value::NativeFunction {
        name: "sin".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("sin() requires a number".to_string())),
            };
            Ok(Value::Float(x.sin()))
        },
    });
    
    // cos(x) -> Float
    module.insert("cos".to_string(), Value::NativeFunction {
        name: "cos".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("cos() requires a number".to_string())),
            };
            Ok(Value::Float(x.cos()))
        },
    });
    
    // tan(x) -> Float
    module.insert("tan".to_string(), Value::NativeFunction {
        name: "tan".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("tan() requires a number".to_string())),
            };
            Ok(Value::Float(x.tan()))
        },
    });
    
    // asin(x) -> Float
    module.insert("asin".to_string(), Value::NativeFunction {
        name: "asin".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("asin() requires a number".to_string())),
            };
            Ok(Value::Float(x.asin()))
        },
    });
    
    // acos(x) -> Float
    module.insert("acos".to_string(), Value::NativeFunction {
        name: "acos".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("acos() requires a number".to_string())),
            };
            Ok(Value::Float(x.acos()))
        },
    });
    
    // atan(x) -> Float
    module.insert("atan".to_string(), Value::NativeFunction {
        name: "atan".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("atan() requires a number".to_string())),
            };
            Ok(Value::Float(x.atan()))
        },
    });
    
    // atan2(y, x) -> Float
    module.insert("atan2".to_string(), Value::NativeFunction {
        name: "atan2".to_string(),
        arity: 2,
        func: |args| {
            let y = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("atan2() requires numbers".to_string())),
            };
            let x = match &args[1] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("atan2() requires numbers".to_string())),
            };
            Ok(Value::Float(y.atan2(x)))
        },
    });
    
    // log(x) -> Float (natural log)
    module.insert("log".to_string(), Value::NativeFunction {
        name: "log".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("log() requires a number".to_string())),
            };
            if x <= 0.0 {
                return Err(IntentError::RuntimeError("log() requires positive number".to_string()));
            }
            Ok(Value::Float(x.ln()))
        },
    });
    
    // log10(x) -> Float
    module.insert("log10".to_string(), Value::NativeFunction {
        name: "log10".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("log10() requires a number".to_string())),
            };
            if x <= 0.0 {
                return Err(IntentError::RuntimeError("log10() requires positive number".to_string()));
            }
            Ok(Value::Float(x.log10()))
        },
    });
    
    // exp(x) -> Float (e^x)
    module.insert("exp".to_string(), Value::NativeFunction {
        name: "exp".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("exp() requires a number".to_string())),
            };
            Ok(Value::Float(x.exp()))
        },
    });
    
    module
}
