# Contributing to NTNT

Thank you for your interest in contributing to NTNT! This document provides guidelines for contributing to the project.

## Code of Conduct

By participating in this project, you agree to abide by our [Code of Conduct](CODE_OF_CONDUCT.md).

## Ways to Contribute

### Reporting Bugs

- Check existing issues to avoid duplicates
- Use a clear, descriptive title
- Describe the steps to reproduce the issue
- Include your environment (OS, Rust version, NTNT version)
- Provide example `.tnt` code if applicable

### Suggesting Features

- Open an issue with the `enhancement` label
- Explain the use case and why it would be valuable
- Consider how it fits with NTNT's goals (agent-native development, Intent-Driven Development)

### Improving Documentation

- Fix typos, clarify explanations, add examples
- Help improve the [AI Agent Guide](docs/AI_AGENT_GUIDE.md) with common patterns
- Add examples to the `examples/` directory

### Contributing Code

1. **Fork the repository** and create a branch from `main`
2. **Make your changes** with clear, focused commits
3. **Add tests** if applicable
4. **Run the test suite**: `cargo test`
5. **Run the linter**: `cargo clippy`
6. **Submit a pull request**

## Development Setup

```bash
# Clone your fork
git clone https://github.com/YOUR_USERNAME/ntnt.git
cd ntnt

# Build (faster dev builds)
cargo build --profile dev-release

# Run tests
cargo test

# Run a .tnt file
cargo run -- run examples/hello.tnt

# Install locally for testing
cargo install --path . --profile dev-release --locked
```

## Project Structure

```
ntnt/
├── src/              # Rust source code
│   ├── main.rs       # CLI entry point
│   ├── lexer.rs      # Tokenizer
│   ├── parser.rs     # Parser
│   ├── interpreter.rs # Runtime
│   ├── intent.rs     # Intent-Driven Development
│   └── stdlib/       # Standard library implementations
├── examples/         # Example .tnt programs
├── docs/             # Documentation
└── tests/            # Integration tests
```

## Coding Guidelines

- Follow existing code style (run `cargo fmt`)
- Add doc comments for public functions
- Keep functions focused and reasonably sized
- Prefer clarity over cleverness

## Commit Messages

- Use present tense ("Add feature" not "Added feature")
- Keep the first line under 72 characters
- Reference issues when relevant (`Fixes #123`)

## Pull Request Process

1. Update documentation if needed
2. Ensure all tests pass
3. Request review from maintainers
4. Address feedback promptly

## Questions?

- Open a GitHub issue for questions about contributing
- Check existing issues and discussions for answers

---

_Thank you for helping make NTNT better!_
