//! Shared native function execution for Intent adapters.

use crate::ast::{Expression, Statement, UnaryOp};
use crate::interpreter::Value;
use crate::lexer::Lexer;
use crate::parser::Parser;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeRequest {
    pub source: PathBuf,
    pub function: String,
    pub args: Vec<NativeValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeAssertion {
    pub source: String,
    pub line: usize,
    pub passed: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeResponse {
    pub value: Option<NativeValue>,
    pub assertions: Vec<NativeAssertion>,
    pub error: Option<String>,
}

pub fn execute(request: &NativeRequest) -> NativeResponse {
    let mut interpreter = crate::interpreter::Interpreter::new();
    interpreter.set_current_file(&request.source.to_string_lossy());
    interpreter.configure_native_test(&request.function);
    let result = (|| -> Result<NativeValue, String> {
        let source = std::fs::read_to_string(&request.source).map_err(|e| e.to_string())?;
        let program = Parser::new(Lexer::new(&source).collect())
            .parse()
            .map_err(|e| e.to_string())?;
        interpreter.eval(&program).map_err(|e| e.to_string())?;
        if let Some(error) = interpreter.native_test_error() {
            return Err(error);
        }
        interpreter.begin_native_assertions();
        let value = interpreter
            .call_function_by_name(
                &request.function,
                request.args.iter().map(NativeValue::to_runtime).collect(),
            )
            .map_err(|e| e.to_string())?;
        if let Some(error) = interpreter.native_test_error() {
            return Err(error);
        }
        NativeValue::from_runtime(&value)
    })();
    let assertions = interpreter.finish_native_assertions();
    match result {
        Ok(value) => NativeResponse {
            value: Some(value),
            assertions,
            error: None,
        },
        Err(error) => NativeResponse {
            value: None,
            assertions,
            error: Some(format!("{}: {error}", request.source.display())),
        },
    }
}

const MAX_REPORT_BYTES: usize = 1024 * 1024;

fn read_transport<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take((MAX_REPORT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_REPORT_BYTES {
        return Err("Native transport exceeds 1 MiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("Invalid native transport: {e}"))
}

fn write_transport<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    struct BoundedBytes(Vec<u8>);
    impl std::io::Write for BoundedBytes {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_REPORT_BYTES.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other("Native transport exceeds 1 MiB"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut bytes = BoundedBytes(Vec::new());
    serde_json::to_writer(&mut bytes, value).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes.0).map_err(|e| format!("{}: {e}", path.display()))
}

/// Internal child entry point; stdout is never interpreted as a report.
pub fn run_child(request_path: &Path, response_path: &Path) -> Result<(), String> {
    crate::config::set_default_type_mode(crate::config::TypeMode::Strict);
    let request: NativeRequest = read_transport(request_path)?;
    for value in &request.args {
        NativeValue::from_runtime(&value.to_runtime())?;
    }
    write_transport(response_path, &execute(&request))
}

/// Execute local trusted test code in a clean process and parent-owned cwd.
/// Explicit absolute file/network access remains possible: this is not a sandbox.
pub fn execute_isolated(
    executable: &Path,
    request: &NativeRequest,
) -> Result<NativeResponse, String> {
    let executable = executable
        .canonicalize()
        .map_err(|e| format!("Native executable: {e}"))?;
    let mut request = request.clone();
    request.source = request
        .source
        .canonicalize()
        .map_err(|e| format!("{}: {e}", request.source.display()))?;
    let root = tempfile::Builder::new()
        .prefix("ntnt-native-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let request_path = root.path().join("request.json");
        let response_path = root.path().join("response.json");
        write_transport(&request_path, &request)?;
        crate::stdlib::process::run_native_test_child(
            &executable,
            root.path(),
            &request_path,
            &response_path,
        )?;
        read_transport(&response_path)
    })();
    match (result, root.close()) {
        (result, Ok(())) => result,
        (Ok(_), Err(error)) => Err(format!("Native fixture cleanup failed: {error}")),
        (Err(error), Err(cleanup)) => {
            Err(format!("{error}; native fixture cleanup failed: {cleanup}"))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NativeValue {
    Unit,
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Array(Vec<NativeValue>),
    Map(BTreeMap<String, NativeValue>),
    Enum {
        enum_name: String,
        variant: String,
        values: Vec<NativeValue>,
    },
}

impl NativeValue {
    pub fn from_runtime(value: &Value) -> Result<Self, String> {
        let mut budget = MAX_REPORT_BYTES;
        Self::from_runtime_bounded(value, 0, &mut budget)
    }

    fn from_runtime_bounded(
        value: &Value,
        depth: usize,
        budget: &mut usize,
    ) -> Result<Self, String> {
        if depth > 32 {
            return Err("Native value nesting exceeds 32".into());
        }
        let size = match value {
            Value::String(s) => s.len(),
            Value::Map(map) => map
                .keys()
                .try_fold(1usize, |size, k| size.checked_add(k.len()))
                .ok_or("Native value too large")?,
            _ => 1,
        };
        *budget = budget
            .checked_sub(size)
            .ok_or("Native value exceeds 1 MiB")?;
        Ok(match value {
            Value::Unit => Self::Unit,
            Value::Int(v) => Self::Int(*v),
            Value::Float(v) if v.is_finite() => Self::Float(*v),
            Value::Bool(v) => Self::Bool(*v),
            Value::String(v) => Self::String(v.clone()),
            Value::Array(v) => Self::Array(v.iter().map(|v| Self::from_runtime_bounded(v, depth + 1, budget)).collect::<Result<_, _>>()?),
            Value::Map(v) => Self::Map(v.iter().map(|(k, v)| Ok((k.clone(), Self::from_runtime_bounded(v, depth + 1, budget)?))).collect::<Result<_, String>>()?),
            Value::EnumValue { enum_name, variant, values } if valid_enum(enum_name, variant, values.len()) => Self::Enum {
                enum_name: enum_name.clone(), variant: variant.clone(), values: values.iter().map(|v| Self::from_runtime_bounded(v, depth + 1, budget)).collect::<Result<_, _>>()?,
            },
            _ => return Err("Unsupported native value (only finite literals and Option/Result are transferable)".into()),
        })
    }

    pub fn to_runtime(&self) -> Value {
        match self {
            Self::Unit => Value::Unit,
            Self::Int(v) => Value::Int(*v),
            Self::Float(v) => Value::Float(*v),
            Self::Bool(v) => Value::Bool(*v),
            Self::String(v) => Value::String(v.clone()),
            Self::Array(v) => Value::Array(v.iter().map(Self::to_runtime).collect()),
            Self::Map(v) => {
                Value::Map(v.iter().map(|(k, v)| (k.clone(), v.to_runtime())).collect())
            }
            Self::Enum {
                enum_name,
                variant,
                values,
            } => Value::EnumValue {
                enum_name: enum_name.clone(),
                variant: variant.clone(),
                values: values.iter().map(Self::to_runtime).collect(),
            },
        }
    }
}

fn valid_enum(name: &str, variant: &str, len: usize) -> bool {
    matches!(
        (name, variant, len),
        ("Option", "None", 0) | ("Option", "Some", 1) | ("Result", "Ok" | "Err", 1)
    )
}

fn literal_expression(expr: &Expression) -> Result<NativeValue, String> {
    Ok(match expr {
        Expression::Integer(v) => NativeValue::Int(*v),
        Expression::Float(v) if v.is_finite() => NativeValue::Float(*v),
        Expression::String(v) => NativeValue::String(v.clone()),
        Expression::Bool(v) => NativeValue::Bool(*v),
        Expression::Array(v) => {
            NativeValue::Array(v.iter().map(literal_expression).collect::<Result<_, _>>()?)
        }
        Expression::MapLiteral(v) => NativeValue::Map(
            v.iter()
                .map(|(k, v)| {
                    let Expression::String(k) = k else {
                        return Err("Native map keys must be literal strings".into());
                    };
                    Ok((k.clone(), literal_expression(v)?))
                })
                .collect::<Result<_, String>>()?,
        ),
        Expression::Unary {
            operator: UnaryOp::Neg,
            operand,
        } => match operand.as_ref() {
            Expression::Integer(v) => NativeValue::Int(v.checked_neg().ok_or("Integer overflow")?),
            Expression::Float(v) if v.is_finite() => NativeValue::Float(-v),
            _ => return Err("Only literal numbers may be negated".into()),
        },
        Expression::Identifier(v) if v == "None" => NativeValue::from_runtime(&Value::none())?,
        Expression::Call {
            function,
            arguments,
        } => {
            let Expression::Identifier(name) = function.as_ref() else {
                return Err("Not a native literal constructor".into());
            };
            let enum_name = match name.as_str() {
                "Some" => "Option",
                "Ok" | "Err" => "Result",
                _ => return Err("Only Some/Ok/Err literal constructors are allowed".into()),
            };
            if !valid_enum(enum_name, name, arguments.len()) {
                return Err("Invalid literal constructor arity".into());
            }
            NativeValue::Enum {
                enum_name: enum_name.into(),
                variant: name.clone(),
                values: arguments
                    .iter()
                    .map(literal_expression)
                    .collect::<Result<_, _>>()?,
            }
        }
        Expression::EnumVariant {
            enum_name,
            variant,
            arguments,
        } if valid_enum(enum_name, variant, arguments.len()) => NativeValue::Enum {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
            values: arguments
                .iter()
                .map(literal_expression)
                .collect::<Result<_, _>>()?,
        },
        _ => return Err("Expected a native literal, not an executable expression".into()),
    })
}

/// Parse data through the native AST, never through evaluation.
pub fn parse_literal(text: &str) -> Result<NativeValue, String> {
    let text = text.trim();
    if text.len() > 65536 {
        return Err("Native literal exceeds 64 KiB".into());
    }
    // Retain the legacy unquoted prose form, but never reinterpret code as prose.
    if !text.is_empty()
        && text.chars().next().is_some_and(char::is_alphabetic)
        && text
            .chars()
            .all(|c| c.is_alphanumeric() || c.is_whitespace() || c == '_')
        && !matches!(
            text.split_whitespace().next().unwrap_or(""),
            "true"
                | "false"
                | "None"
                | "Some"
                | "Ok"
                | "Err"
                | "map"
                | "let"
                | "fn"
                | "return"
                | "if"
                | "while"
                | "for"
                | "import"
        )
    {
        return Ok(NativeValue::String(text.into()));
    }
    let tokens = Lexer::new(text).collect::<Vec<_>>();
    // The language lexer tolerates unfinished strings; transport must not.
    let mut quote = None;
    let mut escaped = false;
    for ch in text.chars() {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == delimiter {
                quote = None;
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if !(ch.is_alphanumeric() || ch.is_whitespace() || "_[]{}():,.-".contains(ch)) {
            return Err("Invalid character in native literal".into());
        }
    }
    if quote.is_some() {
        return Err("Unterminated native string literal".into());
    }
    let mut depth = 0usize;
    let mut previous_minus = false;
    for token in &tokens {
        use crate::lexer::TokenKind;
        let minus = token.lexeme == "-";
        if minus && previous_minus {
            return Err("Repeated native unary operators are not literals".into());
        }
        previous_minus = minus;
        match &token.kind {
            TokenKind::Integer(_) => {
                let digits = token.lexeme.replace('_', "");
                let parsed = if let Some(digits) = digits.strip_prefix("0x") {
                    i64::from_str_radix(digits, 16)
                } else if let Some(digits) = digits.strip_prefix("0b") {
                    i64::from_str_radix(digits, 2)
                } else if let Some(digits) = digits.strip_prefix("0o") {
                    i64::from_str_radix(digits, 8)
                } else {
                    digits.parse::<i64>()
                };
                parsed.map_err(|_| "Native integer literal is out of range")?;
            }
            TokenKind::LeftParen | TokenKind::LeftBracket | TokenKind::LeftBrace => {
                depth += 1;
                if depth > 32 {
                    return Err("Native literal nesting exceeds 32".into());
                }
            }
            TokenKind::RightParen | TokenKind::RightBracket | TokenKind::RightBrace => {
                depth = depth.checked_sub(1).ok_or("Unbalanced native literal")?;
            }
            TokenKind::String(_) => {
                let quote = token
                    .lexeme
                    .chars()
                    .next()
                    .ok_or("Invalid string literal")?;
                if token.lexeme.len() < 2 || !token.lexeme.ends_with(quote) {
                    return Err("Unterminated native string literal".into());
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err("Unbalanced native literal".into());
    }
    let program = Parser::new(tokens).parse().map_err(|e| e.to_string())?;
    if program.statements.len() != 1 {
        return Err("Expected exactly one native literal".into());
    }
    let mut stmt = &program.statements[0];
    while let Statement::Located { stmt: inner, .. } = stmt {
        stmt = inner;
    }
    let Statement::Expression(expr) = stmt else {
        return Err("Expected a native literal".into());
    };
    literal_expression(expr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_test_literals_reject_code_malformed_and_unbounded_data() {
        for text in [
            "1 + 2",
            "print(1)",
            "[1,",
            "map {",
            "Some()",
            "Ok(1, 2)",
            "None garbage",
            "true false",
            "let x",
            "\"#{danger()}\"",
            "<unresolved>",
            "{name}",
            "\"unterminated",
            "1; 2",
        ] {
            assert!(parse_literal(text).is_err(), "accepted {text}");
        }
        assert!(parse_literal(&format!("\"{}\"", "x".repeat(65537))).is_err());
        assert!(parse_literal(&format!("{}0{}", "[".repeat(40), "]".repeat(40))).is_err());
        assert_eq!(
            parse_literal("''").unwrap(),
            NativeValue::String(String::new())
        );
        assert_eq!(parse_literal("-2").unwrap(), NativeValue::Int(-2));
        assert_eq!(
            parse_literal("\"<unresolved>\"").unwrap(),
            NativeValue::String("<unresolved>".into())
        );
        for value in [
            Value::ProcessHandle(7),
            Value::Float(f64::NAN),
            Value::EnumValue {
                enum_name: "Custom".into(),
                variant: "Some".into(),
                values: vec![Value::Int(1)],
            },
        ] {
            assert!(NativeValue::from_runtime(&value).is_err());
        }
    }

    #[test]
    fn native_test_executes_selected_function_and_observes_real_assert() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("checks.tnt");
        std::fs::write(
            &source,
            "assert(true)\nfn check(x) {\n assert(x == 7)\n return x\n}\n",
        )
        .unwrap();
        let response = execute(&NativeRequest {
            source: source.clone(),
            function: "check".into(),
            args: vec![NativeValue::Int(7)],
        });
        assert_eq!(response.error, None);
        assert_eq!(response.value, Some(NativeValue::Int(7)));
        assert_eq!(response.assertions.len(), 1);
        assert!(response.assertions[0].passed);
        assert_eq!(response.assertions[0].line, 3);
        assert_eq!(response.assertions[0].source, source.to_string_lossy());
    }

    #[test]
    fn native_test_caught_failures_missing_entry_and_shadowing() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("checks.tnt");
        std::fs::write(
            &source,
            "fn check() {\n let x = assert(false) otherwise { return 7 }\n return 8\n}\n",
        )
        .unwrap();
        let mut request = NativeRequest {
            source: source.clone(),
            function: "check".into(),
            args: vec![],
        };
        let response = execute(&request);
        assert_eq!(response.value, Some(NativeValue::Int(7)), "{:?}", response);
        assert_eq!(response.assertions.len(), 1);
        assert!(!response.assertions[0].passed);
        request.function = "missing".into();
        assert!(execute(&request).error.unwrap().contains("missing"));
        request.function = "check".into();
        std::fs::write(
            &source,
            "fn assert(x) { true }\nfn check() { assert(false) }",
        )
        .unwrap();
        assert!(execute(&request).assertions.is_empty());
    }

    #[test]
    fn native_test_rejects_autorun_and_unsupported_server_actions() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("checks.tnt");
        let request = NativeRequest {
            source: source.clone(),
            function: "check".into(),
            args: vec![],
        };
        for code in [
            "fn check() { assert(true) }\ncheck()",
            "fn check() { assert(true) }\nlet x = check() otherwise { return 0 }",
            "get(\"/\", fn(req) { \"ok\" })\nfn check() { assert(true) }",
            "fn check() { listen(12345) }",
            "fn check() { let x = listen(12345) otherwise { return 0 } }",
        ] {
            std::fs::write(&source, code).unwrap();
            let response = execute(&request);
            assert!(response.error.is_some(), "accepted {code}: {response:?}");
        }
    }

    #[test]
    fn native_test_imported_assertions_have_helper_source() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("checks.tnt");
        let helper = dir.path().join("helper.tnt");
        std::fs::write(&helper, "fn helper() {\n assert(false)\n}\n").unwrap();
        std::fs::write(
            &source,
            "import { helper } from \"./helper.tnt\"\nfn check() { helper() }",
        )
        .unwrap();
        let response = execute(&NativeRequest {
            source,
            function: "check".into(),
            args: vec![],
        });
        assert_eq!(response.assertions.len(), 1, "{response:?}");
        assert_eq!(response.assertions[0].source, helper.to_string_lossy());
        assert_eq!(response.assertions[0].line, 2);
        assert!(response.error.unwrap().contains("helper.tnt"));
    }

    #[test]
    fn native_test_child_uses_separate_bounded_report() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("checks.tnt");
        let input = dir.path().join("request.json");
        let output = dir.path().join("response.json");
        std::fs::write(
            &source,
            "fn check() { print(\"not JSON\")\n assert(true)\n return Some(7) }",
        )
        .unwrap();
        let request = NativeRequest {
            source,
            function: "check".into(),
            args: vec![],
        };
        std::fs::write(&input, serde_json::to_vec(&request).unwrap()).unwrap();
        run_child(&input, &output).unwrap();
        let response: NativeResponse =
            serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(response.value, Some(parse_literal("Some(7)").unwrap()));
        assert_eq!(response.assertions.len(), 1);
        assert_eq!(response.error, None);
        std::fs::write(&input, vec![b' '; 1024 * 1024 + 1]).unwrap();
        assert!(run_child(&input, &output).is_err());
    }

    #[test]
    fn native_test_isolated_runner_clears_environment_and_removes_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("checks.tnt");
        std::fs::write(&source, "import { get_env, cwd } from \"std/env\"\nfn check() {\n assert(is_none(get_env(\"HOME\")))\n assert(is_none(get_env(\"NTNT_PROCESS_ALLOW\")))\n print(\"not JSON\")\n return cwd()\n}").unwrap();
        let test_exe = std::env::current_exe().unwrap();
        let executable = test_exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("ntnt{}", std::env::consts::EXE_SUFFIX));
        let response = execute_isolated(
            &executable,
            &NativeRequest {
                source,
                function: "check".into(),
                args: vec![],
            },
        )
        .unwrap();
        assert_eq!(response.error, None, "{response:?}");
        assert_eq!(response.assertions.len(), 2);
        assert!(response.assertions.iter().all(|a| a.passed));
        let Some(NativeValue::String(cwd)) = response.value else {
            panic!("missing cwd")
        };
        assert!(
            !Path::new(&cwd).exists(),
            "fixture root was not removed: {cwd}"
        );
    }

    #[test]
    fn native_test_rejects_overflow_and_excessive_values() {
        assert!(parse_literal("9223372036854775808").is_err());
        assert!(parse_literal("999999999999999999999999999999999999").is_err());
        assert!(parse_literal(&format!("{}1", "-".repeat(1000))).is_err());
        let mut deep = Value::Int(1);
        for _ in 0..40 {
            deep = Value::Array(vec![deep]);
        }
        assert!(NativeValue::from_runtime(&deep).is_err());
        assert!(NativeValue::from_runtime(&Value::String("x".repeat(1024 * 1024 + 1))).is_err());
    }

    #[test]
    fn native_test_selected_closure_autorun_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("checks.tnt");
        std::fs::write(&source, "let check = fn() { assert(true) }\ncheck()").unwrap();
        let response = execute(&NativeRequest {
            source,
            function: "check".into(),
            args: vec![],
        });
        assert!(
            response
                .error
                .as_deref()
                .is_some_and(|e| e.contains("module loading")),
            "{response:?}"
        );
    }

    #[test]
    fn native_test_imported_entry_autorun_is_rejected_before_second_call() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("checks.tnt");
        std::fs::write(
            dir.path().join("helper.tnt"),
            "fn check() { assert(true) }\ncheck()",
        )
        .unwrap();
        std::fs::write(&source, "import { check } from \"./helper.tnt\"").unwrap();
        let response = execute(&NativeRequest {
            source,
            function: "check".into(),
            args: vec![],
        });
        assert!(
            response
                .error
                .as_deref()
                .is_some_and(|e| e.contains("module loading")),
            "{response:?}"
        );
        assert!(response.assertions.is_empty());
    }

    #[test]
    fn native_test_literal_preserves_nested_types() {
        let value =
            parse_literal(r#"map { "items": [1, "1", 1.5, true, None, Some(Ok(2)), Err("no")] }"#)
                .unwrap();
        let NativeValue::Map(map) = &value else {
            panic!("not a map")
        };
        let NativeValue::Array(items) = &map["items"] else {
            panic!("not an array")
        };
        assert_eq!(items[0], NativeValue::Int(1));
        assert_eq!(items[1], NativeValue::String("1".into()));
        assert_eq!(items[2], NativeValue::Float(1.5));
        assert_eq!(items[3], NativeValue::Bool(true));
        assert_eq!(
            NativeValue::from_runtime(&value.to_runtime()).unwrap(),
            value
        );
        assert_eq!(
            parse_literal("legacy words").unwrap(),
            NativeValue::String("legacy words".into())
        );
    }
}
