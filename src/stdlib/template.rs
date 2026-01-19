//! std/template module - External template loading and rendering
//!
//! # API
//!
//! - `template(path, data)` - Load and render a template file
//! - `compile(path)` - Pre-compile a template for reuse
//! - `render(compiled, data)` - Render a pre-compiled template
//!
//! Note: The actual rendering is handled by builtins in the interpreter
//! since they need access to eval_expression. This module just provides
//! helper functions for file loading.

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

type Result<T> = std::result::Result<T, IntentError>;

// Global template cache
lazy_static::lazy_static! {
    static ref TEMPLATE_CACHE: Mutex<HashMap<u64, CompiledTemplate>> = Mutex::new(HashMap::new());
    static ref TEMPLATE_COUNTER: Mutex<u64> = Mutex::new(0);
}

/// A compiled template that can be rendered multiple times
#[derive(Clone)]
pub struct CompiledTemplate {
    pub id: u64,
    pub path: String,
    pub content: String,
}

/// Get the next template ID
pub fn get_next_template_id() -> u64 {
    let mut counter = TEMPLATE_COUNTER.lock().unwrap();
    *counter += 1;
    *counter
}

/// Load a template file and return its content
pub fn load_template_file(path: &str, base_path: Option<&str>) -> Result<String> {
    // Resolve path relative to base_path if provided
    let full_path = if let Some(base) = base_path {
        let base_dir = Path::new(base).parent().unwrap_or(Path::new("."));
        base_dir.join(path)
    } else {
        Path::new(path).to_path_buf()
    };

    fs::read_to_string(&full_path).map_err(|e| {
        IntentError::RuntimeError(format!(
            "Failed to load template '{}': {}",
            full_path.display(),
            e
        ))
    })
}

/// Store a compiled template in the cache
pub fn store_compiled_template(id: u64, template: CompiledTemplate) {
    let mut cache = TEMPLATE_CACHE.lock().unwrap();
    cache.insert(id, template);
}

/// Get a compiled template from the cache
pub fn get_compiled_template(id: u64) -> Option<CompiledTemplate> {
    let cache = TEMPLATE_CACHE.lock().unwrap();
    cache.get(&id).cloned()
}

/// Initialize the template module exports
/// Note: Most functions are implemented as builtins in the interpreter
pub fn init() -> HashMap<String, Value> {
    let mut exports = HashMap::new();

    // template, compile, and render are implemented as interpreter builtins
    // because they need access to eval_expression

    // Export placeholder values that explain how to use the module
    exports.insert(
        "_module_info".to_string(),
        Value::String("Template functions (template, compile, render) are builtins".to_string()),
    );

    exports
}
