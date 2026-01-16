# Acknowledgements

NTNT would not exist without the incredible work of many individuals, communities, and organizations. This document recognizes those who made this project possible.

## AI Development Partners

This project was developed in collaboration with AI assistants, representing a new paradigm in human-AI software development:

### Anthropic

- **Claude Code** and **Claude Opus 4.5** - The primary AI collaborator for NTNT's development. Claude helped design language features, write the Rust implementation, create documentation, and reason through complex architectural decisions. The collaborative development process itself validated many of NTNT's core ideas about Intent-Driven Development.

### GitHub

- **GitHub Copilot** - Provided valuable code completion and suggestions throughout the development process.

## The Rust Ecosystem

NTNT is built entirely in Rust, and we're grateful to:

- **The Rust Team** - For creating a language that makes systems programming both safe and enjoyable. Rust's ownership model, powerful type system, and excellent tooling (cargo, rustfmt, clippy) made building a language runtime a pleasure.

- **Key Dependencies:**
  - [Tokio](https://tokio.rs/) - Async runtime powering NTNT's concurrent features
  - [Actix-web](https://actix.rs/) - Fast HTTP server implementation
  - [Reqwest](https://docs.rs/reqwest/) - HTTP client for `std/http`
  - [Tokio-postgres](https://docs.rs/tokio-postgres/) - Async PostgreSQL support
  - [Serde](https://serde.rs/) - Serialization framework for JSON handling
  - [Clap](https://docs.rs/clap/) - Command-line argument parsing
  - [Rustyline](https://docs.rs/rustyline/) - REPL line editing

## Language Design Inspirations

NTNT draws from decades of programming language research:

### Design-by-Contract

- **Bertrand Meyer** and **Eiffel** - The Design-by-Contract philosophy, where "software designers should define formal, precise, verifiable interface specifications for components," directly inspired NTNT's `requires`/`ensures` contract syntax.

- **Racket** - Contract system inspiration for making contracts first-class citizens in the language.

### Effect Systems & Functional Programming

- The functional programming community's work on **effect types** influenced NTNT's approach to making side effects explicit and trackable.

### Semantic Versioning

- **Elm** - Evan Czaplicki's approach to enforcing semantic versioning at the package manager level inspired NTNT's vision of automatic version detection based on API changes. Elm's guarantee that you "never run into a breaking API change in a patch release" is a north star for NTNT's ecosystem goals.

### Concurrency

- **Session Types Research** - Academic work on session types for type-safe communication protocols informed NTNT's concurrent channel design.

### Syntax & Ergonomics

- **Go** - Simplicity and readability as core design values
- **Rust** - Safety guarantees without sacrificing expressiveness
- **Node.js/JavaScript** - Ecosystem agility and the productivity of a dynamic-feeling language
- **Python** - Clean, readable syntax and the power of a strong standard library

## The AI-First Development Vision

NTNT emerged from thinking deeply about how programming languages should evolve for an era of AI-assisted development. Key influences:

- **Research on AI Training with Edit Sequences** - Work showing that AI models learn more effectively from seeing edit histories rather than just final code states.

- **Explainable AI (XAI)** - Principles of transparency in algorithmic decision processes influenced NTNT's observability features and intent annotations.

- **The AI Language Specification Experiment** - Early experiments with JSON-based language specifications designed to be machine-readable first.

## Community

Thank you to everyone who:

- Files issues and bug reports
- Contributes code, documentation, or examples
- Provides feedback on the language design
- Spreads the word about NTNT
- Experiments with Intent-Driven Development

## A Note on Human-AI Collaboration

NTNT itself is proof that meaningful software can emerge from human-AI collaboration. The human provides vision, taste, and judgment. The AI provides tireless implementation, broad knowledge synthesis, and rapid iteration. Together, they create something neither could alone.

This project aims to make that collaboration explicit, traceable, and verifiable—not just for NTNT's own development, but for every project built with it.

## A Note from Josh Cramer

This started (and still is) a fun side project for me. If you find this project interesting, fun, inspiring, or useful, that is inspiring and rewarding to me. I'm genuinely thankful that you dedicated some of your precious time to check out this crazy idea.

---

_If you've contributed to NTNT and aren't listed here, please open a PR!_
