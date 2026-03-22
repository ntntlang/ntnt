//! std/math module - Mathematical functions and constants

use crate::error::IntentError;
use crate::interpreter::Value;
use rand::Rng;
use std::collections::HashMap;

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

    // INT_MAX constant (largest 64-bit signed integer)
    module.insert("INT_MAX".to_string(), Value::Int(i64::MAX));

    // INT_MIN constant (smallest 64-bit signed integer)
    module.insert("INT_MIN".to_string(), Value::Int(i64::MIN));

    // @ntnt sin
    // @module std/math
    // @module_description Mathematical functions and constants
    // @signature sin(x: Number) -> Float
    // Compute the sine of an angle in radians.
    //
    // Returns the sine of the given angle. Accepts Int or Float.
    // @param x The angle in radians.
    // @returns The sine of x as a Float.
    // @see_also cos, tan, asin, sinh, degrees, radians
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example sin(0) => 0.0 ~ "Sine of zero"
    // @example sin(PI / 2) => 1.0 ~ "Sine of pi/2"
    // @error TypeError ~ "sin() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "sin".to_string(),
        Value::NativeFunction {
            name: "sin".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "sin() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.sin()))
            },
        },
    );

    // @ntnt cos
    // @module std/math
    // @signature cos(x: Number) -> Float
    // Compute the cosine of an angle in radians.
    //
    // Returns the cosine of the given angle. Accepts Int or Float.
    // @param x The angle in radians.
    // @returns The cosine of x as a Float.
    // @see_also sin, tan, acos, cosh, degrees, radians
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example cos(0) => 1.0 ~ "Cosine of zero"
    // @example cos(PI) => -1.0 ~ "Cosine of pi"
    // @error TypeError ~ "cos() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "cos".to_string(),
        Value::NativeFunction {
            name: "cos".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "cos() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.cos()))
            },
        },
    );

    // @ntnt tan
    // @module std/math
    // @signature tan(x: Number) -> Float
    // Compute the tangent of an angle in radians.
    //
    // Returns the tangent of the given angle. Accepts Int or Float.
    // @param x The angle in radians.
    // @returns The tangent of x as a Float.
    // @see_also sin, cos, atan, atan2, tanh, degrees, radians
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example tan(0) => 0.0 ~ "Tangent of zero"
    // @error TypeError ~ "tan() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "tan".to_string(),
        Value::NativeFunction {
            name: "tan".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "tan() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.tan()))
            },
        },
    );

    // @ntnt asin
    // @module std/math
    // @signature asin(x: Number) -> Float
    // Compute the arcsine (inverse sine) of a value.
    //
    // Returns the angle in radians whose sine is x. The input must
    // be in the range [-1, 1]; values outside produce NaN.
    // @param x A value in the range [-1, 1].
    // @returns The arcsine of x in radians as a Float.
    // @see_also sin, acos, atan, degrees
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example asin(0) => 0.0 ~ "Arcsine of zero"
    // @example asin(1) => 1.5707963267948966 ~ "Arcsine of one is pi/2"
    // @error TypeError ~ "asin() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "asin".to_string(),
        Value::NativeFunction {
            name: "asin".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "asin() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.asin()))
            },
        },
    );

    // @ntnt acos
    // @module std/math
    // @signature acos(x: Number) -> Float
    // Compute the arccosine (inverse cosine) of a value.
    //
    // Returns the angle in radians whose cosine is x. The input must
    // be in the range [-1, 1]; values outside produce NaN.
    // @param x A value in the range [-1, 1].
    // @returns The arccosine of x in radians as a Float.
    // @see_also cos, asin, atan, degrees
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example acos(1) => 0.0 ~ "Arccosine of one"
    // @example acos(0) => 1.5707963267948966 ~ "Arccosine of zero is pi/2"
    // @error TypeError ~ "acos() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "acos".to_string(),
        Value::NativeFunction {
            name: "acos".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "acos() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.acos()))
            },
        },
    );

    // @ntnt atan
    // @module std/math
    // @signature atan(x: Number) -> Float
    // Compute the arctangent (inverse tangent) of a value.
    //
    // Returns the angle in radians whose tangent is x.
    // @param x The value to compute the arctangent of.
    // @returns The arctangent of x in radians as a Float.
    // @see_also tan, atan2, asin, acos, degrees
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example atan(0) => 0.0 ~ "Arctangent of zero"
    // @example atan(1) => 0.7853981633974483 ~ "Arctangent of one is pi/4"
    // @error TypeError ~ "atan() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "atan".to_string(),
        Value::NativeFunction {
            name: "atan".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "atan() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.atan()))
            },
        },
    );

    // @ntnt atan2
    // @module std/math
    // @signature atan2(y: Number, x: Number) -> Float
    // Compute the two-argument arctangent of y and x.
    //
    // Returns the angle in radians between the positive x-axis and the
    // point (x, y), using the signs of both arguments to determine the
    // correct quadrant.
    // @param y The y-coordinate.
    // @param x The x-coordinate.
    // @returns The angle in radians as a Float.
    // @see_also atan, sin, cos, tan, degrees
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example atan2(0, 1) => 0.0 ~ "Angle to (1,0) is zero"
    // @example atan2(1, 0) => 1.5707963267948966 ~ "Angle to (0,1) is pi/2"
    // @error TypeError ~ "atan2() requires numbers" fix: "Pass Int or Float arguments"
    module.insert(
        "atan2".to_string(),
        Value::NativeFunction {
            name: "atan2".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                let y = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "atan2() requires numbers".to_string(),
                        ))
                    }
                };
                let x = match &args[1] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "atan2() requires numbers".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(y.atan2(x)))
            },
        },
    );

    // @ntnt sinh
    // @module std/math
    // @signature sinh(x: Number) -> Float
    // Compute the hyperbolic sine of a value.
    //
    // Returns the hyperbolic sine of x. Accepts Int or Float.
    // @param x The input value.
    // @returns The hyperbolic sine of x as a Float.
    // @see_also cosh, tanh, sin
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example sinh(0) => 0.0 ~ "Hyperbolic sine of zero"
    // @example sinh(1) => 1.1752011936438014 ~ "Hyperbolic sine of one"
    // @error TypeError ~ "sinh() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "sinh".to_string(),
        Value::NativeFunction {
            name: "sinh".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "sinh() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.sinh()))
            },
        },
    );

    // @ntnt cosh
    // @module std/math
    // @signature cosh(x: Number) -> Float
    // Compute the hyperbolic cosine of a value.
    //
    // Returns the hyperbolic cosine of x. Accepts Int or Float.
    // @param x The input value.
    // @returns The hyperbolic cosine of x as a Float.
    // @see_also sinh, tanh, cos
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example cosh(0) => 1.0 ~ "Hyperbolic cosine of zero"
    // @example cosh(1) => 1.5430806348152437 ~ "Hyperbolic cosine of one"
    // @error TypeError ~ "cosh() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "cosh".to_string(),
        Value::NativeFunction {
            name: "cosh".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "cosh() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.cosh()))
            },
        },
    );

    // @ntnt tanh
    // @module std/math
    // @signature tanh(x: Number) -> Float
    // Compute the hyperbolic tangent of a value.
    //
    // Returns the hyperbolic tangent of x. Accepts Int or Float.
    // @param x The input value.
    // @returns The hyperbolic tangent of x as a Float.
    // @see_also sinh, cosh, tan
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example tanh(0) => 0.0 ~ "Hyperbolic tangent of zero"
    // @example tanh(1) => 0.7615941559557649 ~ "Hyperbolic tangent of one"
    // @error TypeError ~ "tanh() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "tanh".to_string(),
        Value::NativeFunction {
            name: "tanh".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "tanh() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.tanh()))
            },
        },
    );

    // @ntnt log
    // @module std/math
    // @signature log(x: Number) -> Float
    // Compute the natural logarithm (base e) of a value.
    //
    // Returns ln(x). The input must be a positive number;
    // zero or negative values produce a RuntimeError.
    // @param x A positive number.
    // @returns The natural logarithm of x as a Float.
    // @see_also log10, log2, exp
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example log(1) => 0.0 ~ "Natural log of one"
    // @example log(E) => 1.0 ~ "Natural log of e"
    // @error TypeError ~ "log() requires a number" fix: "Pass an Int or Float argument"
    // @error RuntimeError ~ "log() requires positive number" fix: "Ensure the argument is greater than zero"
    module.insert(
        "log".to_string(),
        Value::NativeFunction {
            name: "log".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "log() requires a number".to_string(),
                        ))
                    }
                };
                if x <= 0.0 {
                    return Err(IntentError::runtime_error(
                        "log() requires positive number".to_string(),
                    ));
                }
                Ok(Value::Float(x.ln()))
            },
        },
    );

    // @ntnt log10
    // @module std/math
    // @signature log10(x: Number) -> Float
    // Compute the base-10 logarithm of a value.
    //
    // Returns log10(x). The input must be a positive number;
    // zero or negative values produce a RuntimeError.
    // @param x A positive number.
    // @returns The base-10 logarithm of x as a Float.
    // @see_also log, log2, exp
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example log10(1) => 0.0 ~ "Log base 10 of one"
    // @example log10(100) => 2.0 ~ "Log base 10 of one hundred"
    // @error TypeError ~ "log10() requires a number" fix: "Pass an Int or Float argument"
    // @error RuntimeError ~ "log10() requires positive number" fix: "Ensure the argument is greater than zero"
    module.insert(
        "log10".to_string(),
        Value::NativeFunction {
            name: "log10".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "log10() requires a number".to_string(),
                        ))
                    }
                };
                if x <= 0.0 {
                    return Err(IntentError::runtime_error(
                        "log10() requires positive number".to_string(),
                    ));
                }
                Ok(Value::Float(x.log10()))
            },
        },
    );

    // @ntnt log2
    // @module std/math
    // @signature log2(x: Number) -> Float
    // Compute the base-2 logarithm of a value.
    //
    // Returns log2(x). The input must be a positive number;
    // zero or negative values produce a RuntimeError.
    // @param x A positive number.
    // @returns The base-2 logarithm of x as a Float.
    // @see_also log, log10, exp2
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example log2(1) => 0.0 ~ "Log base 2 of one"
    // @example log2(8) => 3.0 ~ "Log base 2 of eight"
    // @error TypeError ~ "log2() requires a number" fix: "Pass an Int or Float argument"
    // @error RuntimeError ~ "log2() requires positive number" fix: "Ensure the argument is greater than zero"
    module.insert(
        "log2".to_string(),
        Value::NativeFunction {
            name: "log2".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "log2() requires a number".to_string(),
                        ))
                    }
                };
                if x <= 0.0 {
                    return Err(IntentError::runtime_error(
                        "log2() requires positive number".to_string(),
                    ));
                }
                Ok(Value::Float(x.log2()))
            },
        },
    );

    // @ntnt exp
    // @module std/math
    // @signature exp(x: Number) -> Float
    // Compute e raised to the power of x.
    //
    // Returns e^x where e is Euler's number (~2.71828).
    // @param x The exponent.
    // @returns e^x as a Float.
    // @see_also exp2, log, E
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example exp(0) => 1.0 ~ "e to the zero"
    // @example exp(1) => 2.718281828459045 ~ "e to the one"
    // @error TypeError ~ "exp() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "exp".to_string(),
        Value::NativeFunction {
            name: "exp".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "exp() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.exp()))
            },
        },
    );

    // @ntnt exp2
    // @module std/math
    // @signature exp2(x: Number) -> Float
    // Compute 2 raised to the power of x.
    //
    // Returns 2^x.
    // @param x The exponent.
    // @returns 2^x as a Float.
    // @see_also exp, log2
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example exp2(0) => 1.0 ~ "2 to the zero"
    // @example exp2(3) => 8.0 ~ "2 to the three"
    // @example exp2(10) => 1024.0 ~ "2 to the ten"
    // @error TypeError ~ "exp2() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "exp2".to_string(),
        Value::NativeFunction {
            name: "exp2".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "exp2() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.exp2()))
            },
        },
    );

    // @ntnt cbrt
    // @module std/math
    // @signature cbrt(x: Number) -> Float
    // Compute the cube root of a value.
    //
    // Returns the cube root of x. Works for negative numbers as well.
    // @param x The value to take the cube root of.
    // @returns The cube root of x as a Float.
    // @see_also hypot, exp, log
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example cbrt(27) => 3.0 ~ "Cube root of 27"
    // @example cbrt(8) => 2.0 ~ "Cube root of 8"
    // @example cbrt(-8) => -2.0 ~ "Cube root of negative 8"
    // @error TypeError ~ "cbrt() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "cbrt".to_string(),
        Value::NativeFunction {
            name: "cbrt".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "cbrt() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.cbrt()))
            },
        },
    );

    // @ntnt hypot
    // @module std/math
    // @signature hypot(x: Number, y: Number) -> Float
    // Compute the Euclidean distance sqrt(x^2 + y^2).
    //
    // Returns the length of the hypotenuse of a right triangle with
    // legs x and y, computed in a numerically stable way.
    // @param x The first leg.
    // @param y The second leg.
    // @returns sqrt(x^2 + y^2) as a Float.
    // @see_also cbrt, atan2
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example hypot(3, 4) => 5.0 ~ "Classic 3-4-5 triangle"
    // @example hypot(5, 12) => 13.0 ~ "5-12-13 triangle"
    // @error TypeError ~ "hypot() requires numbers" fix: "Pass Int or Float arguments"
    module.insert(
        "hypot".to_string(),
        Value::NativeFunction {
            name: "hypot".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "hypot() requires numbers".to_string(),
                        ))
                    }
                };
                let y = match &args[1] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "hypot() requires numbers".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.hypot(y)))
            },
        },
    );

    // @ntnt degrees
    // @module std/math
    // @signature degrees(x: Number) -> Float
    // Convert an angle from radians to degrees.
    //
    // Multiplies x by 180/PI to convert radians to degrees.
    // @param x The angle in radians.
    // @returns The angle in degrees as a Float.
    // @see_also radians, sin, cos, tan, PI
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example degrees(PI) => 180.0 ~ "Pi radians is 180 degrees"
    // @example degrees(0) => 0.0 ~ "Zero radians is zero degrees"
    // @error TypeError ~ "degrees() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "degrees".to_string(),
        Value::NativeFunction {
            name: "degrees".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "degrees() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.to_degrees()))
            },
        },
    );

    // @ntnt radians
    // @module std/math
    // @signature radians(x: Number) -> Float
    // Convert an angle from degrees to radians.
    //
    // Multiplies x by PI/180 to convert degrees to radians.
    // @param x The angle in degrees.
    // @returns The angle in radians as a Float.
    // @see_also degrees, sin, cos, tan, PI
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example radians(180) => 3.141592653589793 ~ "180 degrees is pi radians"
    // @example radians(0) => 0.0 ~ "Zero degrees is zero radians"
    // @error TypeError ~ "radians() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "radians".to_string(),
        Value::NativeFunction {
            name: "radians".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let x = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "radians() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Float(x.to_radians()))
            },
        },
    );

    // @ntnt random
    // @module std/math
    // @signature random() -> Float
    // Generate a random floating-point number in [0, 1).
    //
    // Returns a uniformly distributed random Float greater than or
    // equal to 0.0 and strictly less than 1.0.
    // @returns A random Float in [0, 1).
    // @see_also random_int, random_range
    // @since v0.1.0
    // @tags #random
    // @example random() => 0.42 ~ "Returns a random float (value varies)"
    module.insert(
        "random".to_string(),
        Value::NativeFunction {
            name: "random".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| {
                let mut rng = rand::thread_rng();
                Ok(Value::Float(rng.gen::<f64>()))
            },
        },
    );

    // @ntnt random_int
    // @module std/math
    // @signature random_int(min: Int, max: Int) -> Int
    // Generate a random integer in the inclusive range [min, max].
    //
    // Returns a uniformly distributed random integer between min and
    // max, inclusive on both ends. min must be less than or equal to max.
    // @param min The lower bound (inclusive).
    // @param max The upper bound (inclusive).
    // @returns A random Int in [min, max].
    // @see_also random, random_range
    // @since v0.1.0
    // @tags #random
    // @example random_int(1, 6) => 3 ~ "Simulates a dice roll (value varies)"
    // @error TypeError ~ "random_int() requires integers" fix: "Pass Int arguments, not Float"
    // @error RuntimeError ~ "random_int() min must be <= max" fix: "Ensure the first argument is less than or equal to the second"
    module.insert(
        "random_int".to_string(),
        Value::NativeFunction {
            name: "random_int".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                let min = match &args[0] {
                    Value::Int(i) => *i,
                    _ => {
                        return Err(IntentError::type_error(
                            "random_int() requires integers".to_string(),
                        ))
                    }
                };
                let max = match &args[1] {
                    Value::Int(i) => *i,
                    _ => {
                        return Err(IntentError::type_error(
                            "random_int() requires integers".to_string(),
                        ))
                    }
                };
                if min > max {
                    return Err(IntentError::runtime_error(
                        "random_int() min must be <= max".to_string(),
                    ));
                }
                let mut rng = rand::thread_rng();
                Ok(Value::Int(rng.gen_range(min..=max)))
            },
        },
    );

    // @ntnt random_range
    // @module std/math
    // @signature random_range(min: Number, max: Number) -> Float
    // Generate a random float in the half-open range [min, max).
    //
    // Returns a uniformly distributed random Float greater than or
    // equal to min and strictly less than max. min must be less than
    // or equal to max.
    // @param min The lower bound (inclusive).
    // @param max The upper bound (exclusive).
    // @returns A random Float in [min, max).
    // @see_also random, random_int
    // @since v0.1.0
    // @tags #random
    // @example random_range(0.0, 10.0) => 4.2 ~ "Random float between 0 and 10 (value varies)"
    // @error TypeError ~ "random_range() requires numbers" fix: "Pass Int or Float arguments"
    // @error RuntimeError ~ "random_range() min must be <= max" fix: "Ensure the first argument is less than or equal to the second"
    module.insert(
        "random_range".to_string(),
        Value::NativeFunction {
            name: "random_range".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                let min = match &args[0] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "random_range() requires numbers".to_string(),
                        ))
                    }
                };
                let max = match &args[1] {
                    Value::Float(f) => *f,
                    Value::Int(i) => *i as f64,
                    _ => {
                        return Err(IntentError::type_error(
                            "random_range() requires numbers".to_string(),
                        ))
                    }
                };
                if min > max {
                    return Err(IntentError::runtime_error(
                        "random_range() min must be <= max".to_string(),
                    ));
                }
                let mut rng = rand::thread_rng();
                Ok(Value::Float(rng.gen_range(min..max)))
            },
        },
    );

    // @ntnt is_nan
    // @module std/math
    // @signature is_nan(x: Number) -> Bool
    // Check whether a numeric value is NaN (Not a Number).
    //
    // Returns true if x is the special NaN floating-point value,
    // false otherwise. Integers always return false.
    // @param x The number to check.
    // @returns true if x is NaN, false otherwise.
    // @see_also is_infinite, is_finite
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example is_nan(0.0) => false ~ "Zero is not NaN"
    // @example is_nan(1) => false ~ "Integers are never NaN"
    // @error TypeError ~ "is_nan() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "is_nan".to_string(),
        Value::NativeFunction {
            name: "is_nan".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let result = match &args[0] {
                    Value::Float(f) => f.is_nan(),
                    Value::Int(_) => false,
                    _ => {
                        return Err(IntentError::type_error(
                            "is_nan() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Bool(result))
            },
        },
    );

    // @ntnt is_infinite
    // @module std/math
    // @signature is_infinite(x: Number) -> Bool
    // Check whether a numeric value is positive or negative infinity.
    //
    // Returns true if x is positive or negative infinity,
    // false otherwise. Integers always return false.
    // @param x The number to check.
    // @returns true if x is infinite, false otherwise.
    // @see_also is_nan, is_finite, INFINITY, NEG_INFINITY
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example is_infinite(0.0) => false ~ "Zero is not infinite"
    // @example is_infinite(1) => false ~ "Integers are never infinite"
    // @error TypeError ~ "is_infinite() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "is_infinite".to_string(),
        Value::NativeFunction {
            name: "is_infinite".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let result = match &args[0] {
                    Value::Float(f) => f.is_infinite(),
                    Value::Int(_) => false,
                    _ => {
                        return Err(IntentError::type_error(
                            "is_infinite() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Bool(result))
            },
        },
    );

    // @ntnt is_finite
    // @module std/math
    // @signature is_finite(x: Number) -> Bool
    // Check whether a numeric value is finite (not NaN and not infinite).
    //
    // Returns true if x is a finite number (neither NaN nor infinity),
    // false otherwise. Integers always return true.
    // @param x The number to check.
    // @returns true if x is finite, false otherwise.
    // @see_also is_nan, is_infinite
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example is_finite(42) => true ~ "Integers are always finite"
    // @example is_finite(3.14) => true ~ "Normal floats are finite"
    // @error TypeError ~ "is_finite() requires a number" fix: "Pass an Int or Float argument"
    module.insert(
        "is_finite".to_string(),
        Value::NativeFunction {
            name: "is_finite".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let result = match &args[0] {
                    Value::Float(f) => f.is_finite(),
                    Value::Int(_) => true,
                    _ => {
                        return Err(IntentError::type_error(
                            "is_finite() requires a number".to_string(),
                        ))
                    }
                };
                Ok(Value::Bool(result))
            },
        },
    );

    module
}
