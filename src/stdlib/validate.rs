//! Schema Validation Module (std/validate)
//!
//! Declarative validation for incoming data — form submissions, API
//! payloads, query params (DD-058 item 1). A schema maps field names to a
//! list of rules; `validate()` returns `Ok(cleaned)` with coerced values or
//! `Err(errors)` with one human-readable message per failing field.
//!
//! Rules compose three kinds of values:
//! - Validator markers/builders from this module (`required`, `email`,
//!   `min_value(13)`, ...)
//! - The global conversion functions `int`, `float`, `bool`, `str` and
//!   `std/string`'s `trim`, recognized by name as coercion rules — no
//!   imports are shadowed
//! - Bare functions (`fn(v) { ... }`) as custom predicates
//!
//! Fields are required by default; `optional` permits absence and
//! `default(value)` supplies one.

use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

const VALIDATOR_STRUCT: &str = "Validator";
const SCHEMA_STRUCT: &str = "Schema";

/// Callback used to invoke NTNT function values (closures) from custom
/// rules. The interpreter supplies a real implementation; the plain native
/// fallback rejects closure rules with a clear error.
pub type ClosureInvoker<'a> = dyn FnMut(&Value, Vec<Value>) -> Result<Value> + 'a;

fn make_validator(rule: &str, args: Vec<Value>) -> Value {
    let mut fields = HashMap::new();
    fields.insert("rule".to_string(), Value::String(rule.to_string()));
    fields.insert("args".to_string(), Value::Array(args));
    Value::Struct {
        name: VALIDATOR_STRUCT.to_string(),
        fields,
    }
}

/// Extract (rule, args) from a Validator struct value
fn as_validator(value: &Value) -> Option<(String, Vec<Value>)> {
    let Value::Struct { name, fields } = value else {
        return None;
    };
    if name != VALIDATOR_STRUCT {
        return None;
    }
    let rule = match fields.get("rule") {
        Some(Value::String(r)) => r.clone(),
        _ => return None,
    };
    let args = match fields.get("args") {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    };
    Some((rule, args))
}

fn email_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^[^@\s]+@[^@\s]+\.[^@\s]+$").unwrap())
}

/// Compiled-pattern cache for matches() rules: a schema validates the same
/// few patterns on every request, and Regex can't be stored inside a Value.
/// Thread-local because validation runs on the interpreter thread.
fn cached_regex(pattern: &str) -> Result<regex::Regex> {
    use std::cell::RefCell;
    thread_local! {
        static PATTERN_CACHE: RefCell<HashMap<String, regex::Regex>> =
            RefCell::new(HashMap::new());
    }
    PATTERN_CACHE.with(|cache| {
        if let Some(re) = cache.borrow().get(pattern) {
            return Ok(re.clone());
        }
        let re = regex::Regex::new(pattern)
            .map_err(|e| IntentError::type_error(format!("matches(): invalid pattern: {}", e)))?;
        cache.borrow_mut().insert(pattern.to_string(), re.clone());
        Ok(re)
    })
}

fn url_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^https?://[^\s/$.?#].[^\s]*$").unwrap())
}

/// Render a value for inclusion in an error message ("Must be one of: ...")
fn display_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn numeric(value: &Value) -> Option<f64> {
    match value {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

fn length_of(value: &Value) -> Option<usize> {
    match value {
        Value::String(s) => Some(s.chars().count()),
        Value::Array(a) => Some(a.len()),
        _ => None,
    }
}

fn is_none_value(value: &Value) -> bool {
    matches!(
        value,
        Value::EnumValue { enum_name, variant, .. }
            if enum_name == "Option" && variant == "None"
    )
}

/// Outcome of applying one rule to a value
enum RuleOutcome {
    /// Rule passed; value possibly transformed (trim, coercions)
    Pass(Value),
    /// Rule failed with a message for the errors map
    Fail(String),
}

/// Apply a single rule to a present value.
fn apply_rule(rule: &Value, value: Value, invoke: &mut ClosureInvoker) -> Result<RuleOutcome> {
    use RuleOutcome::{Fail, Pass};

    // Global conversion/transform functions recognized by name
    if let Value::NativeFunction { name, .. } = rule {
        return Ok(match name.as_str() {
            "int" => match &value {
                Value::Int(_) => Pass(value),
                Value::String(s) => match s.trim().parse::<i64>() {
                    Ok(n) => Pass(Value::Int(n)),
                    Err(_) => Fail("Must be an integer".to_string()),
                },
                _ => Fail("Must be an integer".to_string()),
            },
            "float" => match &value {
                Value::Float(_) => Pass(value),
                Value::Int(i) => Pass(Value::Float(*i as f64)),
                Value::String(s) => match s.trim().parse::<f64>() {
                    Ok(f) => Pass(Value::Float(f)),
                    Err(_) => Fail("Must be a number".to_string()),
                },
                _ => Fail("Must be a number".to_string()),
            },
            "bool" => match &value {
                Value::Bool(_) => Pass(value),
                Value::Int(1) => Pass(Value::Bool(true)),
                Value::Int(0) => Pass(Value::Bool(false)),
                Value::String(s) => match s.trim().to_lowercase().as_str() {
                    "true" | "1" | "yes" | "on" => Pass(Value::Bool(true)),
                    "false" | "0" | "no" | "off" | "" => Pass(Value::Bool(false)),
                    _ => Fail("Must be true or false".to_string()),
                },
                _ => Fail("Must be true or false".to_string()),
            },
            "str" => Pass(Value::String(display_value(&value))),
            "trim" => match &value {
                Value::String(s) => Pass(Value::String(s.trim().to_string())),
                _ => Pass(value),
            },
            // Any other native function acts as a custom predicate
            _ => return custom_outcome(invoke(rule, vec![value.clone()])?, value),
        });
    }

    // Bare NTNT closures are custom predicates
    if matches!(rule, Value::Function { .. }) {
        return custom_outcome(invoke(rule, vec![value.clone()])?, value);
    }

    let Some((rule_name, args)) = as_validator(rule) else {
        return Err(IntentError::type_error(format!(
            "Invalid schema rule: expected a validator, conversion function, \
             or fn(value), got {}",
            value_kind(rule)
        )));
    };

    Ok(match rule_name.as_str() {
        // Presence rules are handled by the field loop; when the value is
        // present, `required` additionally rejects None and empty strings
        "required" => {
            if is_none_value(&value) || matches!(&value, Value::String(s) if s.is_empty()) {
                Fail("Required".to_string())
            } else {
                Pass(value)
            }
        }
        "optional" | "default" => Pass(value),
        "string" => match &value {
            Value::String(_) => Pass(value),
            _ => Fail("Must be a string".to_string()),
        },
        "email" => match &value {
            Value::String(s) if email_regex().is_match(s) => Pass(value),
            _ => Fail("Must be a valid email address".to_string()),
        },
        "url" => match &value {
            Value::String(s) if url_regex().is_match(s) => Pass(value),
            _ => Fail("Must be a valid URL".to_string()),
        },
        "min_value" => {
            let bound = args.first().and_then(numeric).unwrap_or(f64::NEG_INFINITY);
            match numeric(&value) {
                Some(n) if n >= bound => Pass(value),
                Some(_) => Fail(format!(
                    "Must be at least {}",
                    display_value(args.first().unwrap_or(&Value::Unit))
                )),
                None => Fail("Must be a number".to_string()),
            }
        }
        "max_value" => {
            let bound = args.first().and_then(numeric).unwrap_or(f64::INFINITY);
            match numeric(&value) {
                Some(n) if n <= bound => Pass(value),
                Some(_) => Fail(format!(
                    "Must be at most {}",
                    display_value(args.first().unwrap_or(&Value::Unit))
                )),
                None => Fail("Must be a number".to_string()),
            }
        }
        "min_length" => {
            let bound = match args.first() {
                Some(Value::Int(n)) => *n as usize,
                _ => 0,
            };
            match length_of(&value) {
                Some(len) if len >= bound => Pass(value),
                Some(_) => Fail(format!("Must be at least {} characters", bound)),
                None => Fail("Must be a string".to_string()),
            }
        }
        "max_length" => {
            let bound = match args.first() {
                Some(Value::Int(n)) => *n as usize,
                _ => usize::MAX,
            };
            match length_of(&value) {
                Some(len) if len <= bound => Pass(value),
                Some(_) => Fail(format!("Must be at most {} characters", bound)),
                None => Fail("Must be a string".to_string()),
            }
        }
        "one_of" => {
            let options = match args.first() {
                Some(Value::Array(opts)) => opts.clone(),
                _ => Vec::new(),
            };
            if options.iter().any(|opt| values_loosely_equal(opt, &value)) {
                Pass(value)
            } else {
                let rendered: Vec<String> = options.iter().map(display_value).collect();
                Fail(format!("Must be one of: {}", rendered.join(", ")))
            }
        }
        "matches" => {
            let pattern = match args.first() {
                Some(Value::String(p)) => p.clone(),
                _ => {
                    return Err(IntentError::type_error(
                        "matches() requires a pattern string".to_string(),
                    ))
                }
            };
            let re = cached_regex(&pattern)?;
            match &value {
                Value::String(s) if re.is_match(s) => Pass(value),
                Value::String(_) => Fail("Invalid format".to_string()),
                _ => Fail("Must be a string".to_string()),
            }
        }
        other => {
            return Err(IntentError::type_error(format!(
                "Unknown validator '{}'",
                other
            )))
        }
    })
}

/// Interpret a custom predicate's return value
fn custom_outcome(result: Value, original: Value) -> Result<RuleOutcome> {
    use RuleOutcome::{Fail, Pass};
    Ok(match result {
        Value::Bool(true) => Pass(original),
        Value::Bool(false) => Fail("Invalid value".to_string()),
        Value::String(message) => Fail(message),
        Value::EnumValue {
            enum_name,
            variant,
            values,
        } => match (enum_name.as_str(), variant.as_str()) {
            // Ok(v)/Some(v) pass with the (possibly transformed) inner value
            ("Result", "Ok") | ("Option", "Some") => {
                Pass(values.into_iter().next().unwrap_or(original))
            }
            ("Result", "Err") => Fail(display_value(
                values
                    .first()
                    .unwrap_or(&Value::String("Invalid value".to_string())),
            )),
            ("Option", "None") => Fail("Invalid value".to_string()),
            _ => Pass(original),
        },
        other => {
            if other.is_truthy() {
                Pass(original)
            } else {
                Fail("Invalid value".to_string())
            }
        }
    })
}

/// Loose equality for one_of options: Int 5 matches coerced Int 5, and the
/// string form matches pre-coercion values
fn values_loosely_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Int(x), Value::Float(y)) | (Value::Float(y), Value::Int(x)) => *x as f64 == *y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        _ => a.to_string() == b.to_string(),
    }
}

fn value_kind(value: &Value) -> String {
    value.type_name().to_string()
}

/// Normalize a field's rules to a list (a single rule is allowed)
fn field_rules(rules_value: &Value) -> Vec<Value> {
    match rules_value {
        Value::Array(rules) => rules.clone(),
        single => vec![single.clone()],
    }
}

/// Find a validator by rule name in a field's rules
fn find_rule(rules: &[Value], name: &str) -> Option<(usize, Vec<Value>)> {
    rules.iter().enumerate().find_map(|(i, r)| {
        as_validator(r).and_then(|(rule, args)| (rule == name).then_some((i, args)))
    })
}

/// Extract the rules map from a Schema struct or a raw map
fn schema_rules(schema: &Value) -> Result<HashMap<String, Value>> {
    match schema {
        Value::Struct { name, fields } if name == SCHEMA_STRUCT => match fields.get("rules") {
            Some(Value::Map(rules)) => Ok(rules.clone()),
            _ => Err(IntentError::type_error(
                "Invalid schema: missing rules".to_string(),
            )),
        },
        Value::Map(rules) => Ok(rules.clone()),
        other => Err(IntentError::type_error(format!(
            "validate() requires a schema (from schema()) or a map, got {}",
            other.type_name()
        ))),
    }
}

/// Core validation: returns the NTNT-level Result value —
/// `Ok(cleaned_map)` or `Err(field -> message map)`. Rust-level Err is
/// reserved for schema misuse (wrong types), not validation failures.
pub fn run_validation(schema: &Value, data: &Value, invoke: &mut ClosureInvoker) -> Result<Value> {
    let rules_map = schema_rules(schema)?;
    let data_map = match data {
        Value::Map(m) => m.clone(),
        other => {
            return Err(IntentError::type_error(format!(
                "validate() requires a map of input data, got {}",
                other.type_name()
            )))
        }
    };

    let mut cleaned: HashMap<String, Value> = HashMap::new();
    let mut errors: HashMap<String, Value> = HashMap::new();

    // Sort field names so error iteration order is stable
    let mut field_names: Vec<&String> = rules_map.keys().collect();
    field_names.sort();

    for field in field_names {
        let rules = field_rules(&rules_map[field]);

        let present = data_map
            .get(field)
            .map(|v| !is_none_value(v))
            .unwrap_or(false);

        let mut value = if present {
            data_map[field].clone()
        } else if let Some((_, args)) = find_rule(&rules, "default") {
            args.into_iter().next().unwrap_or(Value::Unit)
        } else if find_rule(&rules, "optional").is_some() {
            continue; // absent and optional: omit from output
        } else {
            errors.insert(field.clone(), Value::String("Required".to_string()));
            continue;
        };

        for rule in &rules {
            match apply_rule(rule, value.clone(), invoke)? {
                RuleOutcome::Pass(next) => value = next,
                RuleOutcome::Fail(message) => {
                    errors.insert(field.clone(), Value::String(message));
                    break;
                }
            }
        }

        if !errors.contains_key(field) {
            cleaned.insert(field.clone(), value);
        }
    }

    if errors.is_empty() {
        Ok(Value::ok(Value::Map(cleaned)))
    } else {
        Ok(Value::err(Value::Map(errors)))
    }
}

/// Validate a schema definition's shape (called by schema())
fn check_schema_shape(rules: &HashMap<String, Value>) -> Result<()> {
    for (field, rules_value) in rules {
        for rule in field_rules(rules_value) {
            let ok = matches!(rule, Value::NativeFunction { .. } | Value::Function { .. })
                || as_validator(&rule).is_some();
            if !ok {
                return Err(IntentError::type_error(format!(
                    "schema(): field '{}' has an invalid rule ({}) — rules are \
                     validators, conversion functions, or fn(value)",
                    field,
                    rule.type_name()
                )));
            }
        }
    }
    Ok(())
}

/// Initialize the std/validate module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // Marker validators exported as plain values (no doc blocks needed —
    // they are constants like math.PI; documented on schema/validate)
    module.insert("required".to_string(), make_validator("required", vec![]));
    module.insert("optional".to_string(), make_validator("optional", vec![]));
    module.insert("string".to_string(), make_validator("string", vec![]));
    module.insert("email".to_string(), make_validator("email", vec![]));
    module.insert("url".to_string(), make_validator("url", vec![]));

    // @ntnt schema
    // @module std/validate
    // @module_description Declarative schema validation for forms and API payloads
    // @signature schema(rules: Map<String, Any>) -> Schema
    // Build a validation schema from a map of field names to rule lists.
    //
    // Rules compose validators from this module (`required`, `email`,
    // `min_value(13)`, ...), the global conversion functions (`int`, `float`,
    // `bool`, `str`, and `trim` from std/string) for coercion, and bare
    // functions as custom predicates. Fields are required by default; use
    // `optional` to permit absence or `default(value)` to supply one.
    // @param rules Map of field name to a rule or array of rules.
    // @returns A Schema for use with validate().
    // @see_also validate, one_of, matches, min_value, min_length
    // @since v0.4.12
    // @tags #validation, #forms, #schema
    // @error TypeError ~ "field has an invalid rule" fix: "Rules must be validators, conversion functions, or fn(value)"
    // @example schema(map { "email": [required, email] }) ~ "Schema requiring a valid email"
    module.insert(
        "schema".to_string(),
        Value::NativeFunction {
            name: "schema".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let rules = match &args[0] {
                    Value::Map(m) => m.clone(),
                    other => {
                        return Err(IntentError::type_error(format!(
                            "schema() requires a map of field rules, got {}",
                            other.type_name()
                        )))
                    }
                };
                check_schema_shape(&rules)?;
                let mut fields = HashMap::new();
                fields.insert("rules".to_string(), Value::Map(rules));
                Ok(Value::Struct {
                    name: SCHEMA_STRUCT.to_string(),
                    fields,
                })
            },
        },
    );

    // @ntnt validate
    // @module std/validate
    // @signature validate(schema: Schema, data: Map<String, Any>) -> Result<Map<String, Any>, Map<String, String>>
    // Validate a map of input data against a schema.
    //
    // Returns Ok with a cleaned map (coerced values, only schema fields) when
    // every rule passes, or Err with one human-readable message per failing
    // field. Rules run left to right per field; the first failure wins.
    // Coercion rules accept form-style strings: `int` turns "25" into 25.
    // @param schema A Schema from schema(), or a raw rules map.
    // @param data The input map to validate (e.g. parse_form(req)).
    // @returns Ok(cleaned) or Err(map of field -> error message).
    // @see_also schema, parse_form, parse_json
    // @since v0.4.12
    // @tags #validation, #forms
    // @error TypeError ~ "requires a map of input data" fix: "Pass the parsed form/JSON map, not the raw request"
    // @example validate(schema(map { "age": [required, int] }), map { "age": "25" }) => Ok(map { "age": 25 }) ~ "Coerces string input"
    // @gotcha Fields are required by default — add `optional` or `default(v)` for optional fields.
    module.insert(
        "validate".to_string(),
        Value::NativeFunction {
            name: "validate".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                // Closure rules need interpreter context; the interpreter
                // special-cases validate() calls to provide it. This plain
                // path covers every non-closure schema.
                run_validation(&args[0], &args[1], &mut |_, _| {
                    Err(IntentError::type_error(
                        "Custom fn(value) rules require calling validate() \
                         directly (not through another function value)"
                            .to_string(),
                    ))
                })
            },
        },
    );

    // @ntnt min_value
    // @module std/validate
    // @signature min_value(bound: Int | Float) -> Validator
    // Validator: numeric value must be >= bound.
    //
    // Named min_value (not min) so importing it never shadows the global
    // min() function. Apply after `int`/`float` coercion in the rule list.
    // @param bound The inclusive lower bound.
    // @returns A Validator for use in schema rules.
    // @see_also max_value, min_length, schema
    // @since v0.4.12
    // @tags #validation
    // @example schema(map { "age": [required, int, min_value(13)] }) ~ "Age must be at least 13"
    module.insert(
        "min_value".to_string(),
        Value::NativeFunction {
            name: "min_value".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| Ok(make_validator("min_value", vec![args[0].clone()])),
        },
    );

    // @ntnt max_value
    // @module std/validate
    // @signature max_value(bound: Int | Float) -> Validator
    // Validator: numeric value must be <= bound.
    //
    // Named max_value (not max) so importing it never shadows the global
    // max() function.
    // @param bound The inclusive upper bound.
    // @returns A Validator for use in schema rules.
    // @see_also min_value, max_length, schema
    // @since v0.4.12
    // @tags #validation
    // @example schema(map { "age": [required, int, max_value(120)] }) ~ "Age must be at most 120"
    module.insert(
        "max_value".to_string(),
        Value::NativeFunction {
            name: "max_value".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| Ok(make_validator("max_value", vec![args[0].clone()])),
        },
    );

    // @ntnt min_length
    // @module std/validate
    // @signature min_length(bound: Int) -> Validator
    // Validator: string (or array) length must be >= bound.
    // @param bound The inclusive minimum length in characters/elements.
    // @returns A Validator for use in schema rules.
    // @see_also max_length, min_value, schema
    // @since v0.4.12
    // @tags #validation
    // @example schema(map { "name": [required, min_length(1)] }) ~ "Non-empty name"
    module.insert(
        "min_length".to_string(),
        Value::NativeFunction {
            name: "min_length".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| Ok(make_validator("min_length", vec![args[0].clone()])),
        },
    );

    // @ntnt max_length
    // @module std/validate
    // @signature max_length(bound: Int) -> Validator
    // Validator: string (or array) length must be <= bound.
    // @param bound The inclusive maximum length in characters/elements.
    // @returns A Validator for use in schema rules.
    // @see_also min_length, max_value, schema
    // @since v0.4.12
    // @tags #validation
    // @example schema(map { "name": [required, max_length(100)] }) ~ "Name capped at 100 chars"
    module.insert(
        "max_length".to_string(),
        Value::NativeFunction {
            name: "max_length".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| Ok(make_validator("max_length", vec![args[0].clone()])),
        },
    );

    // @ntnt one_of
    // @module std/validate
    // @signature one_of(options: Array<Any>) -> Validator
    // Validator: value must equal one of the listed options.
    // @param options Allowed values.
    // @returns A Validator for use in schema rules.
    // @see_also matches, schema
    // @since v0.4.12
    // @tags #validation
    // @example schema(map { "role": [one_of(["admin", "user"])] }) ~ "Role allow-list"
    module.insert(
        "one_of".to_string(),
        Value::NativeFunction {
            name: "one_of".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if !matches!(&args[0], Value::Array(_)) {
                    return Err(IntentError::type_error(
                        "one_of() requires an array of allowed values".to_string(),
                    ));
                }
                Ok(make_validator("one_of", vec![args[0].clone()]))
            },
        },
    );

    // @ntnt matches
    // @module std/validate
    // @signature matches(pattern: String) -> Validator
    // Validator: string value must match the regex pattern.
    // @param pattern A regex pattern (anchor with ^...$ for full-string match).
    // @returns A Validator for use in schema rules.
    // @see_also one_of, email, schema
    // @since v0.4.12
    // @tags #validation, #regex
    // @error TypeError ~ "invalid pattern" fix: "Check the regex syntax; remember to escape backslashes in strings"
    // @example schema(map { "phone": [matches("^\\+?[0-9]{10,15}$")] }) ~ "Phone number format"
    module.insert(
        "matches".to_string(),
        Value::NativeFunction {
            name: "matches".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let pattern = match &args[0] {
                    Value::String(p) => p.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "matches() requires a pattern string".to_string(),
                        ))
                    }
                };
                // Fail fast on bad patterns at schema-build time
                regex::Regex::new(&pattern).map_err(|e| {
                    IntentError::type_error(format!("matches(): invalid pattern: {}", e))
                })?;
                Ok(make_validator("matches", vec![Value::String(pattern)]))
            },
        },
    );

    // @ntnt default
    // @module std/validate
    // @signature default(value: Any) -> Validator
    // Validator: supply a default when the field is absent.
    //
    // The default value still runs through the remaining rules (so a string
    // default is coerced by a later `int` rule, for example).
    // @param value The value to use when the field is missing.
    // @returns A Validator for use in schema rules.
    // @see_also schema, validate
    // @since v0.4.12
    // @tags #validation
    // @example schema(map { "page": [default("1"), int, min_value(1)] }) ~ "Pagination default"
    module.insert(
        "default".to_string(),
        Value::NativeFunction {
            name: "default".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| Ok(make_validator("default", vec![args[0].clone()])),
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_string(value: &Value, expected: &str) {
        match value {
            Value::String(s) => assert_eq!(s, expected),
            other => panic!("expected string {:?}, got {:?}", expected, other),
        }
    }

    fn no_closures<'a>() -> Box<ClosureInvoker<'a>> {
        Box::new(|_: &Value, _: Vec<Value>| {
            Err(IntentError::type_error(
                "no closures in this test".to_string(),
            ))
        })
    }

    fn rules_map(entries: Vec<(&str, Value)>) -> Value {
        let mut m = HashMap::new();
        for (k, v) in entries {
            m.insert(k.to_string(), v);
        }
        Value::Map(m)
    }

    fn unwrap_ok_map(result: Value) -> HashMap<String, Value> {
        match result {
            Value::EnumValue {
                enum_name,
                variant,
                values,
            } if enum_name == "Result" && variant == "Ok" => match values.into_iter().next() {
                Some(Value::Map(m)) => m,
                other => panic!("expected Ok(map), got Ok({:?})", other),
            },
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    fn unwrap_err_map(result: Value) -> HashMap<String, Value> {
        match result {
            Value::EnumValue {
                enum_name,
                variant,
                values,
            } if enum_name == "Result" && variant == "Err" => match values.into_iter().next() {
                Some(Value::Map(m)) => m,
                other => panic!("expected Err(map), got Err({:?})", other),
            },
            other => panic!("expected Err, got {:?}", other),
        }
    }

    #[test]
    fn required_field_missing_reports_required() {
        let schema = rules_map(vec![("name", make_validator("required", vec![]))]);
        let data = rules_map(vec![]);
        let result = run_validation(&schema, &data, &mut *no_closures()).unwrap();
        let errors = unwrap_err_map(result);
        assert_string(&errors["name"], "Required");
    }

    #[test]
    fn fields_are_required_by_default() {
        let schema = rules_map(vec![("name", make_validator("string", vec![]))]);
        let data = rules_map(vec![]);
        let errors = unwrap_err_map(run_validation(&schema, &data, &mut *no_closures()).unwrap());
        assert_string(&errors["name"], "Required");
    }

    #[test]
    fn optional_absent_field_is_omitted() {
        let schema = rules_map(vec![(
            "nickname",
            Value::Array(vec![
                make_validator("optional", vec![]),
                make_validator("string", vec![]),
            ]),
        )]);
        let data = rules_map(vec![]);
        let cleaned = unwrap_ok_map(run_validation(&schema, &data, &mut *no_closures()).unwrap());
        assert!(cleaned.is_empty());
    }

    #[test]
    fn default_fills_absent_and_runs_remaining_rules() {
        let schema = rules_map(vec![(
            "page",
            Value::Array(vec![
                make_validator("default", vec![Value::String("1".to_string())]),
                // no coercion here; string default should surface
                make_validator("string", vec![]),
            ]),
        )]);
        let data = rules_map(vec![]);
        let cleaned = unwrap_ok_map(run_validation(&schema, &data, &mut *no_closures()).unwrap());
        assert_string(&cleaned["page"], "1");
    }

    #[test]
    fn required_rejects_empty_string() {
        let schema = rules_map(vec![("name", make_validator("required", vec![]))]);
        let data = rules_map(vec![("name", Value::String(String::new()))]);
        let errors = unwrap_err_map(run_validation(&schema, &data, &mut *no_closures()).unwrap());
        assert_string(&errors["name"], "Required");
    }

    #[test]
    fn min_max_value_bounds() {
        let schema = rules_map(vec![(
            "age",
            Value::Array(vec![
                make_validator("min_value", vec![Value::Int(13)]),
                make_validator("max_value", vec![Value::Int(120)]),
            ]),
        )]);
        let too_young = rules_map(vec![("age", Value::Int(12))]);
        let errors =
            unwrap_err_map(run_validation(&schema, &too_young, &mut *no_closures()).unwrap());
        assert_string(&errors["age"], "Must be at least 13");

        let fine = rules_map(vec![("age", Value::Int(30))]);
        let cleaned = unwrap_ok_map(run_validation(&schema, &fine, &mut *no_closures()).unwrap());
        assert!(matches!(cleaned["age"], Value::Int(30)));
    }

    #[test]
    fn email_and_url_validators() {
        let schema = rules_map(vec![
            ("email", make_validator("email", vec![])),
            ("site", make_validator("url", vec![])),
        ]);
        let good = rules_map(vec![
            ("email", Value::String("a@b.co".to_string())),
            ("site", Value::String("https://example.com/x".to_string())),
        ]);
        unwrap_ok_map(run_validation(&schema, &good, &mut *no_closures()).unwrap());

        let bad = rules_map(vec![
            ("email", Value::String("not-an-email".to_string())),
            ("site", Value::String("ftp://nope".to_string())),
        ]);
        let errors = unwrap_err_map(run_validation(&schema, &bad, &mut *no_closures()).unwrap());
        assert_string(&errors["email"], "Must be a valid email address");
        assert_string(&errors["site"], "Must be a valid URL");
    }

    #[test]
    fn one_of_and_matches() {
        let schema = rules_map(vec![
            (
                "role",
                make_validator(
                    "one_of",
                    vec![Value::Array(vec![
                        Value::String("admin".to_string()),
                        Value::String("user".to_string()),
                    ])],
                ),
            ),
            (
                "code",
                make_validator("matches", vec![Value::String("^[A-Z]{3}$".to_string())]),
            ),
        ]);
        let bad = rules_map(vec![
            ("role", Value::String("root".to_string())),
            ("code", Value::String("abc".to_string())),
        ]);
        let errors = unwrap_err_map(run_validation(&schema, &bad, &mut *no_closures()).unwrap());
        assert_string(&errors["role"], "Must be one of: admin, user");
        assert_string(&errors["code"], "Invalid format");
    }

    #[test]
    fn unknown_keys_are_dropped_from_output() {
        let schema = rules_map(vec![("name", make_validator("string", vec![]))]);
        let data = rules_map(vec![
            ("name", Value::String("Alice".to_string())),
            ("evil", Value::String("payload".to_string())),
        ]);
        let cleaned = unwrap_ok_map(run_validation(&schema, &data, &mut *no_closures()).unwrap());
        assert_eq!(cleaned.len(), 1);
        assert!(cleaned.contains_key("name"));
    }

    #[test]
    fn first_failure_per_field_wins() {
        let schema = rules_map(vec![(
            "name",
            Value::Array(vec![
                make_validator("string", vec![]),
                make_validator("min_length", vec![Value::Int(5)]),
            ]),
        )]);
        let data = rules_map(vec![("name", Value::Int(7))]);
        let errors = unwrap_err_map(run_validation(&schema, &data, &mut *no_closures()).unwrap());
        assert_string(&errors["name"], "Must be a string");
    }
}
