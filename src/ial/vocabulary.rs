//! Vocabulary system for the IAL term rewriting engine.
//!
//! The vocabulary is where ALL knowledge lives - standard assertions, glossary terms,
//! and components are all just vocabulary entries. Terms rewrite to other terms
//! recursively until reaching primitives.

use std::collections::HashMap;

use super::primitives::{Primitive, Value};

/// A pattern that matches natural language (with optional parameter capture).
///
/// Examples:
/// - "status: {code}" matches "status: 200" and captures code=200
/// - "body contains {text}" matches "body contains \"hello\"" and captures text="hello"
/// - "they see {text}" matches "they see \"Welcome\"" and captures text="Welcome"
#[derive(Debug, Clone)]
pub struct Pattern {
    /// The pattern text with {param} placeholders
    pub text: String,
    /// Parameter names extracted from the pattern
    pub params: Vec<String>,
    /// Compiled regex for matching (built from text)
    regex: Option<regex::Regex>,
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.params == other.params
    }
}

impl Pattern {
    /// Create a new pattern from text.
    /// Extracts parameter names from {param} placeholders.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let params = Self::extract_params(&text);
        let regex = Self::compile_regex(&text, &params);

        Pattern {
            text,
            params,
            regex,
        }
    }

    /// Extract parameter names from {param} placeholders
    fn extract_params(text: &str) -> Vec<String> {
        let re = regex::Regex::new(r"\{(\w+)\}").unwrap();
        re.captures_iter(text)
            .map(|cap| cap[1].to_string())
            .collect()
    }

    /// Compile pattern to regex for matching
    fn compile_regex(text: &str, params: &[String]) -> Option<regex::Regex> {
        if params.is_empty() {
            // No params - exact match (but normalize whitespace)
            return None;
        }

        // First, escape regex special chars
        let mut regex_str = regex::escape(text);

        // Now replace escaped {param} placeholders with capture groups
        // After regex::escape, "{code}" becomes r"\{code\}"
        for param in params {
            let escaped_placeholder = format!(r"\{{{}\}}", param);
            // Match quoted strings (preserving content) or unquoted words
            // For quoted: capture content without quotes
            // For unquoted: capture the word
            let capture = r#"(?:"([^"]+)"|'([^']+)'|(\S+))"#;
            regex_str = regex_str.replace(&escaped_placeholder, capture);
        }

        regex::Regex::new(&format!("^{}$", regex_str)).ok()
    }

    /// Try to match input text against this pattern.
    /// Returns captured parameters if match succeeds.
    pub fn match_text(&self, input: &str) -> Option<HashMap<String, Value>> {
        let input = input.trim();

        if self.params.is_empty() {
            // Exact match (case-insensitive, normalized whitespace)
            if self.text.eq_ignore_ascii_case(input) {
                return Some(HashMap::new());
            }
            return None;
        }

        // Use regex to match and capture
        if let Some(ref re) = self.regex {
            if let Some(caps) = re.captures(input) {
                let mut params = HashMap::new();

                for (i, param_name) in self.params.iter().enumerate() {
                    // Each param has 3 capture groups (quoted double, quoted single, unquoted)
                    // We need to find which one matched
                    let base_idx = 1 + i * 3;
                    let value = caps
                        .get(base_idx)
                        .or_else(|| caps.get(base_idx + 1))
                        .or_else(|| caps.get(base_idx + 2))
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default();

                    params.insert(param_name.clone(), Value::String(value));
                }

                return Some(params);
            }
        }

        None
    }

    /// Check if this pattern has parameters
    pub fn has_params(&self) -> bool {
        !self.params.is_empty()
    }
}

/// What a term resolves to.
#[derive(Debug, Clone)]
pub enum Definition {
    /// Resolves to a sequence of other terms (recursion continues)
    Terms(Vec<Term>),
    /// Resolves to a primitive (base case - execution happens)
    Primitive(Primitive),
}

/// A term reference with captured parameter values.
#[derive(Debug, Clone)]
pub struct Term {
    /// The term text (may contain {param} references)
    pub text: String,
    /// Captured or substituted parameter values
    pub params: HashMap<String, Value>,
}

impl Term {
    /// Create a new term with no parameters
    pub fn new(text: impl Into<String>) -> Self {
        Term {
            text: text.into(),
            params: HashMap::new(),
        }
    }

    /// Create a new term with parameters
    pub fn with_params(text: impl Into<String>, params: HashMap<String, Value>) -> Self {
        Term {
            text: text.into(),
            params,
        }
    }

    /// Substitute parameters into the term text.
    /// Replaces {param} and $param with actual values.
    /// If the value contains spaces, wraps it in quotes.
    pub fn substitute(&self, parent_params: &HashMap<String, Value>) -> Term {
        let mut text = self.text.clone();
        let mut new_params = self.params.clone();

        // Substitute {param} placeholders from parent
        for (name, value) in parent_params {
            let placeholder1 = format!("{{{}}}", name);
            let placeholder2 = format!("${}", name);

            if let Value::String(s) = value {
                // Wrap in quotes if value contains spaces and isn't already quoted
                let substitution = if s.contains(' ') && !s.starts_with('"') {
                    format!("\"{}\"", s)
                } else {
                    s.clone()
                };
                text = text.replace(&placeholder1, &substitution);
                text = text.replace(&placeholder2, &substitution);
            }

            // Also copy params down
            new_params.insert(name.clone(), value.clone());
        }

        Term {
            text,
            params: new_params,
        }
    }
}

/// The entire vocabulary - all terms in one unified structure.
///
/// Standard assertions, glossary terms, and components are all just entries here.
/// Resolution is simple: look up term → get definition → recurse or execute.
#[derive(Debug, Default)]
pub struct Vocabulary {
    /// Terms indexed by their pattern text (for exact lookups)
    terms: HashMap<String, (Pattern, Definition)>,
    /// All patterns in order (for longest-match lookup)
    patterns: Vec<(Pattern, Definition)>,
}

impl Vocabulary {
    /// Create an empty vocabulary
    pub fn new() -> Self {
        Vocabulary {
            terms: HashMap::new(),
            patterns: Vec::new(),
        }
    }

    /// Add a term that resolves to other terms
    pub fn add_terms(&mut self, pattern: impl Into<String>, terms: Vec<Term>) {
        let pattern = Pattern::new(pattern);
        let def = Definition::Terms(terms);
        self.terms
            .insert(pattern.text.clone(), (pattern.clone(), def.clone()));
        self.patterns.push((pattern, def));
    }

    /// Add a term that resolves directly to a primitive
    pub fn add_primitive(&mut self, pattern: impl Into<String>, primitive: Primitive) {
        let pattern = Pattern::new(pattern);
        let def = Definition::Primitive(primitive);
        self.terms
            .insert(pattern.text.clone(), (pattern.clone(), def.clone()));
        self.patterns.push((pattern, def));
    }

    /// Look up a term by exact text (fast path)
    pub fn lookup_exact(&self, text: &str) -> Option<&Definition> {
        self.terms.get(text).map(|(_, def)| def)
    }

    /// Look up a term by pattern matching (handles parameters).
    /// Uses longest-match semantics.
    pub fn lookup(&self, text: &str) -> Option<(HashMap<String, Value>, &Definition)> {
        let text = text.trim();

        // Try exact match first (fast path)
        if let Some((_, def)) = self.terms.get(text) {
            return Some((HashMap::new(), def));
        }

        // Try pattern matching - longest match wins
        let mut best_match: Option<(usize, HashMap<String, Value>, &Definition)> = None;

        for (pattern, def) in &self.patterns {
            if let Some(params) = pattern.match_text(text) {
                let match_len = pattern.text.len();
                if best_match.is_none() || match_len > best_match.as_ref().unwrap().0 {
                    best_match = Some((match_len, params, def));
                }
            }
        }

        best_match.map(|(_, params, def)| (params, def))
    }

    /// Check if vocabulary contains a term
    pub fn contains(&self, text: &str) -> bool {
        self.lookup(text).is_some()
    }

    /// Get number of terms in vocabulary
    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// Check if vocabulary is empty
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Merge another vocabulary into this one
    pub fn merge(&mut self, other: Vocabulary) {
        for (pattern, def) in other.patterns {
            self.terms
                .insert(pattern.text.clone(), (pattern.clone(), def.clone()));
            self.patterns.push((pattern, def));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ial::primitives::CheckOp;

    #[test]
    fn test_pattern_no_params() {
        let p = Pattern::new("status is 2xx");
        assert!(p.params.is_empty());
        assert!(p.match_text("status is 2xx").is_some());
        assert!(p.match_text("status is 3xx").is_none());
    }

    #[test]
    fn test_pattern_with_param() {
        let p = Pattern::new("status: {code}");
        assert_eq!(p.params, vec!["code"]);

        let result = p.match_text("status: 200");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("code"), Some(&Value::String("200".to_string())));
    }

    #[test]
    fn test_pattern_quoted_param() {
        let p = Pattern::new("body contains {text}");
        assert_eq!(p.params, vec!["text"]);

        let result = p.match_text("body contains \"hello world\"");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(
            params.get("text"),
            Some(&Value::String("hello world".to_string()))
        );
    }

    #[test]
    fn test_vocabulary_exact_lookup() {
        let mut vocab = Vocabulary::new();
        vocab.add_primitive(
            "status is 2xx",
            Primitive::Check {
                op: CheckOp::InRange,
                path: "response.status".to_string(),
                expected: Value::Range(200.0, 299.0),
            },
        );

        assert!(vocab.lookup("status is 2xx").is_some());
        assert!(vocab.lookup("status is 3xx").is_none());
    }

    #[test]
    fn test_vocabulary_param_lookup() {
        let mut vocab = Vocabulary::new();
        vocab.add_primitive(
            "status: {code}",
            Primitive::Check {
                op: CheckOp::Equals,
                path: "response.status".to_string(),
                expected: Value::String("{code}".to_string()),
            },
        );

        let result = vocab.lookup("status: 200");
        assert!(result.is_some());
        let (params, _) = result.unwrap();
        assert_eq!(params.get("code"), Some(&Value::String("200".to_string())));
    }

    #[test]
    fn test_term_substitution() {
        let mut parent_params = HashMap::new();
        parent_params.insert(
            "message".to_string(),
            Value::String("Task created".to_string()),
        );

        let term = Term::new("body contains {message}");
        let substituted = term.substitute(&parent_params);

        // Value with spaces gets wrapped in quotes
        assert_eq!(substituted.text, "body contains \"Task created\"");
    }

    #[test]
    fn test_vocabulary_terms() {
        let mut vocab = Vocabulary::new();

        // Add a term that expands to other terms
        vocab.add_terms(
            "success response",
            vec![
                Term::new("status is 2xx"),
                Term::new("body contains \"ok\""),
            ],
        );

        let result = vocab.lookup("success response");
        assert!(result.is_some());
        let (_, def) = result.unwrap();
        assert!(matches!(def, Definition::Terms(_)));
    }
}
