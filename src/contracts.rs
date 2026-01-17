//! Contract system for Intent
//!
//! Implements design-by-contract with preconditions, postconditions, and invariants.
//!
//! # Design-by-Contract
//!
//! Intent supports three types of contracts:
//! - **Preconditions** (`requires`): Conditions that must be true before a function executes
//! - **Postconditions** (`ensures`): Conditions that must be true after a function executes
//! - **Invariants** (`invariant`): Conditions that must be true throughout an object's lifetime
//!
//! # Special Variables in Contracts
//!
//! - `result`: Refers to the return value in postconditions
//! - `old(expr)`: Refers to the value of an expression before function execution

use crate::ast::Expression;
use crate::error::{IntentError, Result};
use std::collections::HashMap;
use std::fmt;

/// A contract specification containing all contract clauses for a function
#[derive(Debug, Clone)]
pub struct ContractSpec {
    /// Preconditions that must hold before execution
    pub requires: Vec<ContractClause>,

    /// Postconditions that must hold after execution
    pub ensures: Vec<ContractClause>,

    /// Invariants that must hold throughout execution
    pub invariants: Vec<ContractClause>,
}

/// A single contract clause with its AST expression
#[derive(Debug, Clone)]
pub struct ContractClause {
    /// The condition expression as AST (for evaluation)
    pub expression: Expression,

    /// The condition as source code (for error messages)
    pub condition: String,

    /// Human-readable description/message
    pub message: Option<String>,

    /// Whether this requires human approval to modify
    pub requires_approval: bool,
}

/// Stored old values for postcondition evaluation
#[derive(Debug, Clone, Default)]
pub struct OldValues {
    /// Map from expression string to its pre-execution value
    values: HashMap<String, StoredValue>,
}

/// A stored value that can be compared
#[derive(Debug, Clone)]
pub enum StoredValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Array(Vec<StoredValue>),
    Unit,
}

impl fmt::Display for StoredValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoredValue::Int(n) => write!(f, "{}", n),
            StoredValue::Float(n) => write!(f, "{}", n),
            StoredValue::Bool(b) => write!(f, "{}", b),
            StoredValue::String(s) => write!(f, "\"{}\"", s),
            StoredValue::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| v.to_string()).collect();
                write!(f, "[{}]", items.join(", "))
            }
            StoredValue::Unit => write!(f, "()"),
        }
    }
}

impl OldValues {
    pub fn new() -> Self {
        OldValues {
            values: HashMap::new(),
        }
    }

    /// Store an old value
    pub fn store(&mut self, key: String, value: StoredValue) {
        self.values.insert(key, value);
    }

    /// Retrieve an old value
    pub fn get(&self, key: &str) -> Option<&StoredValue> {
        self.values.get(key)
    }

    /// Check if a key exists
    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }
}

/// Result of a contract check
#[derive(Debug)]
pub struct ContractResult {
    pub passed: bool,
    pub clause_type: ContractType,
    pub condition: String,
    pub message: Option<String>,
    pub actual_value: Option<String>,
}

/// Type of contract clause
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ContractType {
    Precondition,
    Postcondition,
    Invariant,
}

impl fmt::Display for ContractType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContractType::Precondition => write!(f, "Precondition"),
            ContractType::Postcondition => write!(f, "Postcondition"),
            ContractType::Invariant => write!(f, "Invariant"),
        }
    }
}

/// Contract checker for runtime verification
pub struct ContractChecker {
    /// Named contracts that can be referenced
    contracts: HashMap<String, ContractSpec>,

    /// Whether contract checking is enabled
    enabled: bool,

    /// Callback for approval requests
    #[allow(clippy::type_complexity)]
    approval_handler: Option<Box<dyn Fn(&str) -> bool>>,

    /// Contract violation count for statistics
    violation_count: usize,

    /// Contract check count for statistics
    check_count: usize,
}

impl ContractChecker {
    pub fn new() -> Self {
        ContractChecker {
            contracts: HashMap::new(),
            enabled: true,
            approval_handler: None,
            violation_count: 0,
            check_count: 0,
        }
    }

    /// Enable or disable contract checking
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if contract checking is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Register a named contract
    pub fn register(&mut self, name: String, spec: ContractSpec) {
        self.contracts.insert(name, spec);
    }

    /// Get a registered contract
    pub fn get(&self, name: &str) -> Option<&ContractSpec> {
        self.contracts.get(name)
    }

    /// Set the approval handler
    pub fn set_approval_handler<F>(&mut self, handler: F)
    where
        F: Fn(&str) -> bool + 'static,
    {
        self.approval_handler = Some(Box::new(handler));
    }

    /// Check if an approval is granted
    pub fn request_approval(&self, reason: &str) -> Result<bool> {
        if let Some(ref handler) = self.approval_handler {
            Ok(handler(reason))
        } else {
            Err(IntentError::RequiresApproval(reason.to_string()))
        }
    }

    /// Verify a precondition and return a detailed result
    pub fn check_precondition(
        &mut self,
        condition: &str,
        result: bool,
        message: Option<&str>,
    ) -> Result<()> {
        self.check_count += 1;

        if !self.enabled {
            return Ok(());
        }

        if !result {
            self.violation_count += 1;
            let msg = message
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Precondition failed: {}", condition));
            return Err(IntentError::ContractViolation(msg));
        }

        Ok(())
    }

    /// Verify a postcondition and return a detailed result
    pub fn check_postcondition(
        &mut self,
        condition: &str,
        result: bool,
        message: Option<&str>,
    ) -> Result<()> {
        self.check_count += 1;

        if !self.enabled {
            return Ok(());
        }

        if !result {
            self.violation_count += 1;
            let msg = message
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Postcondition failed: {}", condition));
            return Err(IntentError::ContractViolation(msg));
        }

        Ok(())
    }

    /// Verify an invariant and return a detailed result
    pub fn check_invariant(
        &mut self,
        condition: &str,
        result: bool,
        message: Option<&str>,
    ) -> Result<()> {
        self.check_count += 1;

        if !self.enabled {
            return Ok(());
        }

        if !result {
            self.violation_count += 1;
            let msg = message
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Invariant violated: {}", condition));
            return Err(IntentError::ContractViolation(msg));
        }

        Ok(())
    }

    /// Get contract statistics
    pub fn stats(&self) -> (usize, usize) {
        (self.check_count, self.violation_count)
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.check_count = 0;
        self.violation_count = 0;
    }
}

impl Default for ContractChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl ContractSpec {
    pub fn new() -> Self {
        ContractSpec {
            requires: Vec::new(),
            ensures: Vec::new(),
            invariants: Vec::new(),
        }
    }

    /// Add a precondition with an AST expression
    pub fn add_requires(&mut self, expr: Expression, condition: String, message: Option<String>) {
        self.requires.push(ContractClause {
            expression: expr,
            condition,
            message,
            requires_approval: false,
        });
    }

    /// Add a postcondition with an AST expression  
    pub fn add_ensures(&mut self, expr: Expression, condition: String, message: Option<String>) {
        self.ensures.push(ContractClause {
            expression: expr,
            condition,
            message,
            requires_approval: false,
        });
    }

    /// Add an invariant with an AST expression
    pub fn add_invariant(&mut self, expr: Expression, condition: String, message: Option<String>) {
        self.invariants.push(ContractClause {
            expression: expr,
            condition,
            message,
            requires_approval: false,
        });
    }

    /// Check if this contract spec has any clauses
    pub fn is_empty(&self) -> bool {
        self.requires.is_empty() && self.ensures.is_empty() && self.invariants.is_empty()
    }
}

impl Default for ContractSpec {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_checker_precondition() {
        let mut checker = ContractChecker::new();

        // Should pass when condition is true
        assert!(checker.check_precondition("x > 0", true, None).is_ok());

        // Should fail when condition is false
        assert!(checker.check_precondition("x > 0", false, None).is_err());
    }

    #[test]
    fn test_contract_checker_postcondition() {
        let mut checker = ContractChecker::new();

        assert!(checker
            .check_postcondition("result >= 0", true, None)
            .is_ok());
        assert!(checker
            .check_postcondition("result >= 0", false, None)
            .is_err());
    }

    #[test]
    fn test_contract_checker_invariant() {
        let mut checker = ContractChecker::new();

        assert!(checker.check_invariant("balance >= 0", true, None).is_ok());
        assert!(checker
            .check_invariant("balance >= 0", false, None)
            .is_err());
    }

    #[test]
    fn test_contract_checker_disabled() {
        let mut checker = ContractChecker::new();
        checker.set_enabled(false);

        // Should pass even when condition is false if disabled
        assert!(checker.check_precondition("x > 0", false, None).is_ok());
    }

    #[test]
    fn test_contract_stats() {
        let mut checker = ContractChecker::new();

        let _ = checker.check_precondition("x > 0", true, None);
        let _ = checker.check_postcondition("result > 0", true, None);
        let _ = checker.check_invariant("y > 0", false, None);

        let (checks, violations) = checker.stats();
        assert_eq!(checks, 3);
        assert_eq!(violations, 1);
    }

    #[test]
    fn test_old_values() {
        let mut old = OldValues::new();
        old.store("x".to_string(), StoredValue::Int(42));
        old.store("name".to_string(), StoredValue::String("test".to_string()));

        assert!(old.contains("x"));
        assert!(!old.contains("y"));

        match old.get("x") {
            Some(StoredValue::Int(n)) => assert_eq!(*n, 42),
            _ => panic!("Expected Int"),
        }
    }
}
