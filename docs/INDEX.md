# NTNT Documentation Index

This index lists all current, maintained documentation for the NTNT programming language.

## Getting Started

| Document | Description |
|----------|-------------|
| [README](../README.md) | Project overview, installation, quick start |
| [AI Agent Guide](AI_AGENT_GUIDE.md) | Comprehensive language guide and syntax reference |
| [Whitepaper](../whitepaper.md) | Theoretical foundations and motivation |

## Reference (Auto-Generated)

These documents are auto-generated from TOML source files. Regenerate with: `ntnt docs --generate`

| Document | Description |
|----------|-------------|
| [Standard Library Reference](STDLIB_REFERENCE.md) | All stdlib functions and builtins |
| [Syntax Reference](SYNTAX_REFERENCE.md) | Keywords, operators, types, templates |
| [IAL Reference](IAL_REFERENCE.md) | Intent Assertion Language primitives and terms |

Source files:
- Stdlib functions: `// @ntnt` comments in `src/stdlib/*.rs` and `src/interpreter.rs`
- [syntax.toml](syntax.toml) - Language syntax
- [ial.toml](ial.toml) - IAL specification

## Development Guides

| Document | Description |
|----------|-------------|
| [AI Agent Guide](AI_AGENT_GUIDE.md) | Critical syntax rules for AI-assisted development |
| [Deployment Guide](DEPLOYMENT_GUIDE.md) | Docker, workers, scaling, Cloudflare, production setup |
| [Architecture](../ARCHITECTURE.md) | System design and implementation details |
| [Language Overview](ntnt_language_overview.md) | High-level feature summary |

## Project

| Document | Description |
|----------|-------------|
| [Roadmap](../ROADMAP.md) | Implementation phases and progress |
| [v0.5.3 Release Notes](release-notes/v0.5.3.md) | Bounded numeric SNMPv2c GETNEXT WALK with strict completion and resource ceilings |
| [v0.5.2 Release Notes](release-notes/v0.5.2.md) | Gated, bounded SNMPv2c GET through the new explicitly imported `std/netmon` module |
| [v0.5.1 Release Notes](release-notes/v0.5.1.md) | Passwordless magic-link flow, opaque provider-neutral secrets, compatibility guarantees, and auth hardening |
| [v0.5.0 Release Notes](release-notes/v0.5.0.md) | Verification, validation, email, and multi-worker improvements |
| [v0.4.9 Release Notes](release-notes/v0.4.9.md) | Local auth primitives, backend support, demo, and upgrade guidance |
| [Contributing](../CONTRIBUTING.md) | How to contribute |
| [Code of Conduct](../CODE_OF_CONDUCT.md) | Community standards |
| [Acknowledgements](../ACKNOWLEDGEMENTS.md) | Credits and recognition |

## Editor Support

| Document | Description |
|----------|-------------|
| [VS Code Extension](../editors/vscode/intent-lang/README.md) | Syntax highlighting for .tnt and .intent files |

## Internal

| Document | Description |
|----------|-------------|
| [Source Architecture](../src/README.md) | Compiler/interpreter implementation |

---

## See Also

- [Agent Documentation Index](AGENT_DOCS_INDEX.md) - Documentation specifically for AI agents
- [Design Documents](../design-docs/) - Planning and design ideas (may not reflect current state)
- [Examples](../examples/README.md) - Code examples and demos
