# Intent Programming Language

Intent is a revolutionary programming language and ecosystem designed specifically for AI-driven development. Unlike traditional languages built for human developers, Intent empowers AI agents as primary software creators while maintaining deep human oversight and collaboration.

## Quick Start

```bash
# Clone and build
git clone https://github.com/joshcramer/intent.git
cd intent
cargo build --release

# Run a program
./target/release/intent examples/contracts_full.intent

# Start the REPL
./target/release/intent repl
```

## File Extensions

Intent supports two file extensions:
- `.intent` - Standard Intent source files
- `.itn` - Short form (convenient for quick scripts)

## Overview

Intent reimagines software development for an era where AI agents handle the heavy lifting of coding, testing, and deployment. The language features:

- **First-Class Contracts**: Design-by-contract principles built into the syntax for guaranteed correctness
- **Typed Error Effects**: Explicit error handling and failure conditions
- **Semantic Versioning**: Automatic API compatibility management
- **Structured Edits**: AST-based code manipulation for safe refactoring
- **Multi-Agent Collaboration**: Built-in support for AI agents working together
- **Human-in-the-Loop Governance**: Transparent decision-making with human approval gates

## Example

```intent
struct BankAccount {
    balance: Int,
    owner: String
}

impl BankAccount {
    // Invariant: balance can never go negative
    invariant self.balance >= 0
}

fn withdraw(account: BankAccount, amount: Int) -> Bool
    requires amount > 0
    ensures result == true implies account.balance == old(account.balance) - amount
{
    if account.balance >= amount {
        account.balance = account.balance - amount
        return true
    }
    return false
}
```

## Editor Support

### VS Code
Install the Intent Language extension for syntax highlighting:
```bash
cp -r editors/vscode/intent-lang ~/.vscode/extensions/
```
Then restart VS Code. The extension provides:
- Syntax highlighting for `.intent` and `.itn` files
- Code snippets for common patterns
- Bracket matching and auto-closing

## Vision

Intent bridges the gap between AI's speed and consistency with human judgment and design sense. The ecosystem includes:

- Integrated development workflows (CI/CD, reviews, pull requests)
- Rich observability and explainability features
- Formal concurrency protocols
- UI/UX constraint declarations
- Intent encoding for self-documenting code

## Documentation

- [Whitepaper](whitepaper.md) - Complete technical specification and motivation
- [Architecture](ARCHITECTURE.md) - System design and components
- [Language Spec](LANGUAGE_SPEC.md) - Formal language definition
- [Roadmap](ROADMAP.md) - Implementation plan and progress

## Contributing

Intent is an open-source project welcoming contributions from developers, language designers, and AI researchers. See our contributing guidelines for details.

## License

MIT

## Contact

For questions or discussions, please open an issue on this repository.
