//! Contract system for Intent
//!
//! Implements design-by-contract with preconditions, postconditions, and invariants.

use crate::error::{IntentError, Result};
use std::collections::HashMap;

/// A contract specification
#[derive(Debug, Clone)]
pub struct ContractSpec {
    /// Preconditions that must hold before execution
    pub requires: Vec<ContractClause>,
    
    /// Postconditions that must hold after execution
    pub ensures: Vec<ContractClause>,
    
    /// Invariants that must hold throughout execution
    pub invariants: Vec<ContractClause>,
}

/// A single contract clause
#[derive(Debug, Clone)]
pub struct ContractClause {
    /// The condition expression (as source)
    pub condition: String,
    
    /// Human-readable description
    pub message: Option<String>,
    
    /// Whether this requires human approval to modify
    pub requires_approval: bool,
}

/// Contract checker for runtime verification
pub struct ContractChecker {
    /// Named contracts that can be referenced
    contracts: HashMap<String, ContractSpec>,
    
    /// Whether contract checking is enabled
    enabled: bool,
    
    /// Callback for approval requests
    approval_handler: Option<Box<dyn Fn(&str) -> bool>>,
}

impl ContractChecker {
    pub fn new() -> Self {
        ContractChecker {
            contracts: HashMap::new(),
            enabled: true,
            approval_handler: None,
        }
    }

    /// Enable or disable contract checking
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
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

    /// Verify a precondition
    pub fn check_requires(&self, clause: &ContractClause, result: bool) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        if !result {
            let msg = clause
                .message
                .clone()
                .unwrap_or_else(|| format!("Precondition failed: {}", clause.condition));
            return Err(IntentError::ContractViolation(msg));
        }
        Ok(())
    }

    /// Verify a postcondition
    pub fn check_ensures(&self, clause: &ContractClause, result: bool) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        if !result {
            let msg = clause
                .message
                .clone()
                .unwrap_or_else(|| format!("Postcondition failed: {}", clause.condition));
            return Err(IntentError::ContractViolation(msg));
        }
        Ok(())
    }

    /// Verify an invariant
    pub fn check_invariant(&self, clause: &ContractClause, result: bool) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        if !result {
            let msg = clause
                .message
                .clone()
                .unwrap_or_else(|| format!("Invariant violated: {}", clause.condition));
            return Err(IntentError::ContractViolation(msg));
        }
        Ok(())
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

    /// Add a precondition
    pub fn add_requires(&mut self, condition: String, message: Option<String>) {
        self.requires.push(ContractClause {
            condition,
            message,
            requires_approval: false,
        });
    }

    /// Add a postcondition
    pub fn add_ensures(&mut self, condition: String, message: Option<String>) {
        self.ensures.push(ContractClause {
            condition,
            message,
            requires_approval: false,
        });
    }

    /// Add an invariant
    pub fn add_invariant(&mut self, condition: String, message: Option<String>) {
        self.invariants.push(ContractClause {
            condition,
            message,
            requires_approval: false,
        });
    }
}

impl Default for ContractSpec {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for creating contracts fluently
pub struct ContractBuilder {
    spec: ContractSpec,
}

impl ContractBuilder {
    pub fn new() -> Self {
        ContractBuilder {
            spec: ContractSpec::new(),
        }
    }

    pub fn requires(mut self, condition: &str) -> Self {
        self.spec.add_requires(condition.to_string(), None);
        self
    }

    pub fn requires_with_message(mut self, condition: &str, message: &str) -> Self {
        self.spec
            .add_requires(condition.to_string(), Some(message.to_string()));
        self
    }

    pub fn ensures(mut self, condition: &str) -> Self {
        self.spec.add_ensures(condition.to_string(), None);
        self
    }

    pub fn ensures_with_message(mut self, condition: &str, message: &str) -> Self {
        self.spec
            .add_ensures(condition.to_string(), Some(message.to_string()));
        self
    }

    pub fn invariant(mut self, condition: &str) -> Self {
        self.spec.add_invariant(condition.to_string(), None);
        self
    }

    pub fn build(self) -> ContractSpec {
        self.spec
    }
}

impl Default for ContractBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_builder() {
        let contract = ContractBuilder::new()
            .requires("x > 0")
            .ensures("result >= 0")
            .build();

        assert_eq!(contract.requires.len(), 1);
        assert_eq!(contract.ensures.len(), 1);
    }

    #[test]
    fn test_contract_checker() {
        let checker = ContractChecker::new();
        let clause = ContractClause {
            condition: "x > 0".to_string(),
            message: None,
            requires_approval: false,
        };

        assert!(checker.check_requires(&clause, true).is_ok());
        assert!(checker.check_requires(&clause, false).is_err());
    }
}
