//! Intent-Driven Development (IDD) Module
//!
//! Parses `.intent` files and executes tests against NTNT programs.
//!
//! Intent files use the Intent Assertion Language (IAL) format with:
//! - A glossary defining domain terms and their technical meanings
//! - Natural language scenarios that reference glossary terms
//! - Technical bindings that map terms to HTTP requests and assertions
//!
//! # Example intent file (IAL format):
//! ```yaml
//! ## Glossary
//!
//! | Term | Means |
//! |------|-------|
//! | user | Someone accessing the application |
//! | visits | Makes a GET request to |
//! | homepage | The root path "/" |
//!
//! ---
//!
//! Feature: Homepage
//!   id: feature.homepage
//!   description: "Users can view the welcome page"
//!
//!   Scenario: View homepage
//!     When a user visits the homepage
//!     → they see "Welcome"
//! ```

use colored::*;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::error::IntentError;
use crate::ial::{self, standard_vocabulary, Context as IalContext, Term, Vocabulary};
use crate::interpreter::Interpreter;
use crate::lexer::Lexer;
use crate::parser::Parser as IntentParser;

// ============================================================================
// GLOSSARY SYSTEM (IAL Core)
// ============================================================================

/// A glossary term with its human-readable meaning and optional technical binding
#[derive(Debug, Clone, Serialize)]
pub struct GlossaryTerm {
    /// The term as used in scenarios (e.g., "registered user")
    pub term: String,
    /// Human-readable meaning (e.g., "User with verified account")
    pub meaning: String,
    /// Optional type indicator (action, assertion, component, definition)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub term_type: Option<String>,
    /// Optional technical binding for test execution
    pub binding: Option<TechnicalBinding>,
}

/// Technical binding that maps a glossary term to executable test code
#[derive(Debug, Clone, Default, Serialize)]
pub struct TechnicalBinding {
    /// Setup code to run before the test (e.g., database insert, SQL)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,

    /// HTTP action: "METHOD /path" format (e.g., "POST /auth/login")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,

    /// Request body for the action (JSON, form data, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,

    /// Precondition that must be true before this term applies
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precondition: Option<String>,

    /// Assertions to verify (e.g., "status 200", "body contains 'x'")
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub assert: Vec<String>,

    /// Path value for substitution (for location terms)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// The glossary: a collection of domain terms
#[derive(Debug, Clone, Default, Serialize)]
pub struct Glossary {
    pub terms: HashMap<String, GlossaryTerm>,
}

impl Glossary {
    pub fn new() -> Self {
        Self {
            terms: HashMap::new(),
        }
    }

    /// Add a term to the glossary
    pub fn add_term(&mut self, term: String, meaning: String) {
        self.add_term_with_type(term, meaning, None);
    }

    pub fn add_term_with_type(&mut self, term: String, meaning: String, term_type: Option<String>) {
        self.terms.insert(
            term.to_lowercase(),
            GlossaryTerm {
                term,
                meaning,
                term_type,
                binding: None,
            },
        );
    }

    /// Set a technical binding for a term
    pub fn set_binding(&mut self, term: &str, binding: TechnicalBinding) {
        if let Some(t) = self.terms.get_mut(&term.to_lowercase()) {
            t.binding = Some(binding);
        }
    }

    /// Look up a term (case-insensitive)
    pub fn get(&self, term: &str) -> Option<&GlossaryTerm> {
        self.terms.get(&term.to_lowercase())
    }

    /// Convert this glossary to an IAL Vocabulary for term resolution.
    ///
    /// This creates a vocabulary that combines:
    /// 1. Standard IAL vocabulary (status, body contains, etc.)
    /// 2. User-defined glossary terms from this glossary
    ///
    /// Glossary terms are converted to IAL terms:
    /// - Terms with "body contains" in meaning → body contains assertions
    /// - Terms with "they see" in meaning → body contains assertions  
    /// - Terms with HTTP actions → HTTP action primitives
    pub fn to_ial_vocabulary(&self) -> Vocabulary {
        self.to_ial_vocabulary_with_components(&[])
    }

    /// Convert glossary and components to an IAL Vocabulary.
    ///
    /// Components are added as vocabulary entries that expand to their inherent behavior.
    /// When a component is referenced (e.g., "success response with {message}"),
    /// it expands to all assertions in the component's inherent_behavior.
    ///
    /// # Component Syntax in Glossary
    ///
    /// Glossary terms can reference components:
    /// ```text
    /// | success response with {text} | component.success_response(message: {text}) |
    /// ```
    ///
    /// # Parameter Substitution
    ///
    /// Component inherent behaviors use `$param` syntax which gets substituted:
    /// ```text
    /// Inherent Behavior:
    ///   → status 2xx
    ///   → body contains "$message"
    /// ```
    pub fn to_ial_vocabulary_with_components(&self, components: &[Component]) -> Vocabulary {
        let mut vocab = standard_vocabulary();

        // First, add all components as vocabulary entries
        // Component pattern: "component.id(param1: {param1}, param2: {param2})"
        for component in components {
            if component.id.is_empty() || component.inherent_behavior.is_empty() {
                continue;
            }

            // Build the pattern with parameters
            // E.g., "component.success_response(message: {message})"
            let pattern = if component.parameters.is_empty() {
                component.id.clone()
            } else {
                let params_str = component
                    .parameters
                    .iter()
                    .map(|p| format!("{}: {{{}}}", p, p))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({})", component.id, params_str)
            };

            // Convert inherent behaviors to IAL terms
            // Replace $param with {param} for IAL substitution
            let terms: Vec<Term> = component
                .inherent_behavior
                .iter()
                .map(|behavior| {
                    let mut ial_behavior = behavior.clone();
                    // Replace $param with {param} for each parameter
                    for param in &component.parameters {
                        ial_behavior =
                            ial_behavior.replace(&format!("${}", param), &format!("{{{}}}", param));
                    }
                    Term::new(ial_behavior)
                })
                .collect();

            vocab.add_terms(&pattern, terms);
        }

        // Add glossary terms as vocabulary entries
        // Handle both parameterized terms ($param) and static terms
        for term in self.terms.values() {
            let meaning_lower = term.meaning.to_lowercase();

            // Convert $param to {param} for IAL compatibility
            let ial_pattern = Self::convert_params_to_ial(&term.term);
            let ial_meaning = Self::convert_params_to_ial(&term.meaning);

            // Check if this term references a component
            if meaning_lower.contains("component.") {
                if let Some(comp_ref) = Self::extract_component_reference(&ial_meaning) {
                    vocab.add_terms(&ial_pattern, vec![Term::new(&comp_ref)]);
                }
                continue;
            }

            // Handle comma-separated meanings (compound assertions)
            // E.g., "status 200, returns HTML" → two separate terms
            let meaning_parts: Vec<&str> = ial_meaning.split(',').map(|s| s.trim()).collect();
            let mut sub_terms: Vec<Term> = Vec::new();

            for part in meaning_parts {
                let part_lower = part.to_lowercase();

                // status N or status: N
                if part_lower.starts_with("status") {
                    sub_terms.push(Term::new(part));
                }
                // body not contains X (check before body contains to avoid false match)
                else if part_lower.contains("body not contains") {
                    sub_terms.push(Term::new(part));
                }
                // body contains X
                else if part_lower.contains("body contains") {
                    sub_terms.push(Term::new(part));
                }
                // header X contains Y
                else if part_lower.contains("header") && part_lower.contains("contains") {
                    sub_terms.push(Term::new(part));
                }
                // returns HTML / returns JSON
                else if part_lower.contains("returns html") {
                    sub_terms.push(Term::new("header \"Content-Type\" contains \"text/html\""));
                } else if part_lower.contains("returns json") {
                    sub_terms.push(Term::new(
                        "header \"Content-Type\" contains \"application/json\"",
                    ));
                }
                // Reference to another glossary term - add as-is for recursive resolution
                else if !part.is_empty()
                    && !part_lower.starts_with("get ")
                    && !part_lower.starts_with("post ")
                {
                    sub_terms.push(Term::new(part));
                }
            }

            if !sub_terms.is_empty() {
                vocab.add_terms(&ial_pattern, sub_terms);
            }
        }

        vocab
    }

    /// Convert $param syntax to {param} syntax for IAL
    /// E.g., 'they see "$text"' → 'they see "{text}"'
    fn convert_params_to_ial(s: &str) -> String {
        let re = regex::Regex::new(r"\$(\w+)").unwrap();
        re.replace_all(s, "{$1}").to_string()
    }

    /// Extract component reference from glossary meaning
    /// Example: "Displays component.success_message(message: {text})" → "component.success_message(message: {text})"
    fn extract_component_reference(meaning: &str) -> Option<String> {
        // Look for "component." followed by identifier and optional params
        if let Some(start) = meaning.find("component.") {
            let rest = &meaning[start..];

            // Find the end: either closing paren for params or whitespace/end
            if rest.contains('(') {
                // Has parameters - find closing paren
                if let Some(paren_end) = rest.find(')') {
                    return Some(rest[..paren_end + 1].to_string());
                }
            }

            // No parameters - find end by whitespace
            let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
            return Some(rest[..end].to_string());
        }
        None
    }

    /// Resolve a scenario's when_clause to extract the action to perform.
    /// Returns WhenAction::Http for HTTP requests, WhenAction::CodeQuality for code quality checks.
    pub fn resolve_when_clause(&self, when_clause: &str) -> Option<WhenAction> {
        let when_lower = when_clause.to_lowercase();

        // Check for code quality scenarios first
        if when_lower.contains("checking code quality")
            || when_lower.contains("running code quality")
            || when_lower.contains("validating code")
            || when_lower.contains("linting code")
        {
            return Some(WhenAction::CodeQuality {
                file: None,
                lint: true,
                validate: true,
            });
        }

        // First, try to match parameterized glossary terms
        // Example: "a visitor goes to $path" -> "GET $path"
        // Matching "a visitor goes to /" should expand to "GET /"
        for term in self.terms.values() {
            if let Some((method, path, body)) =
                self.match_parameterized_when_term(when_clause, term)
            {
                return Some(WhenAction::Http { method, path, body });
            }
        }

        // Fallback: identify action verb and path separately
        let mut method = "GET".to_string();
        let mut found_action = false;

        for term in self.terms.values() {
            let term_lower = term.term.to_lowercase();
            if when_lower.contains(&term_lower) {
                // First check technical binding for action
                if let Some(ref binding) = term.binding {
                    if let Some(ref action) = binding.action {
                        let action_upper = action.to_uppercase();
                        if action_upper == "GET"
                            || action_upper == "POST"
                            || action_upper == "PUT"
                            || action_upper == "DELETE"
                            || action_upper == "PATCH"
                        {
                            method = action_upper;
                            found_action = true;
                            continue;
                        }
                    }
                }

                // Fallback to meaning-based detection
                let meaning_lower = term.meaning.to_lowercase();

                // Check if this term defines an HTTP method
                if meaning_lower.contains("get request") || meaning_lower.contains("get ") {
                    method = "GET".to_string();
                    found_action = true;
                } else if meaning_lower.contains("post request") || meaning_lower.contains("post ")
                {
                    method = "POST".to_string();
                    found_action = true;
                } else if meaning_lower.contains("put request") || meaning_lower.contains("put ") {
                    method = "PUT".to_string();
                    found_action = true;
                } else if meaning_lower.contains("delete request")
                    || meaning_lower.contains("delete ")
                {
                    method = "DELETE".to_string();
                    found_action = true;
                } else if meaning_lower.contains("patch request")
                    || meaning_lower.contains("patch ")
                {
                    method = "PATCH".to_string();
                    found_action = true;
                }
            }
        }

        // Then find the path from location terms
        let mut path: Option<String> = None;

        for term in self.terms.values() {
            let term_lower = term.term.to_lowercase();
            if when_lower.contains(&term_lower) {
                // First check technical binding for path
                if let Some(ref binding) = term.binding {
                    if let Some(ref bound_path) = binding.path {
                        path = Some(bound_path.clone());
                        continue;
                    }
                }

                // Fallback to extracting path from meaning
                if let Some(extracted) = Self::extract_path_from_meaning(&term.meaning) {
                    path = Some(extracted);
                }
            }
        }

        // Also check for inline paths in the when_clause like "visits /api/tasks"
        if path.is_none() {
            if let Some(p) = Self::extract_path_from_clause(when_clause) {
                path = Some(p);
            }
        }

        // Check for body content (e.g., "creates a task with title "Buy groceries"")
        let body = Self::extract_body_from_clause(when_clause);

        if found_action && path.is_some() {
            Some(WhenAction::Http {
                method,
                path: path.unwrap(),
                body,
            })
        } else if path.is_some() {
            // Default to GET if we found a path but no explicit action
            Some(WhenAction::Http {
                method,
                path: path.unwrap(),
                body,
            })
        } else {
            None
        }
    }

    /// Match a parameterized glossary term against a when clause
    /// Example: term "a visitor goes to $path" with meaning "GET $path"
    /// Input: "a visitor goes to /"
    /// Returns: Some(("GET", "/", None))
    fn match_parameterized_when_term(
        &self,
        clause: &str,
        term: &GlossaryTerm,
    ) -> Option<(String, String, Option<String>)> {
        let term_text = &term.term;
        let meaning = &term.meaning;

        // Check if term has a parameter placeholder ($param)
        if !term_text.contains('$') {
            return None;
        }

        // Build a regex pattern from the term
        // "a visitor goes to $path" -> "a visitor goes to (.+)"
        let pattern = Self::term_to_regex_pattern(term_text);
        let re = regex::Regex::new(&format!("(?i)^{}$", pattern)).ok()?;

        // Try to match the clause
        let caps = re.captures(clause)?;

        // Extract parameter values
        let mut params: HashMap<String, String> = HashMap::new();
        let param_names = Self::extract_param_names(term_text);

        for (i, name) in param_names.iter().enumerate() {
            if let Some(value) = caps.get(i + 1) {
                params.insert(name.clone(), value.as_str().to_string());
            }
        }

        // Substitute parameters in the meaning
        let mut expanded = meaning.clone();
        for (name, value) in &params {
            expanded = expanded.replace(&format!("${}", name), value);
        }

        // Parse the expanded meaning to extract method and path
        // Expected format: "GET /path" or "POST /path"
        let parts: Vec<&str> = expanded.trim().splitn(2, ' ').collect();
        if parts.len() == 2 {
            let method = parts[0].to_uppercase();
            let path = parts[1].to_string();
            if ["GET", "POST", "PUT", "DELETE", "PATCH"].contains(&method.as_str()) {
                return Some((method, path, None));
            }
        }

        None
    }

    /// Convert a parameterized term to a regex pattern
    /// "a visitor goes to $path" -> "a visitor goes to (.+)"
    fn term_to_regex_pattern(term: &str) -> String {
        let mut pattern = regex::escape(term);
        // Replace escaped $param with capture group
        let re = regex::Regex::new(r"\\\$(\w+)").unwrap();
        pattern = re.replace_all(&pattern, "(.+)").to_string();
        pattern
    }

    /// Extract parameter names from a term
    /// "a visitor goes to $path" -> ["path"]
    fn extract_param_names(term: &str) -> Vec<String> {
        let re = regex::Regex::new(r"\$(\w+)").unwrap();
        re.captures_iter(term)
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
            .collect()
    }

    /// Extract a path from a glossary term's meaning
    fn extract_path_from_meaning(meaning: &str) -> Option<String> {
        // Pattern: contains "/" followed by optional path
        // Examples: 'The root path "/"', 'endpoint /api/status', 'at /users'

        // Try to find quoted path first: "/path"
        if let Some(start) = meaning.find('"') {
            if let Some(end) = meaning[start + 1..].find('"') {
                let path = &meaning[start + 1..start + 1 + end];
                if path.starts_with('/') {
                    return Some(path.to_string());
                }
            }
        }

        // Try to find unquoted path: /path (word starting with /)
        for word in meaning.split_whitespace() {
            if word.starts_with('/') {
                // Clean up trailing punctuation
                let path = word.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '/');
                return Some(path.to_string());
            }
        }

        None
    }

    /// Extract path directly from the when clause (e.g., "visits /api/tasks")
    fn extract_path_from_clause(clause: &str) -> Option<String> {
        for word in clause.split_whitespace() {
            if word.starts_with('/') {
                let path = word.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '/');
                return Some(path.to_string());
            }
        }
        None
    }

    /// Extract body content from when clause (e.g., 'with title "Buy groceries"')
    fn extract_body_from_clause(clause: &str) -> Option<String> {
        // Look for patterns like: with title "value", with body {...}
        if let Some(with_idx) = clause.to_lowercase().find("with ") {
            let rest = &clause[with_idx + 5..];

            // Try to extract JSON if present
            if let Some(json_start) = rest.find('{') {
                if let Some(json_end) = rest.rfind('}') {
                    let json = &rest[json_start..=json_end];
                    return Some(json.to_string());
                }
            }

            // Try to extract quoted values for common patterns
            // "with title "Buy groceries"" -> {"title": "Buy groceries"}
            if let Some(title_idx) = rest.to_lowercase().find("title ") {
                let after_title = &rest[title_idx + 6..];
                if let Some(q1) = after_title.find('"') {
                    if let Some(q2) = after_title[q1 + 1..].find('"') {
                        let title = &after_title[q1 + 1..q1 + 1 + q2];
                        return Some(format!(r#"{{"title": "{}", "completed": false}}"#, title));
                    }
                }
            }
        }
        None
    }

    /// Resolve an outcome clause to an assertion using IAL vocabulary
    /// Examples: "they see 'Task Manager'" -> BodyContains("Task Manager")
    ///
    /// Resolution flow:
    /// 1. Build IAL vocabulary from glossary (handles parameterized terms)
    /// 2. Look up the outcome in vocabulary - this expands glossary terms
    /// 3. Recursively resolve to get final assertion terms
    /// 4. Convert to Assertion enum values
    pub fn resolve_outcome(&self, outcome: &str) -> Option<Assertion> {
        // Use resolve_outcomes and return the first result for backwards compatibility
        let assertions = self.resolve_outcomes(outcome);
        assertions.into_iter().next()
    }

    /// Resolve an outcome to ALL assertions it expands to (handles compound glossary terms)
    ///
    /// For example, "has proper layout" might expand to:
    /// - has site branding → body contains "NTNT"
    /// - has navigation → body contains "Learn", body contains "Blog"
    /// - body contains "footer"
    ///
    /// This returns all 4 assertions, not just the first one.
    pub fn resolve_outcomes(&self, outcome: &str) -> Vec<Assertion> {
        // Build IAL vocabulary from glossary
        let vocab = self.to_ial_vocabulary();

        // Convert outcome to IAL format ($param -> {param}) for lookup
        let ial_outcome = Self::convert_params_to_ial(outcome);

        // Try to resolve through IAL vocabulary first
        let mut assertions = self.resolve_outcomes_via_ial(&ial_outcome, &vocab);

        // If no IAL resolution, fall back to direct pattern matching
        if assertions.is_empty() {
            if let Some(assertion) = self.resolve_outcome_direct(outcome) {
                assertions.push(assertion);
            }
        }

        assertions
    }

    /// Resolve an outcome through IAL vocabulary lookup and term expansion
    /// Returns ALL assertions from compound terms (recursive)
    fn resolve_outcomes_via_ial(&self, outcome: &str, vocab: &Vocabulary) -> Vec<Assertion> {
        let mut assertions = Vec::new();

        // Look up in vocabulary - this handles parameterized terms
        if let Some((params, definition)) = vocab.lookup(outcome) {
            match definition {
                ial::Definition::Terms(sub_terms) => {
                    // Expand ALL sub-terms and collect ALL assertions
                    for sub_term in sub_terms {
                        // Substitute captured params
                        let substituted = sub_term.substitute(&params);

                        // Try to convert the substituted term to an assertion
                        if let Some(assertion) = self.term_text_to_assertion(&substituted.text) {
                            assertions.push(assertion);
                        } else {
                            // Recursively resolve through vocabulary (this handles nested terms)
                            let nested = self.resolve_outcomes_via_ial(&substituted.text, vocab);
                            if !nested.is_empty() {
                                assertions.extend(nested);
                            } else {
                                // Try direct pattern matching as final fallback
                                if let Some(assertion) =
                                    self.resolve_outcome_direct(&substituted.text)
                                {
                                    assertions.push(assertion);
                                }
                            }
                        }
                    }
                }
                ial::Definition::Primitive(primitive) => {
                    // Convert IAL primitive to Assertion
                    if let Some(assertion) = self.primitive_to_assertion(primitive, &params) {
                        assertions.push(assertion);
                    }
                }
            }
        }

        assertions
    }

    /// Convert an IAL primitive to an Assertion enum value
    fn primitive_to_assertion(
        &self,
        primitive: &ial::Primitive,
        params: &HashMap<String, ial::Value>,
    ) -> Option<Assertion> {
        match primitive {
            ial::Primitive::Check { op, path, expected } => {
                // Handle code quality assertions first (they use Bool/Number values directly)
                match (op, path.as_str()) {
                    (ial::CheckOp::Equals, "code.quality.passed") => {
                        if let ial::Value::Bool(true) = expected {
                            return Some(Assertion::CodeQualityPassed);
                        }
                        return None;
                    }
                    (ial::CheckOp::Equals, "code.quality.error_count") => {
                        if let ial::Value::Number(n) = expected {
                            if *n == 0.0 {
                                return Some(Assertion::CodeQualityNoErrors);
                            } else {
                                return Some(Assertion::CodeQualityErrorCount(*n as u32));
                            }
                        }
                        return None;
                    }
                    (ial::CheckOp::Equals, "code.quality.warning_count") => {
                        if let ial::Value::Number(n) = expected {
                            if *n == 0.0 {
                                return Some(Assertion::CodeQualityNoWarnings);
                            }
                        }
                        return None;
                    }
                    _ => {}
                }

                // Substitute params in path (for patterns like response.headers.{name})
                let substituted_path = {
                    let mut result = path.clone();
                    for (name, value) in params {
                        if let ial::Value::String(v) = value {
                            result = result.replace(&format!("{{{}}}", name), v);
                        }
                    }
                    result
                };

                // Substitute params in expected value for HTTP assertions
                let expected_str = match expected {
                    ial::Value::String(s) => {
                        let mut result = s.clone();
                        for (name, value) in params {
                            if let ial::Value::String(v) = value {
                                result = result.replace(&format!("{{{}}}", name), v);
                            }
                        }
                        result
                    }
                    ial::Value::Number(n) => format!("{}", n),
                    ial::Value::Range(start, end) => format!("{}-{}", start, end),
                    _ => return None,
                };

                match (op, substituted_path.as_str()) {
                    (ial::CheckOp::Equals, "response.status") => {
                        if let Ok(code) = expected_str.parse::<u16>() {
                            return Some(Assertion::Status(code));
                        }
                    }
                    (ial::CheckOp::InRange, "response.status") => {
                        // Status range like 2xx
                        if let ial::Value::Range(start, _) = expected {
                            return Some(Assertion::Status(*start as u16));
                        }
                    }
                    (ial::CheckOp::Contains, "response.body") => {
                        return Some(Assertion::BodyContains(expected_str));
                    }
                    (ial::CheckOp::NotContains, "response.body") => {
                        return Some(Assertion::BodyNotContains(expected_str));
                    }
                    (ial::CheckOp::Contains, p) if p.starts_with("response.headers.") => {
                        let header_name = p.trim_start_matches("response.headers.");
                        return Some(Assertion::HeaderContains(
                            header_name.to_string(),
                            expected_str,
                        ));
                    }
                    (ial::CheckOp::Exists, p) if p.contains("json") || p.contains("id") => {
                        return Some(Assertion::JsonPathExists(expected_str));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        None
    }

    /// Convert a term text string to an assertion (for resolved IAL terms)
    fn term_text_to_assertion(&self, text: &str) -> Option<Assertion> {
        let text_lower = text.to_lowercase();

        // Status assertions
        if text_lower.starts_with("status") {
            // "status: 200" or "status 200"
            let num_str: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
            if let Ok(code) = num_str.parse::<u16>() {
                return Some(Assertion::Status(code));
            }
            // "status 2xx"
            if text_lower.contains("2xx") {
                return Some(Assertion::Status(200));
            }
        }

        // Body assertions (check "not contains" first to avoid false match)
        if text_lower.contains("body not contains") {
            if let Some(content) = Self::extract_quoted_text(text) {
                return Some(Assertion::BodyNotContains(content));
            }
        }

        if text_lower.contains("body contains") {
            if let Some(content) = Self::extract_quoted_text(text) {
                return Some(Assertion::BodyContains(content));
            }
        }

        // Header assertions
        if text_lower.contains("header") && text_lower.contains("contains") {
            // Pattern: header "X" contains "Y" or header X contains "Y"
            let re = regex::Regex::new(r#"header\s+"?([^"]+)"?\s+contains\s+"([^"]+)""#).ok()?;
            if let Some(caps) = re.captures(text) {
                let header = caps.get(1)?.as_str().trim_matches('"').to_string();
                let value = caps.get(2)?.as_str().to_string();
                return Some(Assertion::HeaderContains(header, value));
            }
        }

        // JSON path assertions
        if text_lower.contains("has an id") || text_lower.contains("has id") {
            return Some(Assertion::JsonPathExists("id".to_string()));
        }

        // CLI assertions
        // "run 'command'" or 'run "command"'
        if text_lower.starts_with("run ") {
            if let Some(cmd) = Self::extract_quoted_text(text) {
                // Split command into program and args
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                if !parts.is_empty() {
                    let program = parts[0].to_string();
                    let args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
                    return Some(Assertion::CliRun(program, args));
                }
            }
        }

        // "exits successfully" or "exits with code 0"
        if text_lower.contains("exits successfully")
            || text_lower.contains("exit code 0")
            || text_lower == "exit code is 0"
        {
            return Some(Assertion::CliExitCode(0));
        }
        if text_lower.contains("exits with code") || text_lower.contains("exit code") {
            let num_str: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
            if let Ok(code) = num_str.parse::<i32>() {
                return Some(Assertion::CliExitCode(code));
            }
        }

        // "output contains" (check "not contains" first)
        if text_lower.contains("output not contains") {
            if let Some(content) = Self::extract_quoted_text(text) {
                return Some(Assertion::CliOutputNotContains(content));
            }
        }
        if text_lower.contains("output contains") {
            if let Some(content) = Self::extract_quoted_text(text) {
                return Some(Assertion::CliOutputContains(content));
            }
        }

        // "error output contains" or "stderr contains"
        if text_lower.contains("error output contains") || text_lower.contains("stderr contains") {
            if let Some(content) = Self::extract_quoted_text(text) {
                return Some(Assertion::CliErrorContains(content));
            }
        }

        None
    }

    /// Direct pattern matching for built-in assertion patterns (fallback)
    fn resolve_outcome_direct(&self, outcome: &str) -> Option<Assertion> {
        let outcome_lower = outcome.to_lowercase();

        // "they see 'X'" or 'they see "X"'
        if outcome_lower.contains("see ") || outcome_lower.contains("sees ") {
            if let Some(text) = Self::extract_quoted_text(outcome) {
                return Some(Assertion::BodyContains(text));
            }
        }

        // "has an ID" pattern
        if Self::matches_word_pattern(&outcome_lower, "has an id")
            || Self::matches_word_pattern(&outcome_lower, "has id")
        {
            return Some(Assertion::JsonPathExists("id".to_string()));
        }

        // "as JSON" pattern
        if outcome_lower.contains("as json") {
            return Some(Assertion::HeaderContains(
                "Content-Type".to_string(),
                "application/json".to_string(),
            ));
        }

        // Direct status pattern
        if outcome_lower.starts_with("status") {
            let num_str: String = outcome.chars().filter(|c| c.is_ascii_digit()).collect();
            if let Ok(code) = num_str.parse::<u16>() {
                return Some(Assertion::Status(code));
            }
        }

        // Direct body contains pattern (check "not contains" first)
        if outcome_lower.contains("body not contains") {
            if let Some(text) = Self::extract_quoted_text(outcome) {
                return Some(Assertion::BodyNotContains(text));
            }
        }

        if outcome_lower.contains("body contains") {
            if let Some(text) = Self::extract_quoted_text(outcome) {
                return Some(Assertion::BodyContains(text));
            }
        }

        // Direct header pattern
        if outcome_lower.contains("header") && outcome_lower.contains("contains") {
            let re = regex::Regex::new(r#"header\s+"([^"]+)"\s+contains\s+"([^"]+)""#).ok()?;
            if let Some(caps) = re.captures(outcome) {
                let header = caps.get(1)?.as_str().to_string();
                let value = caps.get(2)?.as_str().to_string();
                return Some(Assertion::HeaderContains(header, value));
            }
        }

        None
    }

    /// Check if a pattern matches with word boundaries (not as a substring of a larger word)
    fn matches_word_pattern(text: &str, pattern: &str) -> bool {
        if let Some(idx) = text.find(pattern) {
            let end_idx = idx + pattern.len();
            // Check that the character after the pattern (if any) is not alphanumeric
            if end_idx < text.len() {
                let next_char = text[end_idx..].chars().next().unwrap_or(' ');
                if next_char.is_alphanumeric() {
                    return false; // Pattern is part of a larger word
                }
            }
            true
        } else {
            false
        }
    }

    /// Extract text from quotes (single or double)
    fn extract_quoted_text(s: &str) -> Option<String> {
        // Try double quotes first
        if let Some(start) = s.find('"') {
            if let Some(end) = s[start + 1..].find('"') {
                return Some(s[start + 1..start + 1 + end].to_string());
            }
        }
        // Try single quotes
        if let Some(start) = s.find('\'') {
            if let Some(end) = s[start + 1..].find('\'') {
                return Some(s[start + 1..start + 1 + end].to_string());
            }
        }
        // Try smart quotes
        if let Some(start) = s.find('"') {
            if let Some(end) = s[start + 1..].find('"') {
                return Some(s[start + '"'.len_utf8()..start + '"'.len_utf8() + end].to_string());
            }
        }
        None
    }

    /// Resolve a full scenario to a TestCase, expanding component references
    /// Returns (TestCase, unresolved_outcomes, component_refs)
    /// where unresolved_outcomes are outcomes that couldn't be mapped
    /// and component_refs are IDs of components used
    pub fn resolve_scenario(
        &self,
        scenario: &Scenario,
        components: &[Component],
    ) -> Option<(TestCase, Vec<String>, Vec<String>)> {
        self.resolve_scenario_with_base_dir(scenario, components, None)
    }

    /// Resolve a scenario to a TestCase with optional base directory for code quality checks.
    pub fn resolve_scenario_with_base_dir(
        &self,
        scenario: &Scenario,
        components: &[Component],
        base_dir: Option<&str>,
    ) -> Option<(TestCase, Vec<String>, Vec<String>)> {
        // Resolve preconditions from Given clause if present
        let mut preconditions = Vec::new();
        if let Some(given) = &scenario.given_clause {
            // Try to resolve Given clause as an outcome (e.g., "no tasks exist")
            // This becomes precondition assertions to verify before the test
            // Use resolve_outcomes for compound terms
            let resolved = self.resolve_outcomes(given);
            preconditions.extend(resolved);
            // Note: If Given doesn't resolve, we just skip it (it's descriptive only)
        }

        // Resolve the when clause to get the action
        let when_action = self.resolve_when_clause(&scenario.when_clause)?;

        // Resolve each outcome to an assertion
        let mut assertions = Vec::new();
        let mut unresolved = Vec::new();
        let mut component_refs = Vec::new();

        // Determine method/path/body based on action type
        let (method, path, body) = match &when_action {
            WhenAction::Http { method, path, body } => {
                // Add appropriate default status based on HTTP method
                // POST typically returns 201 (Created), others return 200
                let default_status = if method == "POST" { 201 } else { 200 };
                assertions.push(Assertion::Status(default_status));
                (method.clone(), path.clone(), body.clone())
            }
            WhenAction::CodeQuality { file, .. } => {
                // Code quality scenarios use a special method marker
                // No default HTTP assertions for code quality
                // Use base_dir if provided, otherwise default to "."
                let quality_path = file.clone().unwrap_or_else(|| {
                    base_dir
                        .map(|d| d.to_string())
                        .unwrap_or_else(|| ".".to_string())
                });
                ("CODE_QUALITY".to_string(), quality_path, None)
            }
        };

        for outcome in &scenario.outcomes {
            // First check if this outcome references a component
            if let Some((component, params)) = self.resolve_component_reference(outcome, components)
            {
                // Add component to refs
                component_refs.push(component.id.clone());

                // Expand component's inherent behavior with parameter substitution
                for behavior in &component.inherent_behavior {
                    let substituted =
                        Self::substitute_params(behavior, &component.parameters, &params);
                    // Use resolve_outcomes for recursive expansion of compound terms
                    let resolved = self.resolve_outcomes(&substituted);
                    if !resolved.is_empty() {
                        assertions.extend(resolved);
                    } else {
                        unresolved.push(substituted);
                    }
                }
            } else {
                // Use resolve_outcomes to get ALL assertions from compound terms
                let resolved = self.resolve_outcomes(outcome);
                if !resolved.is_empty() {
                    assertions.extend(resolved);
                } else {
                    // Track unresolved outcome
                    unresolved.push(outcome.clone());
                }
            }
        }

        Some((
            TestCase {
                method,
                path,
                body,
                assertions,
                preconditions,
                scenario_name: Some(scenario.name.clone()),
            },
            unresolved,
            component_refs,
        ))
    }

    /// Check if an outcome references a component and extract parameters
    /// Example: "success message with 'Welcome'" → (SuccessMessage component, ["Welcome"])
    fn resolve_component_reference<'a>(
        &self,
        outcome: &str,
        components: &'a [Component],
    ) -> Option<(&'a Component, Vec<String>)> {
        // Look for component references in glossary terms
        // Pattern: "success response with 'X'" where glossary maps "success response with $message" to component

        for term in self.terms.values() {
            // Check if the term's meaning references a component
            if !term.meaning.to_lowercase().contains("component.") {
                continue;
            }

            // Strip parameter placeholders from term for matching
            // "success response with \"$message\"" → "success response with"
            let term_pattern = Self::strip_param_placeholders(&term.term);
            let term_pattern_lower = term_pattern.to_lowercase().trim().to_string();
            let outcome_lower = outcome.to_lowercase();

            // Check if outcome contains this term pattern
            if outcome_lower.contains(&term_pattern_lower) {
                // Extract component ID from meaning
                if let Some(comp_id) = Self::extract_component_id(&term.meaning) {
                    // Find the component
                    if let Some(component) = components.iter().find(|c| c.id == comp_id) {
                        // Extract parameter values from the outcome
                        let params = Self::extract_component_params(outcome);
                        return Some((component, params));
                    }
                }
            }
        }

        None
    }

    /// Strip parameter placeholders from a term for matching
    /// Example: "success response with \"$message\"" → "success response with"
    fn strip_param_placeholders(term: &str) -> String {
        let mut result = term.to_string();

        // Remove $param patterns (with or without quotes)
        // Pattern: "$name" or '$name' or $name
        let re_quoted = regex::Regex::new(r#"["'""]\$\w+["'""]\s*"#).unwrap();
        result = re_quoted.replace_all(&result, "").to_string();

        let re_unquoted = regex::Regex::new(r#"\$\w+\s*"#).unwrap();
        result = re_unquoted.replace_all(&result, "").to_string();

        result.trim().to_string()
    }

    /// Extract component ID from a glossary meaning
    /// Example: "Displays component.success_message" → "component.success_message"
    fn extract_component_id(meaning: &str) -> Option<String> {
        // Look for "component." followed by identifier
        if let Some(start) = meaning.find("component.") {
            let rest = &meaning[start..];
            // Extract until whitespace or end
            let end = rest
                .find(|c: char| c.is_whitespace() || c == ')' || c == ',')
                .unwrap_or(rest.len());
            return Some(rest[..end].to_string());
        }
        None
    }

    /// Extract parameter values from outcome text
    /// Example: "success message with 'Welcome to the Test Server'" → ["Welcome to the Test Server"]
    fn extract_component_params(outcome: &str) -> Vec<String> {
        let mut params = Vec::new();

        // Extract all quoted strings as parameters
        let mut chars = outcome.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '"' || ch == '\'' || ch == '"' {
                let quote = ch;
                let mut param = String::new();
                while let Some(next) = chars.next() {
                    if next == quote || next == '"' || next == '\'' {
                        break;
                    }
                    param.push(next);
                }
                if !param.is_empty() {
                    params.push(param);
                }
            }
        }

        params
    }

    /// Substitute parameters in a behavior string
    /// Example: "page contains \"$text\"" with params=["Welcome"] → "page contains \"Welcome\""
    fn substitute_params(
        behavior: &str,
        param_names: &[String],
        param_values: &[String],
    ) -> String {
        let mut result = behavior.to_string();

        for (i, name) in param_names.iter().enumerate() {
            if let Some(value) = param_values.get(i) {
                // Replace $param_name with actual value
                let pattern = format!("${}", name);
                result = result.replace(&pattern, value);
            }
        }

        result
    }
}

// ============================================================================
// COMPONENT SYSTEM (Reusable Intent Blocks)
// ============================================================================

/// A reusable component with inherent behavior and scenarios
#[derive(Debug, Clone, Serialize)]
pub struct Component {
    /// Component ID (e.g., "component.error_popup")
    pub id: String,
    /// Component name (e.g., "Error Popup")
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// Parameters this component accepts (e.g., ["message"])
    pub parameters: Vec<String>,
    /// Assertions that always apply when component is referenced (Inherent Behavior)
    pub inherent_behavior: Vec<String>,
    /// Additional scenarios for this component
    pub scenarios: Vec<Scenario>,
}

// ============================================================================
// SCENARIO SYSTEM (Natural Language Tests)
// ============================================================================

/// A natural language scenario that can be executed as a test
#[derive(Debug, Clone, Serialize)]
pub struct Scenario {
    /// Scenario name (e.g., "Successful login")
    pub name: String,
    /// Optional description explaining why this scenario exists
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional "Given" clause describing preconditions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub given_clause: Option<String>,
    /// The "When" clause describing the trigger
    pub when_clause: String,
    /// The outcome clauses (each "→" line)
    pub outcomes: Vec<String>,
    /// Resolved test case (after glossary term resolution)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_test: Option<TestCase>,
    /// Component references used in outcomes (for cascading verification)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub component_refs: Vec<String>,
}

// ============================================================================
// ASSERTIONS
// ============================================================================

/// A single assertion within a test
#[derive(Debug, Clone, Serialize)]
pub enum Assertion {
    /// Check HTTP status code: `status: 200`
    Status(u16),
    /// Check body contains text: `body contains "text"`
    BodyContains(String),
    /// Check body matches regex: `body matches r"pattern"`
    BodyMatches(String),
    /// Check body does not contain: `body not contains "error"`
    BodyNotContains(String),
    /// Check header value: `header "Content-Type" contains "text/html"`
    HeaderContains(String, String),
    /// Check JSON path exists: `json path "token" exists`
    JsonPathExists(String),
    /// Check JSON path equals value: `json path "status" == "ok"`
    JsonPathEquals(String, String),
    /// Check redirect: `redirects to "/dashboard"`
    RedirectsTo(String),

    // === CLI Assertions ===
    /// Run a CLI command: `run "ntnt lint server.tnt"`
    CliRun(String, Vec<String>),
    /// Check exit code: `exits successfully` or `exits with code 0`
    CliExitCode(i32),
    /// Check stdout contains: `output contains "text"`
    CliOutputContains(String),
    /// Check stdout does not contain: `output not contains "error"`
    CliOutputNotContains(String),
    /// Check stderr contains: `error output contains "text"`
    CliErrorContains(String),

    // === Code Quality Assertions ===
    /// Check that code quality passed: `code is valid`
    CodeQualityPassed,
    /// Check that there are no errors: `no syntax errors`
    CodeQualityNoErrors,
    /// Check that there are no warnings: `no lint warnings`
    CodeQualityNoWarnings,
    /// Check that error count is below threshold: `error count < N`
    CodeQualityErrorCount(u32),
}

// ============================================================================
// WHEN CLAUSE ACTIONS
// ============================================================================

/// Represents the action to perform from a When clause.
/// Scenarios can trigger HTTP requests or code quality checks.
#[derive(Debug, Clone)]
pub enum WhenAction {
    /// HTTP request: method, path, optional body
    Http {
        method: String,
        path: String,
        body: Option<String>,
    },
    /// Code quality check: lint and/or validate .tnt files
    CodeQuality {
        file: Option<String>,
        lint: bool,
        validate: bool,
    },
}

// ============================================================================
// IAL BRIDGE - Connects Intent assertions to the IAL term rewriting engine
// ============================================================================

impl Assertion {
    /// Convert an Assertion to its IAL term text representation
    pub fn to_ial_term(&self) -> String {
        match self {
            Assertion::Status(code) => format!("status: {}", code),
            Assertion::BodyContains(text) => format!("body contains \"{}\"", text),
            Assertion::BodyMatches(pattern) => format!("body matches \"{}\"", pattern),
            Assertion::BodyNotContains(text) => format!("body not contains \"{}\"", text),
            Assertion::HeaderContains(name, value) => {
                format!("header \"{}\" contains \"{}\"", name, value)
            }
            Assertion::JsonPathExists(path) => format!("json path \"{}\" exists", path),
            Assertion::JsonPathEquals(path, value) => {
                format!("json path \"{}\" == \"{}\"", path, value)
            }
            Assertion::RedirectsTo(path) => format!("redirects to \"{}\"", path),
            // CLI assertions
            Assertion::CliRun(cmd, args) => {
                if args.is_empty() {
                    format!("run \"{}\"", cmd)
                } else {
                    format!("run \"{}\" with args {:?}", cmd, args)
                }
            }
            Assertion::CliExitCode(code) => {
                if *code == 0 {
                    "exits successfully".to_string()
                } else {
                    format!("exits with code {}", code)
                }
            }
            Assertion::CliOutputContains(text) => format!("output contains \"{}\"", text),
            Assertion::CliOutputNotContains(text) => format!("output not contains \"{}\"", text),
            Assertion::CliErrorContains(text) => format!("error output contains \"{}\"", text),
            // Code quality assertions
            Assertion::CodeQualityPassed => "code is valid".to_string(),
            Assertion::CodeQualityNoErrors => "no syntax errors".to_string(),
            Assertion::CodeQualityNoWarnings => "no lint warnings".to_string(),
            Assertion::CodeQualityErrorCount(count) => format!("error count is {}", count),
        }
    }
}

/// Run assertions using the IAL term rewriting engine
///
/// This is the new path that uses the IAL engine for assertion execution.
/// It provides better extensibility through vocabulary-based term resolution.
pub fn run_assertions_ial(
    assertions: &[Assertion],
    vocab: &Vocabulary,
    status: u16,
    body: &str,
    headers: &HashMap<String, String>,
) -> Vec<AssertionResult> {
    // Build the IAL context with response data
    let mut ctx = IalContext::new();
    ctx.set("response.status", ial::Value::Number(status as f64));
    ctx.set("response.body", ial::Value::String(body.to_string()));

    // Add headers to context
    for (name, value) in headers {
        ctx.set(
            &format!("response.headers.{}", name.to_lowercase()),
            ial::Value::String(value.clone()),
        );
    }

    // Process each assertion through the IAL engine
    assertions
        .iter()
        .map(|assertion| {
            let term_text = assertion.to_ial_term();
            let term = Term::new(&term_text);

            // Try to resolve through IAL vocabulary
            match ial::resolve(&term, vocab) {
                Ok(primitives) => {
                    // Execute all primitives and collect results
                    let mut all_passed = true;
                    let mut message = None;

                    for primitive in &primitives {
                        let result = ial::execute::execute_check(primitive, &ctx);
                        if !result.passed {
                            all_passed = false;
                            message = result.message.clone();
                            break;
                        }
                    }

                    AssertionResult {
                        assertion: assertion.clone(),
                        passed: all_passed,
                        actual: ctx.get("response.body").map(|v| match v {
                            ial::Value::String(s) => {
                                if s.len() > 100 {
                                    format!("{}...", &s[..100])
                                } else {
                                    s.clone()
                                }
                            }
                            _ => v.to_string(),
                        }),
                        message,
                    }
                }
                Err(e) => {
                    // Fall back to direct execution if term not found in vocabulary
                    run_assertion_legacy(assertion, status, body, headers).unwrap_or_else(|| {
                        AssertionResult {
                            assertion: assertion.clone(),
                            passed: false,
                            actual: None,
                            message: Some(format!("IAL resolution error: {}", e)),
                        }
                    })
                }
            }
        })
        .collect()
}

/// Direct assertion execution (fallback when IAL vocabulary lookup fails)
fn run_assertion_legacy(
    assertion: &Assertion,
    status: u16,
    body: &str,
    headers: &HashMap<String, String>,
) -> Option<AssertionResult> {
    Some(match assertion {
        Assertion::Status(expected) => {
            let passed = status == *expected;
            AssertionResult {
                assertion: assertion.clone(),
                passed,
                actual: Some(status.to_string()),
                message: if passed {
                    None
                } else {
                    Some(format!("Expected status {}, got {}", expected, status))
                },
            }
        }
        Assertion::BodyContains(text) => {
            let passed = body.contains(text);
            AssertionResult {
                assertion: assertion.clone(),
                passed,
                actual: Some(truncate_body(body)),
                message: if passed {
                    None
                } else {
                    Some(format!("Body does not contain \"{}\"", text))
                },
            }
        }
        Assertion::BodyNotContains(text) => {
            let passed = !body.contains(text);
            AssertionResult {
                assertion: assertion.clone(),
                passed,
                actual: Some(truncate_body(body)),
                message: if passed {
                    None
                } else {
                    Some(format!("Body contains \"{}\" (should not)", text))
                },
            }
        }
        Assertion::BodyMatches(pattern) => {
            let passed = match regex::Regex::new(pattern) {
                Ok(re) => re.is_match(body),
                Err(_) => false,
            };
            AssertionResult {
                assertion: assertion.clone(),
                passed,
                actual: Some(truncate_body(body)),
                message: if passed {
                    None
                } else {
                    Some(format!("Body does not match pattern \"{}\"", pattern))
                },
            }
        }
        Assertion::HeaderContains(header_name, expected_value) => {
            let actual = headers.get(&header_name.to_lowercase());
            let passed = actual.map(|v| v.contains(expected_value)).unwrap_or(false);
            AssertionResult {
                assertion: assertion.clone(),
                passed,
                actual: actual.cloned(),
                message: if passed {
                    None
                } else {
                    Some(format!(
                        "Header \"{}\" does not contain \"{}\"",
                        header_name, expected_value
                    ))
                },
            }
        }
        Assertion::JsonPathExists(path) => {
            let passed = json_path_exists(body, path);
            AssertionResult {
                assertion: assertion.clone(),
                passed,
                actual: Some(truncate_body(body)),
                message: if passed {
                    None
                } else {
                    Some(format!("JSON path \"{}\" does not exist", path))
                },
            }
        }
        Assertion::JsonPathEquals(path, expected) => {
            let actual_value = json_path_value(body, path);
            let passed = actual_value
                .as_ref()
                .map(|v| v == expected)
                .unwrap_or(false);
            AssertionResult {
                assertion: assertion.clone(),
                passed,
                actual: actual_value.clone(),
                message: if passed {
                    None
                } else {
                    Some(format!(
                        "JSON path \"{}\" expected \"{}\", got {:?}",
                        path, expected, actual_value
                    ))
                },
            }
        }
        Assertion::RedirectsTo(expected_path) => {
            let is_redirect = (300..400).contains(&status);
            let location = headers.get("location");
            let passed =
                is_redirect && location.map(|l| l.contains(expected_path)).unwrap_or(false);
            AssertionResult {
                assertion: assertion.clone(),
                passed,
                actual: location.cloned(),
                message: if passed {
                    None
                } else if !is_redirect {
                    Some(format!("Expected redirect, got status {}", status))
                } else {
                    Some(format!(
                        "Expected redirect to \"{}\", got {:?}",
                        expected_path, location
                    ))
                },
            }
        }
        // CLI assertions are not applicable in HTTP context
        Assertion::CliRun(_, _)
        | Assertion::CliExitCode(_)
        | Assertion::CliOutputContains(_)
        | Assertion::CliOutputNotContains(_)
        | Assertion::CliErrorContains(_) => {
            return None;
        }
        // Code quality assertions are not applicable in HTTP context
        Assertion::CodeQualityPassed
        | Assertion::CodeQualityNoErrors
        | Assertion::CodeQualityNoWarnings
        | Assertion::CodeQualityErrorCount(_) => {
            return None;
        }
    })
}

/// Truncate body for display
fn truncate_body(body: &str) -> String {
    if body.len() > 100 {
        format!("{}...", &body[..100])
    } else {
        body.to_string()
    }
}

/// A single test case (HTTP request + assertions)
#[derive(Debug, Clone, Serialize)]
pub struct TestCase {
    pub method: String,
    pub path: String,
    pub body: Option<String>,
    pub assertions: Vec<Assertion>,
    /// Preconditions that should be verified before running the test
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub preconditions: Vec<Assertion>,
    /// Original scenario name if this test was generated from a scenario
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scenario_name: Option<String>,
}

/// A feature/requirement with tests and scenarios
#[derive(Debug, Clone, Serialize)]
pub struct Feature {
    pub id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    /// Traditional test cases (technical format)
    pub tests: Vec<TestCase>,
    /// Natural language scenarios (IAL format)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scenarios: Vec<Scenario>,
}

/// Parsed intent file with optional glossary
#[derive(Debug, Serialize)]
pub struct IntentFile {
    pub features: Vec<Feature>,
    pub source_path: String,
    /// IAL glossary (if present)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glossary: Option<Glossary>,
    /// Reusable components (IAL Components)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<Component>,
}

/// Result of running a single assertion
#[derive(Debug, Clone, Serialize)]
pub struct AssertionResult {
    pub assertion: Assertion,
    pub passed: bool,
    pub actual: Option<String>,
    pub message: Option<String>,
}

/// Result of running a test case
#[derive(Debug, Clone, Serialize)]
pub struct TestResult {
    pub test: TestCase,
    pub passed: bool,
    pub assertion_results: Vec<AssertionResult>,
    pub response_status: u16,
    pub response_body: String,
    pub response_headers: HashMap<String, String>,
}

/// Result of running a feature
#[derive(Debug, Serialize)]
pub struct FeatureResult {
    pub feature: Feature,
    pub passed: bool,
    pub test_results: Vec<TestResult>,
    pub has_implementation: bool, // Whether any @implements annotation links to this feature
}

/// Result of running all intent checks
#[derive(Debug)]
pub struct IntentCheckResult {
    pub intent_file: String,
    pub features_passed: usize,
    pub features_failed: usize,
    pub assertions_passed: usize,
    pub assertions_failed: usize,
    pub feature_results: Vec<FeatureResult>,
}

/// An annotation found in source code linking to intent
#[derive(Debug, Clone)]
pub struct Annotation {
    pub kind: AnnotationKind,
    pub id: String,
    pub file: String,
    pub line: usize,
    pub function_name: Option<String>,
}

/// Types of annotations
#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationKind {
    /// @implements: feature.X - This code implements a feature
    Implements,
    /// @supports: constraint.X - This code supports a constraint
    Supports,
    /// @utility - Utility function
    Utility,
    /// @internal - Internal implementation detail
    Internal,
    /// @infrastructure - Infrastructure code
    Infrastructure,
}

/// Coverage report showing which features have implementations
#[derive(Debug)]
pub struct CoverageReport {
    pub intent_file: String,
    pub source_files: Vec<String>,
    pub features: Vec<FeatureCoverage>,
    pub total_features: usize,
    pub covered_features: usize,
    pub coverage_percent: f64,
}

/// Coverage for a single feature
#[derive(Debug)]
pub struct FeatureCoverage {
    pub feature_id: String,
    pub feature_name: String,
    pub covered: bool,
    pub implementations: Vec<Annotation>,
}

impl IntentFile {
    /// Parse an intent file from a path
    pub fn parse(path: &Path) -> Result<Self, IntentError> {
        let content = fs::read_to_string(path)
            .map_err(|e| IntentError::RuntimeError(format!("Failed to read intent file: {}", e)))?;

        Self::parse_content(&content, path.to_string_lossy().to_string())
    }

    /// Parse intent file content
    pub fn parse_content(content: &str, source_path: String) -> Result<Self, IntentError> {
        let mut features = Vec::new();
        let mut components = Vec::new();
        let mut glossary = Glossary::new();
        let mut has_glossary = false;
        let mut current_feature: Option<Feature> = None;
        let mut current_component: Option<Component> = None;
        let mut current_test: Option<TestCase> = None;
        let mut current_scenario: Option<Scenario> = None;
        let mut in_assertions = false;
        let mut in_glossary = false;
        let mut in_component_inherent = false;
        let mut _in_glossary_bindings = false;
        // Technical bindings parsing state
        let mut current_binding_term: Option<(String, TechnicalBinding)> = None;
        let mut in_binding_assert_list = false;

        for line in content.lines() {
            let trimmed = line.trim();

            // Skip empty lines
            if trimmed.is_empty() {
                continue;
            }

            // Skip pure comments (but not markdown headers)
            if trimmed.starts_with('#') && !trimmed.starts_with("##") {
                continue;
            }

            // Section separator (---)
            if trimmed == "---" {
                // Save current component if any
                if let Some(mut comp) = current_component.take() {
                    if let Some(scenario) = current_scenario.take() {
                        comp.scenarios.push(scenario);
                    }
                    components.push(comp);
                }
                // Save any pending binding term
                if let Some((term, binding)) = current_binding_term.take() {
                    glossary.set_binding(&term, binding);
                }
                in_glossary = false;
                in_component_inherent = false;
                _in_glossary_bindings = false;
                in_binding_assert_list = false;
                continue;
            }

            // Glossary section header: ## Glossary
            if trimmed.starts_with("## Glossary") {
                has_glossary = true;
                if trimmed.contains("[Technical Bindings]") {
                    _in_glossary_bindings = true;
                } else {
                    in_glossary = true;
                    _in_glossary_bindings = false;
                }
                continue;
            }

            // Parse glossary table rows: | term | meaning | or | term | type | meaning |
            if in_glossary && trimmed.starts_with('|') && !trimmed.contains("---") {
                let parts: Vec<&str> = trimmed.split('|').collect();
                // Filter out empty parts from leading/trailing pipes
                let non_empty: Vec<&str> = parts
                    .iter()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();

                if non_empty.len() >= 3 {
                    // 3-column format: | Term | Type | Means |
                    let term = non_empty[0];
                    let term_type = non_empty[1];
                    let meaning = non_empty[2];

                    // Skip header row
                    if term != "Term" && term != "---" && !term.is_empty() {
                        let type_opt = if term_type.is_empty() || term_type == "Type" {
                            None
                        } else {
                            Some(term_type.to_string())
                        };
                        glossary.add_term_with_type(
                            term.to_string(),
                            meaning.to_string(),
                            type_opt,
                        );
                    }
                } else if non_empty.len() >= 2 {
                    // 2-column format (no Type): | Term | Means |
                    let term = non_empty[0];
                    let meaning = non_empty[1];

                    // Skip header row
                    if term != "Term" && term != "---" && !term.is_empty() {
                        glossary.add_term(term.to_string(), meaning.to_string());
                    }
                }
                continue;
            }

            // Parse technical bindings section: YAML-like syntax
            // Format:
            //   term name:
            //     setup: SQL or code
            //     action: METHOD /path
            //     body: { "json": "data" }
            //     precondition: condition
            //     assert:
            //       - assertion 1
            //       - assertion 2
            //     path: /some/path
            if _in_glossary_bindings {
                // Term declaration (not indented, ends with :)
                if !line.starts_with(' ') && !line.starts_with('\t') && trimmed.ends_with(':') {
                    // Save previous binding term
                    if let Some((term, binding)) = current_binding_term.take() {
                        glossary.set_binding(&term, binding);
                    }

                    let term = trimmed.trim_end_matches(':').to_string();
                    current_binding_term = Some((term, TechnicalBinding::default()));
                    in_binding_assert_list = false;
                    continue;
                }

                // Binding property (indented)
                if let Some((ref _term, ref mut binding)) = current_binding_term {
                    // Check for assert list items
                    if in_binding_assert_list && trimmed.starts_with('-') {
                        let assertion = trimmed.trim_start_matches('-').trim().to_string();
                        binding.assert.push(assertion);
                        continue;
                    }

                    // Property declarations
                    if trimmed.starts_with("setup:") {
                        let value = trimmed.trim_start_matches("setup:").trim();
                        // Handle multi-line with | indicator
                        if value == "|" || value.is_empty() {
                            // Multi-line will be collected on subsequent lines
                            binding.setup = Some(String::new());
                        } else {
                            binding.setup = Some(value.to_string());
                        }
                        in_binding_assert_list = false;
                        continue;
                    }

                    if trimmed.starts_with("action:") {
                        let value = trimmed.trim_start_matches("action:").trim();
                        binding.action = Some(value.to_string());
                        in_binding_assert_list = false;
                        continue;
                    }

                    if trimmed.starts_with("body:") {
                        let value = trimmed.trim_start_matches("body:").trim();
                        binding.body = Some(value.to_string());
                        in_binding_assert_list = false;
                        continue;
                    }

                    if trimmed.starts_with("precondition:") {
                        let value = trimmed.trim_start_matches("precondition:").trim();
                        binding.precondition = Some(value.to_string());
                        in_binding_assert_list = false;
                        continue;
                    }

                    if trimmed.starts_with("path:") {
                        let value = trimmed.trim_start_matches("path:").trim();
                        binding.path = Some(value.to_string());
                        in_binding_assert_list = false;
                        continue;
                    }

                    if trimmed.starts_with("assert:") {
                        // Start of assert list - items follow with -
                        in_binding_assert_list = true;
                        continue;
                    }

                    // Multi-line content (continuation of setup with |)
                    if binding
                        .setup
                        .as_ref()
                        .map(|s| s.is_empty())
                        .unwrap_or(false)
                    {
                        binding.setup = Some(trimmed.to_string());
                        continue;
                    }
                }
                continue;
            }

            // Feature declaration
            if trimmed.starts_with("Feature:") {
                // Save previous component
                if let Some(mut comp) = current_component.take() {
                    if let Some(scenario) = current_scenario.take() {
                        comp.scenarios.push(scenario);
                    }
                    components.push(comp);
                }
                // Save previous feature
                if let Some(mut feat) = current_feature.take() {
                    if let Some(test) = current_test.take() {
                        feat.tests.push(test);
                    }
                    if let Some(scenario) = current_scenario.take() {
                        feat.scenarios.push(scenario);
                    }
                    features.push(feat);
                }

                let name = trimmed.trim_start_matches("Feature:").trim().to_string();
                current_feature = Some(Feature {
                    id: None,
                    name,
                    description: None,
                    tests: Vec::new(),
                    scenarios: Vec::new(),
                });
                current_test = None;
                current_scenario = None;
                current_component = None;
                in_assertions = false;
                in_glossary = false;
                in_component_inherent = false;
                _in_glossary_bindings = false;
                continue;
            }

            // Component declaration
            if trimmed.starts_with("Component:") {
                // Save previous component
                if let Some(mut comp) = current_component.take() {
                    if let Some(scenario) = current_scenario.take() {
                        comp.scenarios.push(scenario);
                    }
                    components.push(comp);
                }
                // Save previous feature
                if let Some(mut feat) = current_feature.take() {
                    if let Some(test) = current_test.take() {
                        feat.tests.push(test);
                    }
                    if let Some(scenario) = current_scenario.take() {
                        feat.scenarios.push(scenario);
                    }
                    features.push(feat);
                }

                let name = trimmed.trim_start_matches("Component:").trim().to_string();
                current_component = Some(Component {
                    id: String::new(),
                    name,
                    description: None,
                    parameters: Vec::new(),
                    inherent_behavior: Vec::new(),
                    scenarios: Vec::new(),
                });
                current_test = None;
                current_scenario = None;
                current_feature = None;
                in_assertions = false;
                in_glossary = false;
                in_component_inherent = false;
                _in_glossary_bindings = false;
                continue;
            }

            // Inside a component
            if let Some(ref mut component) = current_component {
                // Component ID
                if trimmed.starts_with("id:") {
                    let id = trimmed.trim_start_matches("id:").trim();
                    component.id = id.to_string();
                    continue;
                }

                // Description - only for component if not inside a scenario
                if trimmed.starts_with("description:") && current_scenario.is_none() {
                    let desc = trimmed.trim_start_matches("description:").trim();
                    let desc = desc.trim_matches('"').to_string();
                    component.description = Some(desc);
                    continue;
                }

                // Parameters
                if trimmed.starts_with("parameters:") {
                    let params_str = trimmed.trim_start_matches("parameters:").trim();
                    // Parse [param1, param2] or [param1] format
                    let params_str = params_str.trim_matches(|c| c == '[' || c == ']');
                    component.parameters = params_str
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    continue;
                }

                // Inherent Behavior section
                if trimmed.starts_with("Inherent Behavior:")
                    || trimmed.starts_with("inherent_behavior:")
                {
                    in_component_inherent = true;
                    continue;
                }

                // Scenario declaration inside component
                if trimmed.starts_with("Scenario:") {
                    in_component_inherent = false;
                    // Save previous scenario
                    if let Some(scenario) = current_scenario.take() {
                        component.scenarios.push(scenario);
                    }

                    let name = trimmed.trim_start_matches("Scenario:").trim().to_string();
                    current_scenario = Some(Scenario {
                        name,
                        description: None,
                        given_clause: None,
                        when_clause: String::new(),
                        outcomes: Vec::new(),
                        resolved_test: None,
                        component_refs: Vec::new(),
                    });
                    continue;
                }

                // Inside component scenario
                if let Some(ref mut scenario) = current_scenario {
                    // Description
                    if trimmed.starts_with("description:") {
                        let desc = trimmed.trim_start_matches("description:").trim();
                        let desc = desc.trim_matches('"');
                        scenario.description = Some(desc.to_string());
                        continue;
                    }

                    // Given clause
                    if trimmed.starts_with("Given ") || trimmed.starts_with("given ") {
                        scenario.given_clause = Some(trimmed[6..].to_string());
                        continue;
                    }

                    // When clause
                    if trimmed.starts_with("When ") || trimmed.starts_with("when ") {
                        scenario.when_clause = trimmed[5..].to_string();
                        continue;
                    }

                    // Outcome clause
                    if trimmed.starts_with("→") || trimmed.starts_with("->") {
                        let outcome = trimmed
                            .trim_start_matches("→")
                            .trim_start_matches("->")
                            .trim()
                            .to_string();
                        scenario.outcomes.push(outcome);
                        continue;
                    }
                }

                // Inherent behavior outcomes
                if in_component_inherent {
                    if trimmed.starts_with("→") || trimmed.starts_with("->") {
                        let outcome = trimmed
                            .trim_start_matches("→")
                            .trim_start_matches("->")
                            .trim()
                            .to_string();
                        component.inherent_behavior.push(outcome);
                        continue;
                    }
                }

                continue;
            }

            // Inside a feature
            if let Some(ref mut feature) = current_feature {
                // Feature ID
                if trimmed.starts_with("id:") {
                    let id = trimmed.trim_start_matches("id:").trim();
                    feature.id = Some(id.to_string());
                    continue;
                }

                // Description - only for feature if not inside a scenario
                if trimmed.starts_with("description:") && current_scenario.is_none() {
                    let desc = trimmed.trim_start_matches("description:").trim();
                    // Remove surrounding quotes if present
                    let desc = desc.trim_matches('"').to_string();
                    feature.description = Some(desc);
                    continue;
                }

                // Scenario declaration (IAL format)
                if trimmed.starts_with("Scenario:") {
                    // Save previous scenario
                    if let Some(scenario) = current_scenario.take() {
                        feature.scenarios.push(scenario);
                    }
                    // Save previous test if any
                    if let Some(test) = current_test.take() {
                        feature.tests.push(test);
                    }

                    let name = trimmed.trim_start_matches("Scenario:").trim().to_string();
                    current_scenario = Some(Scenario {
                        name,
                        description: None,
                        given_clause: None,
                        when_clause: String::new(),
                        outcomes: Vec::new(),
                        resolved_test: None,
                        component_refs: Vec::new(),
                    });
                    in_assertions = false;
                    continue;
                }

                // Inside a scenario
                if let Some(ref mut scenario) = current_scenario {
                    // Description
                    if trimmed.starts_with("description:") {
                        let desc = trimmed.trim_start_matches("description:").trim();
                        let desc = desc.trim_matches('"');
                        scenario.description = Some(desc.to_string());
                        continue;
                    }

                    // Given clause
                    if trimmed.starts_with("Given ") || trimmed.starts_with("given ") {
                        scenario.given_clause = Some(trimmed[6..].to_string());
                        continue;
                    }

                    // When clause
                    if trimmed.starts_with("When ") || trimmed.starts_with("when ") {
                        scenario.when_clause = trimmed[5..].to_string();
                        continue;
                    }

                    // Outcome clause (→ or ->)
                    if trimmed.starts_with("→") || trimmed.starts_with("->") {
                        let outcome = trimmed
                            .trim_start_matches("→")
                            .trim_start_matches("->")
                            .trim()
                            .to_string();
                        scenario.outcomes.push(outcome);
                        continue;
                    }
                }

                // Test section start
                if trimmed == "test:" {
                    // Save any current scenario first
                    if let Some(scenario) = current_scenario.take() {
                        feature.scenarios.push(scenario);
                    }
                    in_assertions = false;
                    continue;
                }

                // Request line (starts a new test case)
                if trimmed.starts_with("- request:") || trimmed.starts_with("request:") {
                    // Save previous test
                    if let Some(test) = current_test.take() {
                        feature.tests.push(test);
                    }

                    let request_str = if trimmed.starts_with("- request:") {
                        trimmed.trim_start_matches("- request:").trim()
                    } else {
                        trimmed.trim_start_matches("request:").trim()
                    };

                    // Parse "METHOD /path"
                    let parts: Vec<&str> = request_str.splitn(2, ' ').collect();
                    let method = parts.first().unwrap_or(&"GET").to_string();
                    let path = parts.get(1).unwrap_or(&"/").to_string();

                    current_test = Some(TestCase {
                        method,
                        path,
                        body: None,
                        assertions: Vec::new(),
                        preconditions: Vec::new(),
                        scenario_name: None,
                    });
                    in_assertions = false;
                    continue;
                }

                // Assert section
                if trimmed == "assert:" {
                    in_assertions = true;
                    continue;
                }

                // Assertion lines
                if in_assertions {
                    if let Some(ref mut test) = current_test {
                        if let Some(assertion) = Self::parse_assertion(trimmed) {
                            test.assertions.push(assertion);
                        }
                    }
                    continue;
                }

                // Body for POST requests
                if trimmed.starts_with("body:") {
                    if let Some(ref mut test) = current_test {
                        let body = trimmed.trim_start_matches("body:").trim();
                        let body = body.trim_matches('"').to_string();
                        test.body = Some(body);
                    }
                    continue;
                }
            }
        }

        // Save final feature, test, scenario, or component
        if let Some(mut feat) = current_feature.take() {
            if let Some(test) = current_test.take() {
                feat.tests.push(test);
            }
            if let Some(scenario) = current_scenario.take() {
                feat.scenarios.push(scenario);
            }
            features.push(feat);
        } else if let Some(mut comp) = current_component.take() {
            if let Some(scenario) = current_scenario.take() {
                comp.scenarios.push(scenario);
            }
            components.push(comp);
        }

        // Save any remaining binding term
        if let Some((term, binding)) = current_binding_term.take() {
            glossary.set_binding(&term, binding);
        }

        Ok(IntentFile {
            features,
            source_path,
            glossary: if has_glossary { Some(glossary) } else { None },
            components,
        })
    }

    /// Parse a single assertion line
    fn parse_assertion(line: &str) -> Option<Assertion> {
        let line = line.trim().trim_start_matches('-').trim();

        // status: 200
        if line.starts_with("status:") {
            let code_str = line.trim_start_matches("status:").trim();
            if let Ok(code) = code_str.parse::<u16>() {
                return Some(Assertion::Status(code));
            }
        }

        // json path "field" exists
        if line.starts_with("json path") && line.contains("exists") {
            let rest = line.trim_start_matches("json path").trim();
            if let Some(idx) = rest.find("exists") {
                let path = rest[..idx].trim().trim_matches('"').to_string();
                return Some(Assertion::JsonPathExists(path));
            }
        }

        // json path "field" == "value" or json path "field" = "value"
        if line.starts_with("json path") && (line.contains("==") || line.contains(" = ")) {
            let rest = line.trim_start_matches("json path").trim();
            let delimiter = if rest.contains("==") { "==" } else { " = " };
            if let Some(idx) = rest.find(delimiter) {
                let path = rest[..idx].trim().trim_matches('"').to_string();
                let value = rest[idx + delimiter.len()..]
                    .trim()
                    .trim_matches('"')
                    .to_string();
                return Some(Assertion::JsonPathEquals(path, value));
            }
        }

        // redirects to "/path"
        if line.starts_with("redirects to") {
            let path = line.trim_start_matches("redirects to").trim();
            let path = path.trim_matches('"').to_string();
            return Some(Assertion::RedirectsTo(path));
        }

        // body contains "text"
        if line.starts_with("body contains") {
            let text = line.trim_start_matches("body contains").trim();
            let text = text.trim_matches('"').to_string();
            return Some(Assertion::BodyContains(text));
        }

        // body not contains "text"
        if line.starts_with("body not contains") {
            let text = line.trim_start_matches("body not contains").trim();
            let text = text.trim_matches('"').to_string();
            return Some(Assertion::BodyNotContains(text));
        }

        // body matches r"pattern" or body matches "pattern"
        if line.starts_with("body matches") {
            let pattern = line.trim_start_matches("body matches").trim();
            // Handle raw string r"..." or regular "..."
            let pattern = if pattern.starts_with("r\"") {
                pattern.trim_start_matches("r\"").trim_end_matches('"')
            } else {
                pattern.trim_matches('"')
            };
            return Some(Assertion::BodyMatches(pattern.to_string()));
        }

        // header "Name" contains "value"
        if line.starts_with("header") {
            // header "Content-Type" contains "text/html"
            let rest = line.trim_start_matches("header").trim();
            if let Some(idx) = rest.find("contains") {
                let header_name = rest[..idx].trim().trim_matches('"').to_string();
                let value = rest[idx..]
                    .trim_start_matches("contains")
                    .trim()
                    .trim_matches('"')
                    .to_string();
                return Some(Assertion::HeaderContains(header_name, value));
            }
        }

        None
    }
}

/// Run intent checks against an NTNT file
pub fn run_intent_check(
    ntnt_path: &Path,
    intent_path: &Path,
    port: u16,
    _verbose: bool,
) -> Result<IntentCheckResult, IntentError> {
    // Parse intent file
    let intent = IntentFile::parse(intent_path)?;

    // Read NTNT source
    let source = fs::read_to_string(ntnt_path)
        .map_err(|e| IntentError::RuntimeError(format!("Failed to read NTNT file: {}", e)))?;

    // Count total tests
    let total_tests: usize = intent.features.iter().map(|f| f.tests.len()).sum();

    if total_tests == 0 {
        return Err(IntentError::RuntimeError(
            "No tests found in intent file".to_string(),
        ));
    }

    // Setup for server
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_clone = shutdown_flag.clone();

    // Collect all tests to run
    let mut all_tests: Vec<(usize, usize, TestCase)> = Vec::new();
    for (fi, feature) in intent.features.iter().enumerate() {
        for (ti, test) in feature.tests.iter().enumerate() {
            all_tests.push((fi, ti, test.clone()));
        }
    }

    let all_tests_clone = all_tests.clone();
    let results: Arc<std::sync::Mutex<Vec<(usize, usize, TestResult)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let results_clone = results.clone();

    // Spawn thread to run tests
    let test_handle = thread::spawn(move || {
        // Wait for server to start
        thread::sleep(Duration::from_millis(300));

        for (fi, ti, test) in all_tests_clone {
            let result = run_single_test(&test, port);
            results_clone.lock().unwrap().push((fi, ti, result));
        }

        // Signal shutdown
        shutdown_flag_clone.store(true, Ordering::SeqCst);
    });

    // Start the server
    let mut interpreter = Interpreter::new();
    interpreter.set_test_mode(port, total_tests, shutdown_flag.clone());

    let lexer = Lexer::new(&source);
    let tokens: Vec<_> = lexer.collect();
    let mut parser = IntentParser::new(tokens);
    let ast = parser.parse()?;

    // Run (will exit when shutdown_flag is set)
    let _ = interpreter.eval(&ast);

    // Wait for test thread
    test_handle.join().ok();

    // Collect results
    let test_results = results.lock().unwrap();

    // Parse annotations from source to check for implementations
    let annotations = parse_annotations(&source, &ntnt_path.to_string_lossy());

    // Build feature results
    let mut feature_results: Vec<FeatureResult> = intent
        .features
        .iter()
        .map(|f| {
            let feature_id =
                f.id.clone()
                    .unwrap_or_else(|| f.name.to_lowercase().replace(' ', "_"));

            // Check if any annotation implements this feature
            let has_impl = annotations.iter().any(|a| {
                a.kind == AnnotationKind::Implements
                    && (a.id == feature_id || a.id == format!("feature.{}", feature_id))
            });

            FeatureResult {
                feature: f.clone(),
                passed: true,
                test_results: Vec::new(),
                has_implementation: has_impl,
            }
        })
        .collect();

    for (fi, _ti, result) in test_results.iter() {
        if !result.passed {
            feature_results[*fi].passed = false;
        }
        feature_results[*fi].test_results.push(result.clone());
    }

    // Calculate totals
    let mut features_passed = 0;
    let mut features_failed = 0;
    let mut assertions_passed = 0;
    let mut assertions_failed = 0;

    for fr in &feature_results {
        if fr.passed {
            features_passed += 1;
        } else {
            features_failed += 1;
        }
        for tr in &fr.test_results {
            for ar in &tr.assertion_results {
                if ar.passed {
                    assertions_passed += 1;
                } else {
                    assertions_failed += 1;
                }
            }
        }
    }

    Ok(IntentCheckResult {
        intent_file: intent.source_path,
        features_passed,
        features_failed,
        assertions_passed,
        assertions_failed,
        feature_results,
    })
}

/// Run a code quality test (lint/validate) without HTTP
fn run_code_quality_test(test: &TestCase) -> TestResult {
    use crate::ial::execute::{execute, Context as ExecuteContext};
    use crate::ial::primitives::Primitive;

    // Create IAL context and run code quality check
    let mut ctx = ExecuteContext::new();

    // Run the CodeQuality primitive to populate context
    let primitive = Primitive::CodeQuality {
        file: if test.path == "." {
            None
        } else {
            Some(test.path.clone())
        },
        lint: true,
        validate: true,
    };

    // Execute the code quality check (port is ignored for CodeQuality)
    let _exec_result = execute(&primitive, &mut ctx, 0);

    // Now run assertions against the populated context
    // We need to convert our Assertion enum to IAL checks and evaluate them
    let mut assertion_results = Vec::new();
    let mut all_passed = true;

    for assertion in &test.assertions {
        let (passed, actual, message) = evaluate_code_quality_assertion(assertion, &ctx);

        if !passed {
            all_passed = false;
        }

        assertion_results.push(AssertionResult {
            assertion: assertion.clone(),
            passed,
            actual,
            message,
        });
    }

    // Extract quality info from context for the response body
    let passed_value = ctx.get("code.quality.passed");
    let error_count = ctx.get("code.quality.error_count");
    let warning_count = ctx.get("code.quality.warning_count");
    let files_checked = ctx.get("code.quality.files_checked");

    let response_body = format!(
        "{{\"passed\": {}, \"error_count\": {}, \"warning_count\": {}, \"files_checked\": {}}}",
        passed_value.map_or("null".to_string(), |v| format!("{:?}", v)),
        error_count.map_or("null".to_string(), |v| format!("{:?}", v)),
        warning_count.map_or("null".to_string(), |v| format!("{:?}", v)),
        files_checked.map_or("null".to_string(), |v| format!("{:?}", v)),
    );

    TestResult {
        test: test.clone(),
        passed: all_passed,
        assertion_results,
        response_status: if all_passed { 200 } else { 500 },
        response_body,
        response_headers: HashMap::new(),
    }
}

/// Evaluate a code quality assertion against the IAL context
fn evaluate_code_quality_assertion(
    assertion: &Assertion,
    ctx: &ial::execute::Context,
) -> (bool, Option<String>, Option<String>) {
    use crate::ial::primitives::Value;

    match assertion {
        // Code quality assertions (primary)
        Assertion::CodeQualityPassed => {
            let passed = ctx
                .get("code.quality.passed")
                .map_or(false, |v| matches!(v, Value::Bool(true)));

            let actual = ctx.get("code.quality.passed").map(|v| format!("{:?}", v));

            (
                passed,
                actual,
                if passed {
                    None
                } else {
                    Some("Code quality checks did not pass".to_string())
                },
            )
        }

        Assertion::CodeQualityNoErrors => {
            let passed = ctx.get("code.quality.error_count").map_or(false, |v| {
                if let Value::Number(n) = v {
                    *n == 0.0
                } else {
                    false
                }
            });

            let actual = ctx
                .get("code.quality.error_count")
                .map(|v| format!("{:?}", v));

            (
                passed,
                actual,
                if passed {
                    None
                } else {
                    let error_count = ctx
                        .get("code.quality.error_count")
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(*n as u32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    Some(format!("Found {} syntax errors", error_count))
                },
            )
        }

        Assertion::CodeQualityNoWarnings => {
            let passed = ctx.get("code.quality.warning_count").map_or(true, |v| {
                if let Value::Number(n) = v {
                    *n == 0.0
                } else {
                    false
                }
            });

            let actual = ctx
                .get("code.quality.warning_count")
                .map(|v| format!("{:?}", v));

            (
                passed,
                actual,
                if passed {
                    None
                } else {
                    let warning_count = ctx
                        .get("code.quality.warning_count")
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(*n as u32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    Some(format!("Found {} lint warnings", warning_count))
                },
            )
        }

        Assertion::CodeQualityErrorCount(expected) => {
            let actual_count = ctx.get("code.quality.error_count").and_then(|v| {
                if let Value::Number(n) = v {
                    Some(*n as u32)
                } else {
                    None
                }
            });

            let passed = actual_count == Some(*expected);

            (
                passed,
                actual_count.map(|n| n.to_string()),
                if passed {
                    None
                } else {
                    Some(format!(
                        "Expected {} errors, found {}",
                        expected,
                        actual_count.unwrap_or(0)
                    ))
                },
            )
        }

        // Handle CLI assertions that map to code quality checks (backward compatibility)
        Assertion::CliExitCode(expected) => {
            // Map exit code to code.quality.passed
            let passed = if *expected == 0 {
                // Exit code 0 means lint passed
                ctx.get("code.quality.passed")
                    .map_or(false, |v| matches!(v, Value::Bool(true)))
            } else {
                // Non-zero exit code means lint failed
                ctx.get("code.quality.passed")
                    .map_or(false, |v| matches!(v, Value::Bool(false)))
            };

            let actual = ctx.get("code.quality.passed").map(|v| format!("{:?}", v));

            (
                passed,
                actual,
                if passed {
                    None
                } else {
                    Some(format!(
                        "Expected exit code {}, code quality {}",
                        expected,
                        if *expected == 0 { "failed" } else { "passed" }
                    ))
                },
            )
        }

        Assertion::CliOutputContains(text) => {
            // Check if error messages contain the text
            let errors = ctx.get("code.quality.errors");
            let passed = errors.map_or(false, |v| {
                if let Value::Array(arr) = v {
                    arr.iter().any(|e| {
                        if let Value::String(s) = e {
                            s.contains(text)
                        } else {
                            false
                        }
                    })
                } else {
                    false
                }
            });

            (
                passed,
                errors.map(|v| format!("{:?}", v)),
                if passed {
                    None
                } else {
                    Some(format!("Output did not contain '{}'", text))
                },
            )
        }

        Assertion::CliOutputNotContains(text) => {
            // Check that error messages don't contain the text
            let errors = ctx.get("code.quality.errors");
            let passed = errors.map_or(true, |v| {
                if let Value::Array(arr) = v {
                    !arr.iter().any(|e| {
                        if let Value::String(s) = e {
                            s.contains(text)
                        } else {
                            false
                        }
                    })
                } else {
                    true
                }
            });

            (
                passed,
                errors.map(|v| format!("{:?}", v)),
                if passed {
                    None
                } else {
                    Some(format!("Output contained '{}'", text))
                },
            )
        }

        // For other assertions, mark as not applicable to code quality
        _ => (
            true,
            None,
            Some("Assertion not applicable to code quality tests".to_string()),
        ),
    }
}

/// Run a single test case against the server
fn run_single_test(test: &TestCase, port: u16) -> TestResult {
    // Handle CODE_QUALITY tests separately (no HTTP needed)
    if test.method == "CODE_QUALITY" {
        return run_code_quality_test(test);
    }

    let path = if test.path.starts_with('/') {
        test.path.clone()
    } else {
        format!("/{}", test.path)
    };

    let body_content = test.body.clone().unwrap_or_default();
    let request = if body_content.is_empty() {
        format!(
            "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            test.method, path, port
        )
    } else {
        format!(
            "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            test.method, path, port, body_content.len(), body_content
        )
    };

    // Try to connect
    let mut attempts = 0;
    let max_attempts = 20;

    while attempts < max_attempts {
        #[allow(clippy::single_match)]
        match TcpStream::connect(format!("127.0.0.1:{}", port)) {
            Ok(mut stream) => {
                stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
                stream.set_write_timeout(Some(Duration::from_secs(5))).ok();

                if stream.write_all(request.as_bytes()).is_ok() {
                    let mut response = Vec::new();
                    let _ = stream.read_to_end(&mut response);

                    if !response.is_empty() {
                        let response_str = String::from_utf8_lossy(&response).to_string();
                        let parts: Vec<&str> = response_str.splitn(2, "\r\n\r\n").collect();
                        let headers_str = parts.first().unwrap_or(&"");
                        let body = parts.get(1).unwrap_or(&"").to_string();

                        // Parse status code
                        let status_code = headers_str
                            .lines()
                            .next()
                            .unwrap_or("")
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("0")
                            .parse::<u16>()
                            .unwrap_or(0);

                        // Parse headers
                        let mut headers = HashMap::new();
                        for line in headers_str.lines().skip(1) {
                            if let Some(idx) = line.find(':') {
                                let key = line[..idx].trim().to_lowercase();
                                let value = line[idx + 1..].trim().to_string();
                                headers.insert(key, value);
                            }
                        }

                        // Run assertions
                        let assertion_results =
                            run_assertions(&test.assertions, status_code, &body, &headers);
                        let all_passed = assertion_results.iter().all(|r| r.passed);

                        return TestResult {
                            test: test.clone(),
                            passed: all_passed,
                            assertion_results,
                            response_status: status_code,
                            response_body: body,
                            response_headers: headers,
                        };
                    }
                }
            }
            Err(_) => {}
        }
        attempts += 1;
        thread::sleep(Duration::from_millis(100));
    }

    // Connection failed
    TestResult {
        test: test.clone(),
        passed: false,
        assertion_results: vec![AssertionResult {
            assertion: Assertion::Status(0),
            passed: false,
            actual: None,
            message: Some("Connection failed".to_string()),
        }],
        response_status: 0,
        response_body: String::new(),
        response_headers: HashMap::new(),
    }
}

/// Run assertions against a response
fn run_assertions(
    assertions: &[Assertion],
    status: u16,
    body: &str,
    headers: &HashMap<String, String>,
) -> Vec<AssertionResult> {
    assertions
        .iter()
        .map(|assertion| match assertion {
            Assertion::Status(expected) => {
                let passed = status == *expected;
                AssertionResult {
                    assertion: assertion.clone(),
                    passed,
                    actual: Some(status.to_string()),
                    message: if passed {
                        None
                    } else {
                        Some(format!("Expected status {}, got {}", expected, status))
                    },
                }
            }
            Assertion::BodyContains(text) => {
                let passed = body.contains(text);
                AssertionResult {
                    assertion: assertion.clone(),
                    passed,
                    actual: Some(if body.len() > 100 {
                        format!("{}...", &body[..100])
                    } else {
                        body.to_string()
                    }),
                    message: if passed {
                        None
                    } else {
                        Some(format!("Body does not contain \"{}\"", text))
                    },
                }
            }
            Assertion::BodyNotContains(text) => {
                let passed = !body.contains(text);
                AssertionResult {
                    assertion: assertion.clone(),
                    passed,
                    actual: Some(if body.len() > 100 {
                        format!("{}...", &body[..100])
                    } else {
                        body.to_string()
                    }),
                    message: if passed {
                        None
                    } else {
                        Some(format!("Body contains \"{}\" (should not)", text))
                    },
                }
            }
            Assertion::BodyMatches(pattern) => {
                let passed = match regex::Regex::new(pattern) {
                    Ok(re) => re.is_match(body),
                    Err(_) => false,
                };
                AssertionResult {
                    assertion: assertion.clone(),
                    passed,
                    actual: Some(if body.len() > 100 {
                        format!("{}...", &body[..100])
                    } else {
                        body.to_string()
                    }),
                    message: if passed {
                        None
                    } else {
                        Some(format!("Body does not match pattern \"{}\"", pattern))
                    },
                }
            }
            Assertion::HeaderContains(header_name, expected_value) => {
                let actual = headers.get(&header_name.to_lowercase());
                let passed = actual.map(|v| v.contains(expected_value)).unwrap_or(false);
                AssertionResult {
                    assertion: assertion.clone(),
                    passed,
                    actual: actual.cloned(),
                    message: if passed {
                        None
                    } else {
                        Some(format!(
                            "Header \"{}\" does not contain \"{}\"",
                            header_name, expected_value
                        ))
                    },
                }
            }
            Assertion::JsonPathExists(path) => {
                let passed = json_path_exists(body, path);
                AssertionResult {
                    assertion: assertion.clone(),
                    passed,
                    actual: Some(if body.len() > 100 {
                        format!("{}...", &body[..100])
                    } else {
                        body.to_string()
                    }),
                    message: if passed {
                        None
                    } else {
                        Some(format!("JSON path \"{}\" does not exist", path))
                    },
                }
            }
            Assertion::JsonPathEquals(path, expected) => {
                let actual_value = json_path_value(body, path);
                let passed = actual_value
                    .as_ref()
                    .map(|v| v == expected)
                    .unwrap_or(false);
                AssertionResult {
                    assertion: assertion.clone(),
                    passed,
                    actual: actual_value.clone(),
                    message: if passed {
                        None
                    } else {
                        Some(format!(
                            "JSON path \"{}\" expected \"{}\", got {:?}",
                            path, expected, actual_value
                        ))
                    },
                }
            }
            Assertion::RedirectsTo(expected_path) => {
                let is_redirect = (300..400).contains(&status);
                let location = headers.get("location");
                let passed =
                    is_redirect && location.map(|l| l.contains(expected_path)).unwrap_or(false);
                AssertionResult {
                    assertion: assertion.clone(),
                    passed,
                    actual: location.cloned(),
                    message: if passed {
                        None
                    } else if !is_redirect {
                        Some(format!("Expected redirect, got status {}", status))
                    } else {
                        Some(format!(
                            "Expected redirect to \"{}\", got {:?}",
                            expected_path, location
                        ))
                    },
                }
            }
            // CLI assertions are not applicable in HTTP context
            Assertion::CliRun(_, _)
            | Assertion::CliExitCode(_)
            | Assertion::CliOutputContains(_)
            | Assertion::CliOutputNotContains(_)
            | Assertion::CliErrorContains(_) => AssertionResult {
                assertion: assertion.clone(),
                passed: false,
                actual: None,
                message: Some("CLI assertion not applicable in HTTP test".to_string()),
            },
            // Code quality assertions are not applicable in HTTP context
            Assertion::CodeQualityPassed
            | Assertion::CodeQualityNoErrors
            | Assertion::CodeQualityNoWarnings
            | Assertion::CodeQualityErrorCount(_) => AssertionResult {
                assertion: assertion.clone(),
                passed: false,
                actual: None,
                message: Some("Code quality assertion not applicable in HTTP test".to_string()),
            },
        })
        .collect()
}

/// Check if a JSON path exists in the body
fn json_path_exists(body: &str, path: &str) -> bool {
    // Simple JSON path implementation for MVP
    // Supports: "field", "field.nested", "field[0]"
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        navigate_json_path(&json, path).is_some()
    } else {
        false
    }
}

/// Get the value at a JSON path
fn json_path_value(body: &str, path: &str) -> Option<String> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        navigate_json_path(&json, path).map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Null => "null".to_string(),
            _ => v.to_string(),
        })
    } else {
        None
    }
}

/// Navigate a JSON value using a simple path notation
fn navigate_json_path<'a>(
    json: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut current = json;

    for part in path.split('.') {
        // Handle array index: field[0]
        if let Some(bracket_idx) = part.find('[') {
            let field = &part[..bracket_idx];
            let idx_str = &part[bracket_idx + 1..part.len() - 1];

            if !field.is_empty() {
                current = current.get(field)?;
            }

            if let Ok(idx) = idx_str.parse::<usize>() {
                current = current.get(idx)?;
            } else {
                return None;
            }
        } else {
            current = current.get(part)?;
        }
    }

    Some(current)
}

/// Results from running tests against a live server (for Intent Studio)
#[derive(Debug, Serialize)]
pub struct LiveTestResults {
    pub features: Vec<LiveFeatureResult>,
    pub components: Vec<LiveComponentResult>,
    pub total_assertions: usize,
    pub passed_assertions: usize,
    pub failed_assertions: usize,
    pub linked_features: usize,
    pub total_features: usize,
    /// Glossary terms if present (for IAL format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glossary: Option<Vec<GlossaryTermDisplay>>,
}

/// A glossary term for display in the UI
#[derive(Debug, Serialize)]
pub struct GlossaryTermDisplay {
    pub term: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub term_type: Option<String>,
    pub meaning: String,
    /// Technical binding (if defined)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<TechnicalBinding>,
}

/// Result for a single feature in live testing
#[derive(Debug, Serialize)]
pub struct LiveFeatureResult {
    pub feature_id: String,
    pub feature_name: String,
    pub description: Option<String>,
    pub passed: bool,
    pub tests: Vec<LiveTestResult>,
    /// Natural language scenarios (IAL format)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scenarios: Vec<LiveScenarioResult>,
    pub has_implementation: bool,
}

/// Live test result for a component
#[derive(Debug, Clone, Serialize)]
pub struct LiveComponentResult {
    pub component_id: String,
    pub component_name: String,
    pub description: String,
    /// Inherent behaviors that always apply to this component
    pub inherent_behavior: Vec<String>,
    pub passed: bool,
    pub scenarios: Vec<LiveScenarioResult>,
}

/// Result for a scenario in live testing (IAL format)
#[derive(Debug, Clone, Serialize)]
pub struct LiveScenarioResult {
    pub name: String,
    /// Optional description explaining why this scenario exists
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub given_clause: Option<String>,
    pub when_clause: String,
    pub outcomes: Vec<String>,
    /// Status: "pass", "fail", "warning", "pending", or "skip" (precondition not met)
    pub status: String,
    /// The test results from this scenario's resolved test
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_result: Option<LiveTestResult>,
    /// Outcomes that couldn't be resolved to assertions (warnings)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unresolved_outcomes: Vec<String>,
    /// Component references used in this scenario
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub component_refs: Vec<String>,
}

/// Result for a single test in live testing
#[derive(Debug, Clone, Serialize)]
pub struct LiveTestResult {
    pub method: String,
    pub path: String,
    pub passed: bool,
    pub assertions: Vec<LiveAssertionResult>,
    /// Precondition assertion results (verified before main test)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub preconditions: Vec<LiveAssertionResult>,
    /// Scenario name if this test came from a scenario
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scenario_name: Option<String>,
}

/// Result for a single assertion in live testing
#[derive(Debug, Clone, Serialize)]
pub struct LiveAssertionResult {
    pub assertion_text: String,
    pub passed: bool,
    pub message: Option<String>,
}

/// Run tests against an already-running server (no interpreter needed)
/// Returns results in a format suitable for JSON serialization and UI display
/// If source_files is provided, will check for @implements annotations across all files
pub fn run_tests_against_server(
    intent: &IntentFile,
    port: u16,
    source_files: &[(String, String)], // (path, content) pairs
) -> LiveTestResults {
    let mut feature_results = Vec::new();
    let mut total_assertions = 0;
    let mut passed_assertions = 0;
    let mut failed_assertions = 0;

    // Get the base directory from the intent file path for code quality checks
    let intent_base_dir = Path::new(&intent.source_path)
        .parent()
        .and_then(|p| p.to_str())
        .filter(|s| !s.is_empty());

    // Parse annotations from all source files
    let mut annotations: Vec<Annotation> = Vec::new();
    for (path, content) in source_files {
        annotations.extend(parse_annotations(content, path));
    }

    for feature in &intent.features {
        let feature_id = feature.id.clone().unwrap_or_else(|| "unknown".to_string());
        let mut test_results = Vec::new();
        let mut scenario_results = Vec::new();
        let mut feature_passed = true;

        // Check if any annotation implements this feature
        let has_impl = annotations.iter().any(|a| {
            a.kind == AnnotationKind::Implements
                && (a.id == feature_id || a.id == format!("feature.{}", feature_id))
        });

        // Process scenarios FIRST (IAL format) - these are the primary tests now
        // Resolve each scenario to a TestCase using the glossary
        if let Some(ref glossary) = intent.glossary {
            for scenario in &feature.scenarios {
                if let Some((resolved_test, unresolved_outcomes, component_refs)) = glossary
                    .resolve_scenario_with_base_dir(scenario, &intent.components, intent_base_dir)
                {
                    // First, check preconditions if any exist
                    let mut precondition_results = Vec::new();
                    let mut preconditions_passed = true;
                    let mut skip_test = false;

                    if !resolved_test.preconditions.is_empty() {
                        // Run precondition checks (typically on the same endpoint as the test)
                        let precondition_test = TestCase {
                            method: resolved_test.method.clone(),
                            path: resolved_test.path.clone(),
                            body: None,
                            assertions: resolved_test.preconditions.clone(),
                            preconditions: vec![],
                            scenario_name: None,
                        };

                        let precond_result = run_single_test(&precondition_test, port);

                        for ar in &precond_result.assertion_results {
                            let assertion_text = format_assertion(&ar.assertion);
                            precondition_results.push(LiveAssertionResult {
                                assertion_text,
                                passed: ar.passed,
                                message: ar.message.clone(),
                            });

                            if !ar.passed {
                                preconditions_passed = false;
                                // Precondition not met = SKIP test (not fail)
                                skip_test = true;
                            }
                        }
                    }

                    // If preconditions not met, skip the test
                    if skip_test {
                        // Add scenario result as skipped
                        scenario_results.push(LiveScenarioResult {
                            name: scenario.name.clone(),
                            description: scenario.description.clone(),
                            given_clause: scenario.given_clause.clone(),
                            when_clause: scenario.when_clause.clone(),
                            outcomes: scenario.outcomes.clone(),
                            status: "skip".to_string(),
                            test_result: Some(LiveTestResult {
                                method: resolved_test.method.clone(),
                                path: resolved_test.path.clone(),
                                passed: false,
                                assertions: vec![],
                                preconditions: precondition_results,
                                scenario_name: Some(scenario.name.clone()),
                            }),
                            unresolved_outcomes: vec![],
                            component_refs,
                        });
                        continue; // Skip to next scenario
                    }

                    // Execute the resolved test
                    let result = run_single_test(&resolved_test, port);
                    let mut assertion_results = Vec::new();
                    let mut test_passed = result.passed && preconditions_passed;

                    for ar in &result.assertion_results {
                        total_assertions += 1;
                        let assertion_text = format_assertion(&ar.assertion);

                        if ar.passed {
                            passed_assertions += 1;
                        } else {
                            failed_assertions += 1;
                            test_passed = false;
                            feature_passed = false;
                        }

                        assertion_results.push(LiveAssertionResult {
                            assertion_text,
                            passed: ar.passed,
                            message: ar.message.clone(),
                        });
                    }

                    let live_test_result = LiveTestResult {
                        method: resolved_test.method.clone(),
                        path: resolved_test.path.clone(),
                        passed: test_passed,
                        assertions: assertion_results,
                        preconditions: precondition_results,
                        scenario_name: Some(scenario.name.clone()),
                    };

                    // Add to test_results for backward compatibility
                    test_results.push(live_test_result.clone());

                    // Determine status: warning if there are unresolved outcomes
                    let status = if !unresolved_outcomes.is_empty() {
                        feature_passed = false; // Unresolved outcomes = incomplete test
                        "warning".to_string()
                    } else if test_passed {
                        "pass".to_string()
                    } else {
                        "fail".to_string()
                    };

                    // Add scenario result with its test result
                    scenario_results.push(LiveScenarioResult {
                        name: scenario.name.clone(),
                        description: scenario.description.clone(),
                        given_clause: scenario.given_clause.clone(),
                        when_clause: scenario.when_clause.clone(),
                        outcomes: scenario.outcomes.clone(),
                        status,
                        test_result: Some(live_test_result),
                        unresolved_outcomes,
                        component_refs,
                    });
                } else {
                    // Could not resolve scenario - mark as pending
                    scenario_results.push(LiveScenarioResult {
                        name: scenario.name.clone(),
                        description: scenario.description.clone(),
                        given_clause: scenario.given_clause.clone(),
                        when_clause: scenario.when_clause.clone(),
                        outcomes: scenario.outcomes.clone(),
                        status: "pending".to_string(),
                        test_result: None,
                        unresolved_outcomes: vec![],
                        component_refs: vec![],
                    });
                }
            }
        } else {
            // No glossary - scenarios can't be resolved
            for scenario in &feature.scenarios {
                scenario_results.push(LiveScenarioResult {
                    name: scenario.name.clone(),
                    description: scenario.description.clone(),
                    given_clause: scenario.given_clause.clone(),
                    when_clause: scenario.when_clause.clone(),
                    outcomes: scenario.outcomes.clone(),
                    status: "pending".to_string(),
                    test_result: None,
                    unresolved_outcomes: vec![],
                    component_refs: vec![],
                });
            }
        }

        // Process test: blocks (run in addition to scenario tests)
        for test in &feature.tests {
            let result = run_single_test(test, port);
            let mut assertion_results = Vec::new();
            let mut test_passed = result.passed;

            for ar in &result.assertion_results {
                total_assertions += 1;
                let assertion_text = format_assertion(&ar.assertion);

                if ar.passed {
                    passed_assertions += 1;
                } else {
                    failed_assertions += 1;
                    test_passed = false;
                    feature_passed = false;
                }

                assertion_results.push(LiveAssertionResult {
                    assertion_text,
                    passed: ar.passed,
                    message: ar.message.clone(),
                });
            }

            test_results.push(LiveTestResult {
                method: test.method.clone(),
                path: test.path.clone(),
                passed: test_passed,
                assertions: assertion_results,
                preconditions: vec![],
                scenario_name: test.scenario_name.clone(),
            });
        }

        feature_results.push(LiveFeatureResult {
            feature_id,
            feature_name: feature.name.clone(),
            description: feature.description.clone(),
            passed: feature_passed,
            tests: test_results,
            scenarios: scenario_results,
            has_implementation: has_impl,
        });
    }

    // Calculate coverage stats
    let linked_features = feature_results
        .iter()
        .filter(|f| f.has_implementation)
        .count();
    let total_features = feature_results.len();

    // Extract glossary for display
    let glossary = intent.glossary.as_ref().map(|g| {
        g.terms
            .values()
            .map(|t| GlossaryTermDisplay {
                term: t.term.clone(),
                term_type: t.term_type.clone(),
                meaning: t.meaning.clone(),
                binding: t.binding.clone(),
            })
            .collect()
    });

    // Test component scenarios
    let mut component_results = Vec::new();

    if let Some(ref glossary_obj) = intent.glossary {
        for component in &intent.components {
            let mut component_scenarios = Vec::new();
            let mut component_passed = true;

            for scenario in &component.scenarios {
                if let Some((resolved_test, unresolved_outcomes, component_refs)) = glossary_obj
                    .resolve_scenario_with_base_dir(scenario, &intent.components, intent_base_dir)
                {
                    // Check preconditions
                    let mut precondition_results = Vec::new();
                    let mut preconditions_passed = true;

                    if !resolved_test.preconditions.is_empty() {
                        let precondition_test = TestCase {
                            method: resolved_test.method.clone(),
                            path: resolved_test.path.clone(),
                            body: None,
                            assertions: resolved_test.preconditions.clone(),
                            preconditions: vec![],
                            scenario_name: None,
                        };

                        let precond_result = run_single_test(&precondition_test, port);
                        for ar in &precond_result.assertion_results {
                            let assertion_text = format_assertion(&ar.assertion);
                            precondition_results.push(LiveAssertionResult {
                                assertion_text,
                                passed: ar.passed,
                                message: ar.message.clone(),
                            });
                            if !ar.passed {
                                preconditions_passed = false;
                            }
                        }
                    }

                    // Execute component scenario test
                    let result = run_single_test(&resolved_test, port);
                    let mut assertion_results = Vec::new();
                    let mut test_passed = result.passed && preconditions_passed;

                    for ar in &result.assertion_results {
                        total_assertions += 1;
                        let assertion_text = format_assertion(&ar.assertion);

                        if ar.passed {
                            passed_assertions += 1;
                        } else {
                            failed_assertions += 1;
                            test_passed = false;
                            component_passed = false;
                        }

                        assertion_results.push(LiveAssertionResult {
                            assertion_text,
                            passed: ar.passed,
                            message: ar.message.clone(),
                        });
                    }

                    let live_test_result = LiveTestResult {
                        method: resolved_test.method.clone(),
                        path: resolved_test.path.clone(),
                        passed: test_passed,
                        assertions: assertion_results,
                        preconditions: precondition_results,
                        scenario_name: Some(scenario.name.clone()),
                    };

                    let status = if !unresolved_outcomes.is_empty() {
                        component_passed = false;
                        "warning".to_string()
                    } else if test_passed {
                        "pass".to_string()
                    } else {
                        "fail".to_string()
                    };

                    component_scenarios.push(LiveScenarioResult {
                        name: scenario.name.clone(),
                        description: scenario.description.clone(),
                        given_clause: scenario.given_clause.clone(),
                        when_clause: scenario.when_clause.clone(),
                        outcomes: scenario.outcomes.clone(),
                        status,
                        test_result: Some(live_test_result),
                        unresolved_outcomes,
                        component_refs,
                    });
                }
            }

            component_results.push(LiveComponentResult {
                component_id: component.id.clone(),
                component_name: component.name.clone(),
                description: component.description.clone().unwrap_or_default(),
                inherent_behavior: component.inherent_behavior.clone(),
                passed: component_passed,
                scenarios: component_scenarios,
            });
        }
    }

    LiveTestResults {
        features: feature_results,
        components: component_results,
        total_assertions,
        passed_assertions,
        failed_assertions,
        linked_features,
        total_features,
        glossary,
    }
}

/// Print intent check results
pub fn print_intent_results(result: &IntentCheckResult) {
    println!();

    let mut unlinked_features: Vec<&str> = Vec::new();

    for fr in &result.feature_results {
        // Feature header
        let status_icon = if fr.passed {
            "✓".green()
        } else {
            "✗".red()
        };
        println!("{} Feature: {}", status_icon, fr.feature.name.bold());

        if let Some(ref desc) = fr.feature.description {
            println!("  {}", desc.dimmed());
        }

        // Show warning if no implementation linked
        if !fr.has_implementation {
            let feature_id = fr.feature.id.as_deref().unwrap_or("(no id)");
            unlinked_features.push(feature_id);
            println!(
                "  {} {}",
                "⚠".yellow(),
                "No code linked to this feature".yellow()
            );
        }

        // Test results
        for tr in &fr.test_results {
            println!();
            let test_icon = if tr.passed {
                "✓".green()
            } else {
                "✗".red()
            };
            println!("  {} {} {}", test_icon, tr.test.method.cyan(), tr.test.path);

            // Assertion results
            for ar in &tr.assertion_results {
                let assertion_icon = if ar.passed {
                    "✓".green()
                } else {
                    "✗".red()
                };
                let assertion_desc = format_assertion(&ar.assertion);

                if ar.passed {
                    println!("    {} {}", assertion_icon, assertion_desc);
                } else {
                    println!("    {} {}", assertion_icon, assertion_desc.red());
                    if let Some(ref msg) = ar.message {
                        println!("      {}", msg.yellow());
                    }
                }
            }
        }
        println!();
    }

    // Summary
    let total_features = result.features_passed + result.features_failed;
    let total_assertions = result.assertions_passed + result.assertions_failed;

    let summary = format!(
        "{}/{} features passing ({}/{} assertions)",
        result.features_passed, total_features, result.assertions_passed, total_assertions
    );

    println!();
    if result.features_failed == 0 {
        println!("{}", summary.green().bold());
    } else {
        println!("{}", summary.red().bold());
    }

    // Show unlinked features warning at the end
    if !unlinked_features.is_empty() {
        println!();
        println!(
            "{}",
            "⚠️  Warning: Some features have no linked code"
                .yellow()
                .bold()
        );
        println!(
            "{}",
            "   Add @implements annotations to link code to features:".dimmed()
        );
        for id in &unlinked_features {
            println!("     // @implements: {}", id);
        }
    }
}

/// Format an assertion for display
fn format_assertion(assertion: &Assertion) -> String {
    match assertion {
        Assertion::Status(code) => format!("status: {}", code),
        Assertion::BodyContains(text) => format!("body contains \"{}\"", text),
        Assertion::BodyNotContains(text) => format!("body not contains \"{}\"", text),
        Assertion::BodyMatches(pattern) => format!("body matches \"{}\"", pattern),
        Assertion::HeaderContains(name, value) => {
            format!("header \"{}\" contains \"{}\"", name, value)
        }
        Assertion::JsonPathExists(path) => format!("json path \"{}\" exists", path),
        Assertion::JsonPathEquals(path, value) => {
            format!("json path \"{}\" == \"{}\"", path, value)
        }
        Assertion::RedirectsTo(path) => format!("redirects to \"{}\"", path),
        // CLI assertions
        Assertion::CliRun(cmd, args) => format!("run {} {}", cmd, args.join(" ")),
        Assertion::CliExitCode(code) => {
            if *code == 0 {
                "exits successfully".to_string()
            } else {
                format!("exit code {}", code)
            }
        }
        Assertion::CliOutputContains(text) => format!("output contains \"{}\"", text),
        Assertion::CliOutputNotContains(text) => format!("output not contains \"{}\"", text),
        Assertion::CliErrorContains(text) => format!("stderr contains \"{}\"", text),
        // Code quality assertions
        Assertion::CodeQualityPassed => "code is valid".to_string(),
        Assertion::CodeQualityNoErrors => "no syntax errors".to_string(),
        Assertion::CodeQualityNoWarnings => "no lint warnings".to_string(),
        Assertion::CodeQualityErrorCount(count) => format!("error count is {}", count),
    }
}

/// Find the intent file for a given NTNT file
/// Looks for: <name>.intent, <name>.tnt.intent, or intent.yaml in same directory
pub fn find_intent_file(ntnt_path: &Path) -> Option<std::path::PathBuf> {
    let parent = ntnt_path.parent()?;
    let stem = ntnt_path.file_stem()?.to_string_lossy();

    // Try <name>.intent
    let intent_path = parent.join(format!("{}.intent", stem));
    if intent_path.exists() {
        return Some(intent_path);
    }

    // Try <name>.tnt.intent
    let intent_path = parent.join(format!("{}.tnt.intent", stem));
    if intent_path.exists() {
        return Some(intent_path);
    }

    // Try intent.yaml in same directory
    let intent_path = parent.join("intent.yaml");
    if intent_path.exists() {
        return Some(intent_path);
    }

    None
}

/// Resolve both .intent and .tnt paths from either extension
///
/// This function accepts either a .tnt or .intent file and resolves both paths.
/// Returns (intent_path, tnt_path) where:
/// - intent_path is always the .intent file (if found)
/// - tnt_path is always the .tnt file (if found)
///
/// This allows commands like `ntnt intent studio` and `ntnt intent check`
/// to work consistently with either file extension.
pub fn resolve_intent_tnt_pair(
    input_path: &Path,
) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    let parent = match input_path.parent() {
        Some(p) => p,
        None => return (None, None),
    };

    let ext = input_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let stem = match input_path.file_stem() {
        Some(s) => s.to_string_lossy().to_string(),
        None => return (None, None),
    };

    let (intent_path, tnt_path) = if ext == "tnt" {
        // Input is .tnt, look for .intent
        let intent = parent.join(format!("{}.intent", stem));
        let tnt = input_path.to_path_buf();
        (intent, tnt)
    } else if ext == "intent" {
        // Input is .intent, look for .tnt
        let intent = input_path.to_path_buf();
        let tnt = parent.join(format!("{}.tnt", stem));
        (intent, tnt)
    } else {
        // Unknown extension, try both
        let intent = parent.join(format!("{}.intent", stem));
        let tnt = parent.join(format!("{}.tnt", stem));
        (intent, tnt)
    };

    let intent_exists = intent_path.exists();
    let tnt_exists = tnt_path.exists();

    (
        if intent_exists {
            Some(intent_path)
        } else {
            None
        },
        if tnt_exists { Some(tnt_path) } else { None },
    )
}

/// Parse annotations from NTNT source code
///
/// Looks for comments like:
/// - `// @implements: feature.site_selection`
/// - `// @supports: constraint.valid_email`
/// - `// @utility`
/// - `// @internal`
/// - `// @infrastructure`
pub fn parse_annotations(source: &str, file_path: &str) -> Vec<Annotation> {
    let mut annotations = Vec::new();
    let lines: Vec<&str> = source.lines().collect();

    for (line_num, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Look for annotation comments
        if trimmed.starts_with("// @") {
            let annotation_str = trimmed.trim_start_matches("// @");

            // Look ahead to find the next function declaration
            let mut function_name: Option<String> = None;
            for future_line in lines.iter().skip(line_num + 1) {
                let future_trimmed = future_line.trim();
                // Skip empty lines, comments
                if future_trimmed.is_empty() || future_trimmed.starts_with("//") {
                    continue;
                }
                // Found a function declaration
                if future_trimmed.starts_with("fn ") {
                    let rest = future_trimmed.trim_start_matches("fn ");
                    if let Some(paren_idx) = rest.find('(') {
                        function_name = Some(rest[..paren_idx].trim().to_string());
                    }
                }
                // Stop looking after first non-comment/empty line
                break;
            }

            if let Some(ann) =
                parse_single_annotation(annotation_str, file_path, line_num + 1, &function_name)
            {
                annotations.push(ann);
            }
        }
    }

    annotations
}

/// Parse a single annotation from its string content
fn parse_single_annotation(
    annotation_str: &str,
    file_path: &str,
    line: usize,
    function_name: &Option<String>,
) -> Option<Annotation> {
    // @implements: feature.X
    if annotation_str.starts_with("implements:") {
        let id = annotation_str
            .trim_start_matches("implements:")
            .trim()
            .to_string();
        return Some(Annotation {
            kind: AnnotationKind::Implements,
            id,
            file: file_path.to_string(),
            line,
            function_name: function_name.clone(),
        });
    }

    // @supports: constraint.X
    if annotation_str.starts_with("supports:") {
        let id = annotation_str
            .trim_start_matches("supports:")
            .trim()
            .to_string();
        return Some(Annotation {
            kind: AnnotationKind::Supports,
            id,
            file: file_path.to_string(),
            line,
            function_name: function_name.clone(),
        });
    }

    // @utility
    if annotation_str == "utility" || annotation_str.starts_with("utility ") {
        return Some(Annotation {
            kind: AnnotationKind::Utility,
            id: String::new(),
            file: file_path.to_string(),
            line,
            function_name: function_name.clone(),
        });
    }

    // @internal
    if annotation_str == "internal" || annotation_str.starts_with("internal ") {
        return Some(Annotation {
            kind: AnnotationKind::Internal,
            id: String::new(),
            file: file_path.to_string(),
            line,
            function_name: function_name.clone(),
        });
    }

    // @infrastructure
    if annotation_str == "infrastructure" || annotation_str.starts_with("infrastructure ") {
        return Some(Annotation {
            kind: AnnotationKind::Infrastructure,
            id: String::new(),
            file: file_path.to_string(),
            line,
            function_name: function_name.clone(),
        });
    }

    None
}

/// Generate a coverage report for an intent file against source files
pub fn generate_coverage_report(
    intent: &IntentFile,
    source_files: &[(String, String)], // (path, content)
) -> CoverageReport {
    // Parse all annotations from all source files
    let mut all_annotations: Vec<Annotation> = Vec::new();
    let mut file_paths: Vec<String> = Vec::new();

    for (path, content) in source_files {
        let annotations = parse_annotations(content, path);
        all_annotations.extend(annotations);
        file_paths.push(path.clone());
    }

    // Build coverage for each feature
    let mut features: Vec<FeatureCoverage> = Vec::new();
    let mut covered_count = 0;

    for feature in &intent.features {
        let feature_id = feature.id.clone().unwrap_or_else(|| {
            // Generate ID from name if not specified
            feature.name.to_lowercase().replace(' ', "_")
        });

        // Find all annotations that implement this feature
        let implementations: Vec<Annotation> = all_annotations
            .iter()
            .filter(|a| {
                a.kind == AnnotationKind::Implements
                    && (a.id == feature_id || a.id == format!("feature.{}", feature_id))
            })
            .cloned()
            .collect();

        let covered = !implementations.is_empty();
        if covered {
            covered_count += 1;
        }

        features.push(FeatureCoverage {
            feature_id,
            feature_name: feature.name.clone(),
            covered,
            implementations,
        });
    }

    let total = intent.features.len();
    let coverage_percent = if total > 0 {
        (covered_count as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    CoverageReport {
        intent_file: intent.source_path.clone(),
        source_files: file_paths,
        features,
        total_features: total,
        covered_features: covered_count,
        coverage_percent,
    }
}

/// Print coverage report
pub fn print_coverage_report(report: &CoverageReport) {
    println!();
    println!("{}", "=== Intent Coverage Report ===".cyan().bold());
    println!();
    println!("Intent: {}", report.intent_file.green());
    println!("Source files: {}", report.source_files.len());
    println!();

    for fc in &report.features {
        let status = if fc.covered {
            "✓".green()
        } else {
            "✗".red()
        };

        println!(
            "{} {} ({})",
            status,
            fc.feature_name.bold(),
            fc.feature_id.dimmed()
        );

        if fc.covered {
            for ann in &fc.implementations {
                let func_info = ann
                    .function_name
                    .as_ref()
                    .map(|f| format!(" in fn {}", f))
                    .unwrap_or_default();
                println!(
                    "    {} {}:{}{}",
                    "└─".dimmed(),
                    ann.file,
                    ann.line,
                    func_info.dimmed()
                );
            }
        } else {
            println!(
                "    {} {}",
                "└─".dimmed(),
                "No implementation found".yellow()
            );
        }
    }

    println!();

    // Summary bar
    let bar_width = 30;
    let filled = (report.coverage_percent / 100.0 * bar_width as f64) as usize;
    let empty = bar_width - filled;
    let bar = format!("[{}{}]", "█".repeat(filled), "░".repeat(empty));

    let summary = format!(
        "{} {:.1}% coverage ({}/{} features)",
        bar, report.coverage_percent, report.covered_features, report.total_features
    );

    if report.coverage_percent >= 80.0 {
        println!("{}", summary.green().bold());
    } else if report.coverage_percent >= 50.0 {
        println!("{}", summary.yellow().bold());
    } else {
        println!("{}", summary.red().bold());
    }
}

/// Generate initial code scaffolding from an intent file
pub fn generate_scaffolding(intent: &IntentFile) -> String {
    let mut output = String::new();

    output.push_str("// Auto-generated from intent file\n");
    output.push_str(&format!("// Intent: {}\n", intent.source_path));
    output.push_str("// \n");
    output.push_str("// TODO: Implement the features defined in the intent file\n\n");

    output.push_str("import { html, json, text, status } from \"std/http/server\"\n\n");

    // Generate stubs for each feature
    for feature in &intent.features {
        let feature_id = feature
            .id
            .clone()
            .unwrap_or_else(|| feature.name.to_lowercase().replace(' ', "_"));

        // Add feature comment block
        output.push_str(
            "// =============================================================================\n",
        );
        output.push_str(&format!("// Feature: {}\n", feature.name));
        if let Some(ref desc) = feature.description {
            output.push_str(&format!("// {}\n", desc));
        }
        output.push_str(
            "// =============================================================================\n\n",
        );

        // Generate handler for each test's route
        let mut seen_routes: std::collections::HashSet<String> = std::collections::HashSet::new();

        for test in &feature.tests {
            let route_key = format!("{} {}", test.method, test.path);
            if seen_routes.contains(&route_key) {
                continue;
            }
            seen_routes.insert(route_key);

            // Generate function name from path
            let fn_name = generate_function_name(&test.path, &test.method);

            output.push_str(&format!("// @implements: {}\n", feature_id));
            output.push_str(&format!("fn {}(req) {{\n", fn_name));
            output.push_str("    // TODO: Implement this handler\n");

            // Add hints from assertions
            output.push_str("    // Expected:\n");
            for assertion in &test.assertions {
                match assertion {
                    Assertion::Status(code) => {
                        output.push_str(&format!("    //   - Return status {}\n", code));
                    }
                    Assertion::BodyContains(text) => {
                        output.push_str(&format!("    //   - Body should contain: \"{}\"\n", text));
                    }
                    Assertion::BodyNotContains(text) => {
                        output.push_str(&format!(
                            "    //   - Body should NOT contain: \"{}\"\n",
                            text
                        ));
                    }
                    Assertion::BodyMatches(pattern) => {
                        output
                            .push_str(&format!("    //   - Body should match: r\"{}\"\n", pattern));
                    }
                    Assertion::HeaderContains(name, value) => {
                        output.push_str(&format!(
                            "    //   - Header \"{}\" should contain: \"{}\"\n",
                            name, value
                        ));
                    }
                    Assertion::JsonPathExists(path) => {
                        output.push_str(&format!(
                            "    //   - JSON response should have \"{}\" field\n",
                            path
                        ));
                    }
                    Assertion::JsonPathEquals(path, value) => {
                        output.push_str(&format!(
                            "    //   - JSON \"{}\" should equal \"{}\"\n",
                            path, value
                        ));
                    }
                    Assertion::RedirectsTo(path) => {
                        output.push_str(&format!("    //   - Should redirect to \"{}\"\n", path));
                    }
                    // CLI assertions are not applicable for HTTP handlers
                    Assertion::CliRun(_, _)
                    | Assertion::CliExitCode(_)
                    | Assertion::CliOutputContains(_)
                    | Assertion::CliOutputNotContains(_)
                    | Assertion::CliErrorContains(_) => {
                        // Skip CLI assertions in HTTP handler generation
                    }
                    // Code quality assertions are not applicable for HTTP handlers
                    Assertion::CodeQualityPassed
                    | Assertion::CodeQualityNoErrors
                    | Assertion::CodeQualityNoWarnings
                    | Assertion::CodeQualityErrorCount(_) => {
                        // Skip code quality assertions in HTTP handler generation
                    }
                }
            }

            output.push_str("    return text(\"Not implemented\")\n");
            output.push_str("}\n\n");
        }
    }

    // Generate route registrations
    output.push_str(
        "// =============================================================================\n",
    );
    output.push_str("// Routes\n");
    output.push_str(
        "// =============================================================================\n\n",
    );

    let mut seen_routes: std::collections::HashSet<String> = std::collections::HashSet::new();

    for feature in &intent.features {
        for test in &feature.tests {
            let route_key = format!("{} {}", test.method, test.path);
            if seen_routes.contains(&route_key) {
                continue;
            }
            seen_routes.insert(route_key);

            let fn_name = generate_function_name(&test.path, &test.method);
            let method_fn = test.method.to_lowercase();

            // Use raw string for paths with parameters
            let path_str = if test.path.contains('{') {
                format!("r\"{}\"", test.path)
            } else {
                format!("\"{}\"", test.path)
            };

            output.push_str(&format!("{}({}, {})\n", method_fn, path_str, fn_name));
        }
    }

    output.push_str("\nlisten(8080)\n");

    output
}

/// Generate a function name from a route path and method
fn generate_function_name(path: &str, method: &str) -> String {
    let clean_path = path
        .trim_start_matches('/')
        .replace('/', "_")
        .replace(['{', '}'], "")
        .replace('?', "_query")
        .replace(['&', '='], "_");

    let base = if clean_path.is_empty() {
        "index".to_string()
    } else {
        clean_path
    };

    if method == "GET" {
        base
    } else {
        format!("{}_{}", base, method.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_intent() {
        let content = r#"
Feature: Site Selection
  id: feature.site_selection
  description: "Users can select sites"
  test:
    - request: GET /
      assert:
        - status: 200
        - body contains "Bear Lake"
"#;

        let intent = IntentFile::parse_content(content, "test.intent".to_string()).unwrap();
        assert_eq!(intent.features.len(), 1);

        let feature = &intent.features[0];
        assert_eq!(feature.name, "Site Selection");
        assert_eq!(feature.id, Some("feature.site_selection".to_string()));
        assert_eq!(feature.tests.len(), 1);

        let test = &feature.tests[0];
        assert_eq!(test.method, "GET");
        assert_eq!(test.path, "/");
        assert_eq!(test.assertions.len(), 2);
    }

    #[test]
    fn test_parse_assertions() {
        assert!(matches!(
            IntentFile::parse_assertion("status: 200"),
            Some(Assertion::Status(200))
        ));

        assert!(matches!(
            IntentFile::parse_assertion("body contains \"test\""),
            Some(Assertion::BodyContains(s)) if s == "test"
        ));

        assert!(matches!(
            IntentFile::parse_assertion("body not contains \"error\""),
            Some(Assertion::BodyNotContains(s)) if s == "error"
        ));

        assert!(matches!(
            IntentFile::parse_assertion("body matches r\"\\d+\""),
            Some(Assertion::BodyMatches(s)) if s == "\\d+"
        ));
    }

    #[test]
    fn test_multiple_features() {
        let content = r#"
Feature: Home Page
  test:
    - request: GET /
      assert:
        - status: 200

Feature: API
  test:
    - request: GET /api/status
      assert:
        - status: 200
"#;

        let intent = IntentFile::parse_content(content, "test.intent".to_string()).unwrap();
        assert_eq!(intent.features.len(), 2);
        assert_eq!(intent.features[0].name, "Home Page");
        assert_eq!(intent.features[1].name, "API");
    }

    #[test]
    fn test_assertion_to_ial_term() {
        // Test conversion from Assertion to IAL term text
        let a = Assertion::Status(200);
        assert_eq!(a.to_ial_term(), "status: 200");

        let a = Assertion::BodyContains("hello".to_string());
        assert_eq!(a.to_ial_term(), "body contains \"hello\"");

        let a = Assertion::BodyNotContains("error".to_string());
        assert_eq!(a.to_ial_term(), "body not contains \"error\"");

        let a = Assertion::HeaderContains("Content-Type".to_string(), "text/html".to_string());
        assert_eq!(
            a.to_ial_term(),
            "header \"Content-Type\" contains \"text/html\""
        );

        let a = Assertion::RedirectsTo("/dashboard".to_string());
        assert_eq!(a.to_ial_term(), "redirects to \"/dashboard\"");
    }

    #[test]
    fn test_run_assertions_ial_basic() {
        // Test running assertions through the IAL engine
        let vocab = standard_vocabulary();
        let assertions = vec![
            Assertion::Status(200),
            Assertion::BodyContains("Hello".to_string()),
        ];

        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "text/html".to_string());

        let results = run_assertions_ial(&assertions, &vocab, 200, "Hello, World!", &headers);

        assert_eq!(results.len(), 2);
        assert!(results[0].passed, "Status assertion should pass");
        assert!(results[1].passed, "Body contains assertion should pass");
    }

    #[test]
    fn test_run_assertions_ial_failing() {
        // Test that assertions fail correctly
        let vocab = standard_vocabulary();
        let assertions = vec![
            Assertion::Status(404),
            Assertion::BodyContains("Error".to_string()),
        ];

        let headers = HashMap::new();
        let results = run_assertions_ial(&assertions, &vocab, 200, "Success", &headers);

        assert_eq!(results.len(), 2);
        assert!(
            !results[0].passed,
            "Status assertion should fail (expected 404, got 200)"
        );
        assert!(!results[1].passed, "Body contains assertion should fail");
    }

    #[test]
    fn test_glossary_to_ial_vocabulary() {
        // Test that glossary terms get converted to IAL vocabulary
        let mut glossary = Glossary::new();
        glossary.add_term(
            "welcome message".to_string(),
            "body contains \"Welcome to our app\"".to_string(),
        );
        glossary.add_term(
            "error displayed".to_string(),
            "They see \"Error occurred\"".to_string(),
        );

        let vocab = glossary.to_ial_vocabulary();

        // Should find the custom terms
        assert!(
            vocab.lookup("welcome message").is_some(),
            "Should find 'welcome message' term"
        );
        assert!(
            vocab.lookup("error displayed").is_some(),
            "Should find 'error displayed' term"
        );

        // Should also have standard vocabulary
        assert!(
            vocab.lookup("status: 200").is_some(),
            "Should have standard status term"
        );
        assert!(
            vocab.lookup("body contains \"test\"").is_some(),
            "Should have standard body contains term"
        );
    }

    #[test]
    fn test_component_to_ial_vocabulary() {
        // Test that components get added to IAL vocabulary
        let components = vec![Component {
            id: "component.success_response".to_string(),
            name: "Success Response".to_string(),
            description: None,
            parameters: vec!["message".to_string()],
            inherent_behavior: vec![
                "status 2xx".to_string(),
                "body contains \"ok\"".to_string(),
                "body contains \"$message\"".to_string(),
            ],
            scenarios: vec![],
        }];

        let glossary = Glossary::new();
        let vocab = glossary.to_ial_vocabulary_with_components(&components);

        // Should find the component
        let result = vocab.lookup("component.success_response(message: {message})");
        assert!(result.is_some(), "Should find component pattern");
    }

    #[test]
    fn test_glossary_component_reference() {
        // Test that glossary terms referencing components get resolved
        let components = vec![Component {
            id: "component.success_response".to_string(),
            name: "Success Response".to_string(),
            description: None,
            parameters: vec!["message".to_string()],
            inherent_behavior: vec![
                "status 2xx".to_string(),
                "body contains \"ok\"".to_string(),
                "body contains \"$message\"".to_string(),
            ],
            scenarios: vec![],
        }];

        let mut glossary = Glossary::new();
        glossary.add_term(
            "success response with {text}".to_string(),
            "component.success_response(message: {text})".to_string(),
        );

        let vocab = glossary.to_ial_vocabulary_with_components(&components);

        // Should find the glossary term
        let result = vocab.lookup("success response with {text}");
        assert!(
            result.is_some(),
            "Should find glossary term with component reference"
        );
    }

    #[test]
    fn test_component_resolution_chain() {
        // Test that a term resolves through glossary → component → assertions
        use crate::ial;

        let components = vec![Component {
            id: "component.success_response".to_string(),
            name: "Success Response".to_string(),
            description: None,
            parameters: vec!["message".to_string()],
            inherent_behavior: vec![
                "status 2xx".to_string(),
                "body contains \"ok\"".to_string(),
                "body contains \"$message\"".to_string(),
            ],
            scenarios: vec![],
        }];

        let mut glossary = Glossary::new();
        glossary.add_term(
            "success response with {text}".to_string(),
            "component.success_response(message: {text})".to_string(),
        );

        let vocab = glossary.to_ial_vocabulary_with_components(&components);

        // Resolve a concrete term through the chain
        let term = Term::new("success response with \"Hello\"");
        let result = ial::resolve(&term, &vocab);

        // Should resolve to 3 primitives (status 2xx, body contains "ok", body contains "Hello")
        assert!(result.is_ok(), "Resolution should succeed");
        let primitives = result.unwrap();
        assert_eq!(
            primitives.len(),
            3,
            "Should have 3 assertions from component"
        );
    }

    #[test]
    fn test_scenario_description_parsing() {
        let content = r#"# Test App

## Glossary
| Term | Means |
|------|-------|
| user requests home | GET / |
| welcome message | body contains "Welcome" |

---

Feature: Home Page
  id: feature.home

  Scenario: View home page
    description: "Tests the landing page experience for new visitors"
    When user requests home
    → welcome message
"#;
        let intent =
            IntentFile::parse_content(content, "test.intent".to_string()).expect("Should parse");

        let feature = &intent.features[0];
        let scenario = &feature.scenarios[0];

        assert_eq!(scenario.name, "View home page");
        assert_eq!(
            scenario.description,
            Some("Tests the landing page experience for new visitors".to_string())
        );
    }

    #[test]
    fn test_scenario_description_in_component() {
        let content = r#"# Test App

Component: Auth Flow
  id: component.auth
  parameters: []
  
  Scenario: User logs in
    description: "Happy path login flow"
    When user provides credentials
    → success
"#;
        let intent =
            IntentFile::parse_content(content, "test.intent".to_string()).expect("Should parse");

        let component = &intent.components[0];
        let scenario = &component.scenarios[0];

        assert_eq!(scenario.name, "User logs in");
        assert_eq!(
            scenario.description,
            Some("Happy path login flow".to_string())
        );
    }

    // ============================================================================
    // PHASE 3: COMPREHENSIVE IAL TESTS
    // ============================================================================

    #[test]
    fn test_glossary_parameter_substitution() {
        // Test that {param} placeholders in glossary terms are extracted correctly
        let mut glossary = Glossary::new();
        glossary.add_term(
            "user {name} sees {count} items".to_string(),
            "GET /api/users/{name}/items returns body contains \"{count}\"".to_string(),
        );

        // The glossary should store the term with placeholders
        assert!(glossary.get("user {name} sees {count} items").is_some());
    }

    #[test]
    fn test_glossary_multiple_terms() {
        let mut glossary = Glossary::new();
        glossary.add_term("home page".to_string(), "GET /".to_string());
        glossary.add_term("api status".to_string(), "GET /api/status".to_string());
        glossary.add_term("create user".to_string(), "POST /api/users".to_string());

        assert!(glossary.get("home page").is_some());
        assert!(glossary.get("api status").is_some());
        assert!(glossary.get("create user").is_some());
        assert!(glossary.get("nonexistent").is_none());
    }

    #[test]
    fn test_component_with_multiple_parameters() {
        let content = r#"# Test App

Component: API Response
  id: component.api_response
  parameters: [status, message]
  Inherent Behavior:
    → status $status
    → body contains "$message"
"#;
        let intent =
            IntentFile::parse_content(content, "test.intent".to_string()).expect("Should parse");

        let component = &intent.components[0];
        assert_eq!(component.parameters, vec!["status", "message"]);
        assert_eq!(component.inherent_behavior.len(), 2);
    }

    #[test]
    fn test_scenario_with_given_clause() {
        let content = r#"# Test App

## Glossary
| Term | Means |
|------|-------|
| user is logged in | GET /api/me returns status: 200 |
| home page | GET / |
| welcome | body contains "Welcome" |

---

Feature: Dashboard
  id: feature.dashboard

  Scenario: View dashboard
    Given user is logged in
    When home page
    → welcome
"#;
        let intent =
            IntentFile::parse_content(content, "test.intent".to_string()).expect("Should parse");

        let scenario = &intent.features[0].scenarios[0];
        assert_eq!(scenario.given_clause, Some("user is logged in".to_string()));
        assert_eq!(scenario.when_clause, "home page");
        assert_eq!(scenario.outcomes, vec!["welcome"]);
    }

    #[test]
    fn test_ial_vocabulary_standard_assertions() {
        use crate::ial;

        let vocab = ial::standard_vocabulary();

        // Test various standard assertions are present
        assert!(vocab.lookup("status: 200").is_some());
        assert!(vocab.lookup("status 2xx").is_some());
        assert!(vocab.lookup("status 4xx").is_some());
        assert!(vocab.lookup("body contains \"test\"").is_some());
        assert!(vocab.lookup("body not contains \"error\"").is_some());
    }

    #[test]
    fn test_ial_resolve_status_range() {
        use crate::ial::{self, CheckOp, Primitive, Term, Value};

        let vocab = ial::standard_vocabulary();
        let term = Term::new("status 2xx");

        let result = ial::resolve(&term, &vocab);
        assert!(result.is_ok());

        let primitives = result.unwrap();
        assert_eq!(primitives.len(), 1);

        match &primitives[0] {
            Primitive::Check { op, path, expected } => {
                assert!(matches!(op, CheckOp::InRange));
                assert_eq!(path, "response.status");
                if let Value::Range(min, max) = expected {
                    assert_eq!(*min, 200.0);
                    assert_eq!(*max, 299.0);
                } else {
                    panic!("Expected Range value");
                }
            }
            _ => panic!("Expected Check primitive"),
        }
    }

    #[test]
    fn test_feature_description_separate_from_scenario() {
        let content = r#"# Test App

Feature: User Management
  id: feature.users
  description: "All user-related features"

  Scenario: List users
    description: "Tests the user listing endpoint"
    When users requested
    → user list shown
"#;
        let intent =
            IntentFile::parse_content(content, "test.intent".to_string()).expect("Should parse");

        let feature = &intent.features[0];
        let scenario = &feature.scenarios[0];

        // Both should have their own descriptions
        assert_eq!(
            feature.description,
            Some("All user-related features".to_string())
        );
        assert_eq!(
            scenario.description,
            Some("Tests the user listing endpoint".to_string())
        );
    }

    #[test]
    fn test_component_inherent_behavior_parsing() {
        let content = r#"# Test App

Component: Success Response
  id: component.success
  parameters: []
  Inherent Behavior:
    → status 2xx
    → body contains "ok"
    → body contains "success"
"#;
        let intent =
            IntentFile::parse_content(content, "test.intent".to_string()).expect("Should parse");

        let component = &intent.components[0];
        assert_eq!(component.inherent_behavior.len(), 3);
        assert_eq!(component.inherent_behavior[0], "status 2xx");
        assert_eq!(component.inherent_behavior[1], "body contains \"ok\"");
        assert_eq!(component.inherent_behavior[2], "body contains \"success\"");
    }

    #[test]
    fn test_empty_intent_file() {
        let content = "";
        let intent = IntentFile::parse_content(content, "empty.intent".to_string())
            .expect("Should parse empty");

        assert!(intent.features.is_empty());
        assert!(intent.components.is_empty());
        assert!(intent.glossary.is_none());
    }

    #[test]
    fn test_intent_file_only_glossary() {
        let content = r#"# Test App

## Glossary
| Term | Means |
|------|-------|
| home | GET / |
| about | GET /about |
"#;
        let intent =
            IntentFile::parse_content(content, "test.intent".to_string()).expect("Should parse");

        assert!(intent.glossary.is_some());
        let glossary = intent.glossary.unwrap();
        assert!(glossary.get("home").is_some());
        assert!(glossary.get("about").is_some());
    }

    #[test]
    fn test_scenario_without_given() {
        let content = r#"# Test App

Feature: Simple
  id: feature.simple

  Scenario: Basic test
    When home page requested
    → success
"#;
        let intent =
            IntentFile::parse_content(content, "test.intent".to_string()).expect("Should parse");

        let scenario = &intent.features[0].scenarios[0];
        assert!(scenario.given_clause.is_none());
        assert_eq!(scenario.when_clause, "home page requested");
    }

    #[test]
    fn test_multiple_scenarios_in_feature() {
        let content = r#"# Test App

Feature: API
  id: feature.api

  Scenario: Get status
    When status requested
    → status ok

  Scenario: Get health
    When health requested
    → health ok

  Scenario: Get version
    When version requested
    → version shown
"#;
        let intent =
            IntentFile::parse_content(content, "test.intent".to_string()).expect("Should parse");

        assert_eq!(intent.features[0].scenarios.len(), 3);
        assert_eq!(intent.features[0].scenarios[0].name, "Get status");
        assert_eq!(intent.features[0].scenarios[1].name, "Get health");
        assert_eq!(intent.features[0].scenarios[2].name, "Get version");
    }

    #[test]
    fn test_component_scenarios() {
        let content = r#"# Test App

Component: Auth
  id: component.auth
  parameters: []

  Scenario: Valid login
    When credentials provided
    → token returned

  Scenario: Invalid login
    When bad credentials provided
    → error returned
"#;
        let intent =
            IntentFile::parse_content(content, "test.intent".to_string()).expect("Should parse");

        let component = &intent.components[0];
        assert_eq!(component.scenarios.len(), 2);
        assert_eq!(component.scenarios[0].name, "Valid login");
        assert_eq!(component.scenarios[1].name, "Invalid login");
    }
}
