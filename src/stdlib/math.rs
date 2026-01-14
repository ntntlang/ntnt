//! std/math module - Mathematical functions and constants

use std::collections::HashMap;
use crate::interpreter::Value;
use crate::error::IntentError;
use rand::Rng;

/// Initialize the std/math module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();
    
    // PI constant
    module.insert("PI".to_string(), Value::Float(std::f64::consts::PI));
    
    // E constant (Euler's number)
    module.insert("E".to_string(), Value::Float(std::f64::consts::E));
    
    // TAU constant (2*PI)
    module.insert("TAU".to_string(), Value::Float(std::f64::consts::TAU));
    
    // INFINITY constant
    module.insert("INFINITY".to_string(), Value::Float(f64::INFINITY));
    
    // NEG_INFINITY constant
    module.insert("NEG_INFINITY".to_string(), Value::Float(f64::NEG_INFINITY));
    
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
    
    // sinh(x) -> Float (hyperbolic sine)
    module.insert("sinh".to_string(), Value::NativeFunction {
        name: "sinh".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("sinh() requires a number".to_string())),
            };
            Ok(Value::Float(x.sinh()))
        },
    });
    
    // cosh(x) -> Float (hyperbolic cosine)
    module.insert("cosh".to_string(), Value::NativeFunction {
        name: "cosh".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("cosh() requires a number".to_string())),
            };
            Ok(Value::Float(x.cosh()))
        },
    });
    
    // tanh(x) -> Float (hyperbolic tangent)
    module.insert("tanh".to_string(), Value::NativeFunction {
        name: "tanh".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("tanh() requires a number".to_string())),
            };
            Ok(Value::Float(x.tanh()))
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
    
    // log2(x) -> Float
    module.insert("log2".to_string(), Value::NativeFunction {
        name: "log2".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("log2() requires a number".to_string())),
            };
            if x <= 0.0 {
                return Err(IntentError::RuntimeError("log2() requires positive number".to_string()));
            }
            Ok(Value::Float(x.log2()))
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
    
    // exp2(x) -> Float (2^x)
    module.insert("exp2".to_string(), Value::NativeFunction {
        name: "exp2".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("exp2() requires a number".to_string())),
            };
            Ok(Value::Float(x.exp2()))
        },
    });
    
    // cbrt(x) -> Float (cube root)
    module.insert("cbrt".to_string(), Value::NativeFunction {
        name: "cbrt".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("cbrt() requires a number".to_string())),
            };
            Ok(Value::Float(x.cbrt()))
        },
    });
    
    // hypot(x, y) -> Float (sqrt(x^2 + y^2))
    module.insert("hypot".to_string(), Value::NativeFunction {
        name: "hypot".to_string(),
        arity: 2,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("hypot() requires numbers".to_string())),
            };
            let y = match &args[1] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("hypot() requires numbers".to_string())),
            };
            Ok(Value::Float(x.hypot(y)))
        },
    });
    
    // degrees(x) -> Float (convert radians to degrees)
    module.insert("degrees".to_string(), Value::NativeFunction {
        name: "degrees".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("degrees() requires a number".to_string())),
            };
            Ok(Value::Float(x.to_degrees()))
        },
    });
    
    // radians(x) -> Float (convert degrees to radians)
    module.insert("radians".to_string(), Value::NativeFunction {
        name: "radians".to_string(),
        arity: 1,
        func: |args| {
            let x = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("radians() requires a number".to_string())),
            };
            Ok(Value::Float(x.to_radians()))
        },
    });
    
    // random() -> Float (random number between 0 and 1)
    module.insert("random".to_string(), Value::NativeFunction {
        name: "random".to_string(),
        arity: 0,
        func: |_args| {
            let mut rng = rand::thread_rng();
            Ok(Value::Float(rng.gen::<f64>()))
        },
    });
    
    // random_int(min, max) -> Int (random integer between min and max inclusive)
    module.insert("random_int".to_string(), Value::NativeFunction {
        name: "random_int".to_string(),
        arity: 2,
        func: |args| {
            let min = match &args[0] {
                Value::Int(i) => *i,
                _ => return Err(IntentError::TypeError("random_int() requires integers".to_string())),
            };
            let max = match &args[1] {
                Value::Int(i) => *i,
                _ => return Err(IntentError::TypeError("random_int() requires integers".to_string())),
            };
            if min > max {
                return Err(IntentError::RuntimeError("random_int() min must be <= max".to_string()));
            }
            let mut rng = rand::thread_rng();
            Ok(Value::Int(rng.gen_range(min..=max)))
        },
    });
    
    // random_range(min, max) -> Float (random float between min and max)
    module.insert("random_range".to_string(), Value::NativeFunction {
        name: "random_range".to_string(),
        arity: 2,
        func: |args| {
            let min = match &args[0] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("random_range() requires numbers".to_string())),
            };
            let max = match &args[1] {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                _ => return Err(IntentError::TypeError("random_range() requires numbers".to_string())),
            };
            if min > max {
                return Err(IntentError::RuntimeError("random_range() min must be <= max".to_string()));
            }
            let mut rng = rand::thread_rng();
            Ok(Value::Float(rng.gen_range(min..max)))
        },
    });
    
    // is_nan(x) -> Bool
    module.insert("is_nan".to_string(), Value::NativeFunction {
        name: "is_nan".to_string(),
        arity: 1,
        func: |args| {
            let result = match &args[0] {
                Value::Float(f) => f.is_nan(),
                Value::Int(_) => false,
                _ => return Err(IntentError::TypeError("is_nan() requires a number".to_string())),
            };
            Ok(Value::Bool(result))
        },
    });
    
    // is_infinite(x) -> Bool
    module.insert("is_infinite".to_string(), Value::NativeFunction {
        name: "is_infinite".to_string(),
        arity: 1,
        func: |args| {
            let result = match &args[0] {
                Value::Float(f) => f.is_infinite(),
                Value::Int(_) => false,
                _ => return Err(IntentError::TypeError("is_infinite() requires a number".to_string())),
            };
            Ok(Value::Bool(result))
        },
    });
    
    // is_finite(x) -> Bool
    module.insert("is_finite".to_string(), Value::NativeFunction {
        name: "is_finite".to_string(),
        arity: 1,
        func: |args| {
            let result = match &args[0] {
                Value::Float(f) => f.is_finite(),
                Value::Int(_) => true,
                _ => return Err(IntentError::TypeError("is_finite() requires a number".to_string())),
            };
            Ok(Value::Bool(result))
        },
    });
    
    module
}
