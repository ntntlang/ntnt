//! Global builtins that need interpreter context: template rendering, route
//! registration and higher-order array helpers. They are ordinary
//! `Value::ContextFunction` values, so calls resolve the callee first and user
//! definitions with the same names shadow them like any other binding.
use super::*;

fn array_arg<'a>(name: &str, value: &'a Value) -> Result<&'a Vec<Value>> {
    match value {
        Value::Array(items) => Ok(items),
        _ => Err(IntentError::type_error(format!(
            "{name}() requires an array as first argument"
        ))),
    }
}

pub(crate) fn transform(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    let items = array_arg("transform", &args[0])?;
    let mut result = Vec::with_capacity(items.len());
    for item in items {
        result.push(interp.call_function(args[1].clone(), vec![item.clone()])?);
    }
    Ok(Value::Array(result))
}

pub(crate) fn reduce(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    let items = array_arg("reduce", &args[0])?;
    let mut acc = args[1].clone();
    for item in items {
        acc = interp.call_function(args[2].clone(), vec![acc, item.clone()])?;
    }
    Ok(acc)
}

pub(crate) fn flat_map(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    let items = array_arg("flat_map", &args[0])?;
    let mut result = Vec::new();
    for item in items {
        match interp.call_function(args[1].clone(), vec![item.clone()])? {
            Value::Array(inner) => result.extend(inner),
            other => result.push(other),
        }
    }
    Ok(Value::Array(result))
}

pub(crate) fn sort_desc(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    let items = array_arg("sort_desc", &args[0])?;
    let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(items.len());
    for item in items {
        let key = match args.get(1) {
            None => item.clone(),
            Some(Value::String(field)) => match item {
                Value::Map(m) => m.get(field).cloned().unwrap_or(Value::Unit),
                _ => item.clone(),
            },
            Some(
                func @ (Value::Function { .. }
                | Value::NativeFunction { .. }
                | Value::ContextFunction { .. }),
            ) => interp.call_function(func.clone(), vec![item.clone()])?,
            Some(_) => item.clone(),
        };
        keyed.push((key, item.clone()));
    }
    keyed.sort_by(|(a, _), (b, _)| Interpreter::compare_values(b, a));
    Ok(Value::Array(keyed.into_iter().map(|(_, v)| v).collect()))
}

pub(crate) fn template(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    let Value::String(path) = &args[0] else {
        return Err(IntentError::type_error(
            "template() first argument must be a string path".to_string(),
        ));
    };
    let Value::Map(data) = &args[1] else {
        return Err(IntentError::type_error(
            "template() second argument must be a map".to_string(),
        ));
    };
    let base_path = interp.current_file.clone();
    let parts = interp.get_cached_template_parts(path, base_path.as_deref())?;
    interp.render_template_parts_with_data(parts.as_slice(), data)
}

pub(crate) fn compile(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    let Value::String(path_str) = &args[0] else {
        return Err(IntentError::type_error(
            "compile() argument must be a string path".to_string(),
        ));
    };
    let base_path = interp.current_file.as_deref();
    let content = crate::stdlib::template::load_template_file(path_str, base_path)?;
    let resolved_path = match base_path {
        Some(base) => std::path::Path::new(base)
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(path_str),
        None => std::path::PathBuf::from(path_str),
    };
    let mtime = std::fs::metadata(&resolved_path)
        .ok()
        .and_then(|m| m.modified().ok());
    let id = crate::stdlib::template::get_next_template_id();
    crate::stdlib::template::store_compiled_template(
        id,
        crate::stdlib::template::CompiledTemplate {
            id,
            path: path_str.clone(),
            resolved_path: resolved_path.to_string_lossy().to_string(),
            content,
            mtime,
        },
    );
    let mut result = HashMap::new();
    result.insert("_template_id".to_string(), Value::Int(id as i64));
    result.insert("path".to_string(), Value::String(path_str.clone()));
    Ok(Value::Map(result))
}

pub(crate) fn render(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    let not_compiled = || {
        IntentError::type_error("render() first argument must be a compiled template".to_string())
    };
    let template_id = match &args[0] {
        Value::Map(m) => match m.get("_template_id") {
            Some(Value::Int(id)) => *id as u64,
            _ => return Err(not_compiled()),
        },
        _ => return Err(not_compiled()),
    };
    let Value::Map(data) = &args[1] else {
        return Err(IntentError::type_error(
            "render() second argument must be a map".to_string(),
        ));
    };
    // Keep the compiled template's file identity so imports inside it
    // resolve relative to the template (#297).
    let template = crate::stdlib::template::get_compiled_template(template_id)
        .ok_or_else(|| IntentError::runtime_error("Template not found in cache".to_string()))?;
    interp.render_template_with_data(
        &template.content,
        data,
        std::path::Path::new(&template.resolved_path),
    )
}

fn register_route(interp: &mut Interpreter, method: &str, args: &[Value]) -> Result<Value> {
    // Capability (HttpServer) is enforced by the call gate before this runs.
    match &args[0] {
        Value::String(pattern) if pattern.starts_with('/') => {
            // Optional route options: map { "rate": "5/minute" | "off" } (#294).
            // Validate everything before changing any state.
            let mut rate: Option<String> = None;
            match args.get(2) {
                None | Some(Value::Unit) => {}
                Some(Value::Map(opts)) => {
                    for (key, value) in opts {
                        match (key.as_str(), value) {
                            ("rate", Value::String(r)) => {
                                crate::stdlib::rate_limit::RouteRate::parse(r).map_err(|e| {
                                    IntentError::runtime_error(format!("{method} {pattern}: {e} (or \"off\")"))
                                })?;
                                rate = Some(r.clone());
                            }
                            ("rate", _) => {
                                return Err(IntentError::type_error(
                                    "route option `rate` must be a string like \"5/minute\" or \"off\"".to_string(),
                                ))
                            }
                            (other, _) => {
                                return Err(IntentError::type_error(format!(
                                    "unknown route option `{other}` (supported: rate)"
                                )))
                            }
                        }
                    }
                }
                Some(_) => {
                    return Err(IntentError::type_error(format!(
                        "{}(pattern, handler, options): options must be a map",
                        method.to_lowercase()
                    )))
                }
            }
            if let Some(r) = rate {
                interp.server_state.set_route_rate(method, pattern, &r)?;
            }
            interp.server_state.add_route_with_source(
                method,
                pattern,
                args[1].clone(),
                interp.current_file.clone(),
                interp.imported_files.clone(),
            );
            Ok(Value::Unit)
        }
        _ => Err(IntentError::type_error(format!(
            "{}() route pattern must be a string starting with '/' (for HTTP requests use fetch() from std/http)",
            method.to_lowercase()
        ))),
    }
}

pub(crate) fn route_get(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    register_route(interp, "GET", args)
}
pub(crate) fn route_post(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    register_route(interp, "POST", args)
}
pub(crate) fn route_put(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    register_route(interp, "PUT", args)
}
pub(crate) fn route_delete(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    register_route(interp, "DELETE", args)
}
pub(crate) fn route_patch(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    register_route(interp, "PATCH", args)
}

fn is_callable(value: &Value) -> bool {
    matches!(
        value,
        Value::Function { .. } | Value::NativeFunction { .. } | Value::ContextFunction { .. }
    )
}

pub(crate) fn filter(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    let items = array_arg("filter", &args[0])?;
    let mut result = Vec::new();
    for item in items {
        if interp
            .call_function(args[1].clone(), vec![item.clone()])?
            .is_truthy()
        {
            result.push(item.clone());
        }
    }
    Ok(Value::Array(result))
}

pub(crate) fn find(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    let items = array_arg("find", &args[0])?;
    for item in items {
        if interp
            .call_function(args[1].clone(), vec![item.clone()])?
            .is_truthy()
        {
            return Ok(Value::some(item.clone()));
        }
    }
    Ok(Value::none())
}

pub(crate) fn any(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    let items = array_arg("any", &args[0])?;
    for item in items {
        if interp
            .call_function(args[1].clone(), vec![item.clone()])?
            .is_truthy()
        {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

pub(crate) fn all(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    let items = array_arg("all", &args[0])?;
    for item in items {
        if !interp
            .call_function(args[1].clone(), vec![item.clone()])?
            .is_truthy()
        {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

/// `count(array, predicate)` or `count(string, substring)`.
pub(crate) fn count(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    match (&args[0], &args[1]) {
        (Value::Array(items), predicate) => {
            let mut n = 0i64;
            for item in items {
                if interp
                    .call_function(predicate.clone(), vec![item.clone()])?
                    .is_truthy()
                {
                    n += 1;
                }
            }
            Ok(Value::Int(n))
        }
        (Value::String(s), Value::String(sub)) => {
            Ok(Value::Int(s.matches(sub.as_str()).count() as i64))
        }
        _ => Err(IntentError::type_error(
            "count() requires an array and a predicate, or two strings".to_string(),
        )),
    }
}

type PlainFn = fn(&[Value]) -> Result<Value>;

/// Plain natives wrapped by context functions, captured once from their modules.
fn plain(module: &str, name: &str) -> PlainFn {
    use std::sync::OnceLock;
    static PLAIN: OnceLock<HashMap<String, PlainFn>> = OnceLock::new();
    let table = PLAIN.get_or_init(|| {
        let mut table = HashMap::new();
        for (module, exports) in [
            ("std/fs", crate::stdlib::fs::init()),
            ("std/collections", crate::stdlib::collections::init()),
        ] {
            for (name, value) in exports {
                if let Value::NativeFunction { func, .. } = value {
                    table.insert(format!("{module}:{name}"), func);
                }
            }
        }
        table
    });
    *table
        .get(&format!("{module}:{name}"))
        .unwrap_or_else(|| panic!("missing plain native {module}:{name}"))
}

/// `sort(arr, key?)`: a function key runs in the interpreter; other forms use
/// the plain native implementation.
pub(crate) fn sort(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    match args.get(1) {
        Some(key) if is_callable(key) => {
            if args[0].contains_secret() {
                return Err(IntentError::type_error(
                    "sort() cannot compare Secret values".to_string(),
                ));
            }
            let items = array_arg("sort", &args[0])?;
            let mut keyed = Vec::with_capacity(items.len());
            for item in items {
                keyed.push((
                    interp.call_function(key.clone(), vec![item.clone()])?,
                    item.clone(),
                ));
            }
            keyed.sort_by(|(a, _), (b, _)| Interpreter::compare_values(a, b));
            Ok(Value::Array(keyed.into_iter().map(|(_, v)| v).collect()))
        }
        _ => plain("std/collections", "sort")(args),
    }
}

pub(crate) fn sort_by(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    interp.sort_by_hof(args[0].clone(), args[1].clone())
}

pub(crate) fn validate(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    let mut invoke =
        |func: &Value, call_args: Vec<Value>| interp.call_function(func.clone(), call_args);
    crate::stdlib::validate::run_validation(&args[0], &args[1], &mut invoke)
}

pub(crate) fn magic_link_flow(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
    let mut invoke =
        |func: &Value, call_args: Vec<Value>| interp.call_function(func.clone(), call_args);
    crate::stdlib::auth::run_magic_link_flow(args, &mut invoke)
}

/// Resolve the listed string path arguments relative to the calling script,
/// then call the plain std/fs implementation.
fn fs_call(
    interp: &mut Interpreter,
    name: &str,
    path_args: &[usize],
    args: &[Value],
) -> Result<Value> {
    let mut resolved = args.to_vec();
    for &i in path_args {
        if let Some(Value::String(path)) = args.get(i) {
            resolved[i] = Value::String(interp.resolve_path_relative_to_script(path));
        }
    }
    plain("std/fs", name)(&resolved)
}

macro_rules! fs_wrappers {
    ($($fn_name:ident => $name:literal [$($idx:literal),*]),* $(,)?) => {
        $(fn $fn_name(interp: &mut Interpreter, args: &[Value]) -> Result<Value> {
            fs_call(interp, $name, &[$($idx),*], args)
        })*
        const FS_WRAPPERS: &[(&str, ContextFn)] = &[$(($name, $fn_name)),*];
    };
}

fs_wrappers! {
    fs_read_file => "read_file" [0],
    fs_read_bytes => "read_bytes" [0],
    fs_exists => "exists" [0],
    fs_is_file => "is_file" [0],
    fs_is_dir => "is_dir" [0],
    fs_mkdir => "mkdir" [0],
    fs_mkdir_all => "mkdir_all" [0],
    fs_readdir => "readdir" [0],
    fs_remove => "remove" [0],
    fs_remove_dir => "remove_dir" [0],
    fs_remove_dir_all => "remove_dir_all" [0],
    fs_file_size => "file_size" [0],
    fs_rename => "rename" [0, 1],
    fs_copy => "copy" [0, 1],
    fs_write_file => "write_file" [0],
    fs_append_file => "append_file" [0],
}

/// Replace module exports that need interpreter context with context
/// functions, keeping each native's arity and capability metadata.
pub(crate) fn install_module_overrides(modules: &mut HashMap<String, HashMap<String, Value>>) {
    let mut wrap = |module: &str, name: &str, func: ContextFn| {
        let Some(exports) = modules.get_mut(module) else {
            return;
        };
        if let Some(Value::NativeFunction {
            arity,
            max_arity,
            requires,
            ..
        }) = exports.get(name).cloned()
        {
            exports.insert(
                name.to_string(),
                Value::ContextFunction {
                    name: name.to_string(),
                    arity,
                    max_arity,
                    func,
                    requires,
                },
            );
        }
    };
    for (name, func) in FS_WRAPPERS {
        wrap("std/fs", name, *func);
    }
    wrap("std/collections", "sort", sort);
    wrap("std/collections", "sort_by", sort_by);
    wrap("std/validate", "validate", validate);
    wrap("std/auth", "magic_link_flow", magic_link_flow);
}
