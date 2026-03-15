//! NTNT Language CLI
//!
//! Command-line interface for the NTNT (Intent) programming language.

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use colored::*;
use ntnt::{
    error::IntentError, intent, intent_studio_server, interpreter::Interpreter, lexer::Lexer,
    parser::Parser as IntentParser,
};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ntnt")]
#[command(author = "NTNT Language Team")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "NTNT (Intent) - A programming language for AI-driven development")]
#[command(
    long_about = "NTNT (Intent) - A programming language for AI-driven development\n\n\
Environment variables:\n  \
NTNT_ENV=production    Disable hot-reload for better performance\n  \
NTNT_TIMEOUT=60        Request timeout in seconds (default: 30)\n\n\
Quick start:\n  \
ntnt run server.tnt    Run a file (hot-reload enabled by default)\n  \
ntnt lint app.tnt      Check for errors and style issues\n  \
ntnt docs              Browse stdlib documentation"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Source file to execute
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the interactive REPL
    Repl,
    /// Run an NTNT source file
    ///
    /// For HTTP servers, the program runs until Ctrl+C:
    ///   ntnt run examples/http_server.tnt
    ///
    /// The HTTP server uses Axum + Tokio for high-concurrency production use.
    Run {
        /// The source file to run
        #[arg(value_name = "FILE")]
        file: PathBuf,

        /// Request timeout in seconds for HTTP server (default: 30, env: NTNT_TIMEOUT)
        #[arg(long, default_value = "30", env("NTNT_TIMEOUT"))]
        timeout: u64,
    },
    /// Test an HTTP server by running it and making requests
    ///
    /// Starts the server, makes the specified HTTP request(s), prints responses,
    /// then shuts down. Perfect for AI agents and CI/CD testing.
    ///
    /// Examples:
    ///   ntnt test server.tnt --get /api/status
    ///   ntnt test server.tnt --get "/divide?a=10&b=2"
    ///   ntnt test server.tnt --post /users --body '{"name":"test"}'
    ///   ntnt test server.tnt --get /health --get /api/status
    Test {
        /// The source file containing the HTTP server
        #[arg(value_name = "FILE")]
        file: PathBuf,

        /// Make a GET request to the specified path
        #[arg(long = "get", value_name = "PATH")]
        get_requests: Vec<String>,

        /// Make a POST request to the specified path
        #[arg(long = "post", value_name = "PATH")]
        post_requests: Vec<String>,

        /// Make a PUT request to the specified path
        #[arg(long = "put", value_name = "PATH")]
        put_requests: Vec<String>,

        /// Make a DELETE request to the specified path
        #[arg(long = "delete", value_name = "PATH")]
        delete_requests: Vec<String>,

        /// Request body for POST/PUT requests (applies to the preceding request)
        #[arg(long = "body", value_name = "JSON")]
        body: Option<String>,

        /// Port to run the test server on (default: 18080)
        #[arg(long = "port", default_value = "18080")]
        port: u16,

        /// Show verbose output including headers
        #[arg(long = "verbose", short = 'v')]
        verbose: bool,
    },
    /// Parse and display the AST (for debugging)
    ///
    /// Internal command for compiler development. Shows the abstract syntax tree.
    #[command(hide = true)]
    Parse {
        /// The source file to parse
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Tokenize and display tokens (for debugging)
    ///
    /// Internal command for compiler development. Shows lexer output.
    #[command(hide = true)]
    Lex {
        /// The source file to tokenize
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
    /// Quick syntax check (use 'lint' for comprehensive analysis)
    ///
    /// Parses the file and reports any syntax errors. For more thorough
    /// analysis including style issues and common mistakes, use 'ntnt lint'.
    ///
    /// Examples:
    ///   ntnt check app.tnt
    Check {
        /// The source file to check
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
    /// Inspect a project and output JSON structure (for agents and tools)
    ///
    /// Outputs a JSON description of:
    /// - All functions with their parameters, return types, and contracts
    /// - HTTP routes registered
    /// - Imports and exports
    /// - Structs and enums
    ///
    /// Examples:
    ///   ntnt inspect app.tnt
    ///   ntnt inspect app.tnt --pretty
    Inspect {
        /// The source file or directory to inspect
        #[arg(value_name = "PATH")]
        path: PathBuf,

        /// Pretty-print the JSON output
        #[arg(long, short)]
        pretty: bool,
    },
    /// Validate source files for errors (outputs JSON for tools)
    ///
    /// Checks syntax, imports, and contracts without running.
    /// Outputs JSON with detailed error information.
    ///
    /// Examples:
    ///   ntnt validate app.tnt
    ///   ntnt validate routes/
    Validate {
        /// The source file or directory to validate
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
    /// Lint source files for common issues and style problems
    ///
    /// Performs comprehensive analysis to catch common mistakes:
    /// - Route patterns without raw strings (should use r"/path/{id}")
    /// - Potential map literal confusion (suggests map {} when appropriate)
    /// - Missing contracts on public functions
    /// - Unused imports
    /// - And more...
    ///
    /// Outputs JSON with suggestions and auto-fix hints.
    ///
    /// Examples:
    ///   ntnt lint app.tnt
    ///   ntnt lint routes/ --fix
    ///   ntnt lint . --quiet
    Lint {
        /// The source file or directory to lint
        #[arg(value_name = "PATH")]
        path: PathBuf,

        /// Show only errors, not warnings or suggestions
        #[arg(long, short)]
        quiet: bool,

        /// Output auto-fix suggestions as JSON patch
        #[arg(long)]
        fix: bool,

        /// Enable strict type checking (require type annotations on all functions)
        #[arg(long, conflicts_with = "warn_untyped")]
        strict: bool,

        /// Warn about untyped function signatures (non-fatal)
        #[arg(long, conflicts_with = "strict")]
        warn_untyped: bool,
    },
    /// Intent-Driven Development commands
    ///
    /// Verify that code matches human intent specifications.
    /// Intent files (.intent) define requirements as executable tests.
    ///
    /// Examples:
    ///   ntnt intent check server.tnt
    ///   ntnt intent check server.tnt --intent custom.intent
    #[command(subcommand)]
    Intent(IntentCommands),

    /// Browse and validate stdlib documentation
    ///
    /// Documentation is auto-generated from source code // @ntnt comments.
    /// Use --validate to check all functions are documented.
    ///
    /// Examples:
    ///   ntnt docs                    # List all modules
    ///   ntnt docs std/string         # Show string module docs
    ///   ntnt docs split              # Search for a function
    ///   ntnt docs --validate         # Check documentation coverage
    ///   ntnt docs --generate         # Regenerate STDLIB_REFERENCE.md
    Docs {
        /// Module or function to look up (e.g., "std/string", "split")
        #[arg(value_name = "QUERY")]
        query: Option<String>,

        /// Validate that all stdlib functions are documented
        #[arg(long)]
        validate: bool,

        /// Regenerate docs/STDLIB_REFERENCE.md from source annotations
        #[arg(long)]
        generate: bool,

        /// Output as JSON (for tooling)
        #[arg(long)]
        json: bool,
    },

    /// Set up AI agent configuration for NTNT development
    ///
    /// Generates platform-specific config files so your AI coding agent
    /// understands NTNT syntax. Run this after installing ntnt.
    ///
    /// Examples:
    ///   ntnt learn claude-code    # Generate .claude/CLAUDE.md + .claude/rules/ntnt.md
    ///   ntnt learn cursor         # Generate .cursorrules
    ///   ntnt learn codex          # Generate AGENTS.md
    ///   ntnt learn copilot        # Update .github/copilot-instructions.md
    ///   ntnt learn                # Show guide to stdout (any agent)
    ///   ntnt learn --check        # Check if config files are up to date
    ///   ntnt learn --update       # Update all existing config files
    Learn {
        /// Target platform (claude-code, cursor, codex, copilot)
        #[arg(value_name = "PLATFORM")]
        platform: Option<String>,

        /// Check if existing config files match current ntnt version
        #[arg(long)]
        check: bool,

        /// Update all existing config files to current version
        #[arg(long)]
        update: bool,
    },

    /// Generate shell completion scripts
    ///
    /// Output completion scripts for your shell. Add the output to your
    /// shell configuration file.
    ///
    /// Examples:
    ///   # Bash (add to ~/.bashrc)
    ///   ntnt completions bash >> ~/.bashrc
    ///
    ///   # Zsh (add to ~/.zshrc)
    ///   ntnt completions zsh >> ~/.zshrc
    ///
    ///   # Fish (save to completions dir)
    ///   ntnt completions fish > ~/.config/fish/completions/ntnt.fish
    ///
    ///   # PowerShell
    ///   ntnt completions powershell >> $PROFILE
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}

/// Intent-Driven Development subcommands
#[derive(Subcommand)]
enum IntentCommands {
    /// Check that code matches its intent specification
    ///
    /// Runs all tests defined in the .intent file against the NTNT program.
    /// Looks for <name>.intent file automatically, or specify with --intent.
    ///
    /// Verbosity levels:
    ///   (default) - Summary: feature pass/fail counts only
    ///   -v        - Scenarios: show each scenario's pass/fail status
    ///   -vv       - Assertions: show all assertions and how terms resolved
    ///
    /// Examples:
    ///   ntnt intent check server.tnt
    ///   ntnt intent check server.tnt -v
    ///   ntnt intent check server.tnt -vv
    ///   ntnt intent check server.tnt --intent requirements.intent
    Check {
        /// The NTNT source file to check
        #[arg(value_name = "FILE")]
        file: PathBuf,

        /// Path to intent file (default: looks for <name>.intent)
        #[arg(long = "intent", short = 'i')]
        intent_file: Option<PathBuf>,

        /// Port to run the test server on (default: 18081)
        #[arg(long = "port", default_value = "18081")]
        port: u16,

        /// Increase output verbosity (-v for scenarios, -vv for assertions)
        #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
        verbose: u8,

        /// Output results as JSON (for programmatic access)
        #[arg(long)]
        json: bool,
    },
    /// Show implementation coverage of intent features
    ///
    /// Analyzes source code for @implements annotations and shows
    /// which features from the intent file have implementations.
    ///
    /// Examples:
    ///   ntnt intent coverage server.tnt
    ///   ntnt intent coverage server.tnt --intent requirements.intent
    Coverage {
        /// The NTNT source file(s) to analyze
        #[arg(value_name = "FILE")]
        file: PathBuf,

        /// Path to intent file (default: looks for <name>.intent)
        #[arg(long = "intent", short = 'i')]
        intent_file: Option<PathBuf>,
    },
    /// Generate code scaffolding from an intent file
    ///
    /// Creates a new .tnt file with function stubs and route
    /// registrations based on the intent specification.
    ///
    /// Examples:
    ///   ntnt intent init requirements.intent
    ///   ntnt intent init requirements.intent -o server.tnt
    Init {
        /// The intent file to generate code from
        #[arg(value_name = "INTENT_FILE")]
        intent_file: PathBuf,

        /// Output file (default: prints to stdout)
        #[arg(long = "output", short = 'o')]
        output: Option<PathBuf>,
    },
    /// Start Intent Studio - a visual workspace for developing intent
    ///
    /// Opens a beautiful HTML view of your intent file that auto-refreshes
    /// as you edit. Perfect for collaborative intent development with AI.
    ///
    /// Examples:
    ///   ntnt intent studio server.intent
    ///   ntnt intent studio server.intent --port 4000 --app-port 9000
    Studio {
        /// The intent file to visualize
        #[arg(value_name = "INTENT_FILE")]
        intent_file: PathBuf,

        /// Port to run the studio server on (default: 3001)
        #[arg(long = "port", short = 'p', default_value = "3001")]
        port: u16,

        /// Port where the application server is running (default: 8081)
        #[arg(long = "app-port", short = 'a', default_value = "8081")]
        app_port: u16,

        /// Don't automatically open the browser
        #[arg(long = "no-open")]
        no_open: bool,
    },
}

/// Format and display an error with rich context (error codes, source snippets, suggestions).
fn format_error(error: &anyhow::Error, file_path: Option<&PathBuf>) {
    // Try to downcast to IntentError for rich formatting
    if let Some(intent_err) = error.downcast_ref::<IntentError>() {
        let code = intent_err.error_code();
        let line_info = intent_err.line();
        let col_info = intent_err.column();

        // Error header: error[E006]: Undefined variable: usres
        eprintln!(
            "{}{}{}{}",
            "error".red().bold(),
            "[".dimmed(),
            code.red().bold(),
            "]".dimmed(),
        );
        // Show file:line location if available
        if let (Some(line), Some(path)) = (line_info, file_path) {
            let display_path = path.display();
            eprintln!("  {} {}:{}", "-->".blue().bold(), display_path, line);
        }
        eprintln!("  {} {}", "=".blue().bold(), intent_err);

        // Source code snippet (for errors with line numbers and a known file)
        if let (Some(line), Some(path)) = (line_info, file_path) {
            if let Ok(source) = fs::read_to_string(path) {
                let lines: Vec<&str> = source.lines().collect();
                let line_idx = line.saturating_sub(1); // 0-indexed

                eprintln!("   {}", "|".blue().bold());

                // Show 1 line before for context
                if line_idx > 0 {
                    let prev_num = line;
                    if let Some(prev_line) = lines.get(line_idx - 1) {
                        eprintln!(
                            " {} {} {}",
                            format!("{:>3}", prev_num - 1).blue(),
                            "|".blue().bold(),
                            prev_line
                        );
                    }
                }

                // Show the error line
                if let Some(error_line) = lines.get(line_idx) {
                    eprintln!(
                        " {} {} {}",
                        format!("{:>3}", line).blue(),
                        "|".blue().bold(),
                        error_line
                    );

                    // Column pointer
                    if let Some(col) = col_info {
                        if col > 0 {
                            let padding = " ".repeat(col.saturating_sub(1));
                            eprintln!("     {} {}{}", "|".blue().bold(), padding, "^".red().bold());
                        }
                    }
                }

                // Show 1 line after for context
                if let Some(next_line) = lines.get(line_idx + 1) {
                    eprintln!(
                        " {} {} {}",
                        format!("{:>3}", line + 1).blue(),
                        "|".blue().bold(),
                        next_line
                    );
                }

                eprintln!("   {}", "|".blue().bold());
            }
        }

        // Rich type context (expected/got)
        if let Some(ctx) = intent_err.type_context() {
            eprintln!("  {} {}", "expected:".cyan().bold(), ctx.expected.green());
            eprintln!("     {} {}", "found:".cyan().bold(), ctx.got.red());
            if let Some(hint) = &ctx.hint {
                eprintln!("      {} {}", "hint:".cyan().bold(), hint);
            }
        }

        // Suggestion ("Did you mean?")
        if let Some(suggestion) = intent_err.suggestion() {
            eprintln!(
                "  {} Did you mean '{}'?",
                "help:".cyan().bold(),
                suggestion.green()
            );
        }
    } else {
        // Non-IntentError: fall back to simple display
        eprintln!("{}: {}", "Error".red().bold(), error);
    }
}

fn main() {
    let cli = Cli::parse();

    // Extract the file path for error context (used in format_error for source snippets)
    let file_hint: Option<PathBuf> = match &cli.command {
        Some(Commands::Run { file, .. }) => Some(file.clone()),
        Some(Commands::Check { file }) => Some(file.clone()),
        Some(Commands::Parse { file, .. }) => Some(file.clone()),
        Some(Commands::Lex { file }) => Some(file.clone()),
        None => cli.file.clone(),
        _ => None,
    };

    let result = match cli.command {
        Some(Commands::Repl) => run_repl(),
        Some(Commands::Run { file, timeout }) => run_file(&file, timeout),
        Some(Commands::Test {
            file,
            get_requests,
            post_requests,
            put_requests,
            delete_requests,
            body,
            port,
            verbose,
        }) => test_http_server(
            &file,
            get_requests,
            post_requests,
            put_requests,
            delete_requests,
            body,
            port,
            verbose,
        ),
        Some(Commands::Parse { file, json }) => parse_file(&file, json),
        Some(Commands::Lex { file }) => lex_file(&file),
        Some(Commands::Check { file }) => check_file(&file),
        Some(Commands::Inspect { path, pretty }) => inspect_project(&path, pretty),
        Some(Commands::Validate { path }) => validate_project(&path),
        Some(Commands::Lint {
            path,
            quiet,
            fix,
            strict,
            warn_untyped,
        }) => lint_project(&path, quiet, fix, strict, warn_untyped),
        Some(Commands::Intent(intent_cmd)) => run_intent_command(intent_cmd),
        Some(Commands::Docs {
            query,
            validate,
            generate,
            json,
        }) => run_docs_command(query, validate, generate, json),
        Some(Commands::Learn {
            platform,
            check,
            update,
        }) => run_learn_command(platform, check, update),
        Some(Commands::Completions { shell }) => {
            generate_completions(shell);
            Ok(())
        }
        None => {
            if let Some(file) = cli.file {
                run_file(&file, 30)
            } else {
                run_repl()
            }
        }
    };

    if let Err(e) = result {
        format_error(&e, file_hint.as_ref());
        std::process::exit(1);
    }
}

fn run_repl() -> anyhow::Result<()> {
    println!(
        "{}",
        format!(
            "NTNT (Intent) Programming Language v{}",
            env!("CARGO_PKG_VERSION")
        )
        .green()
        .bold()
    );
    println!(
        "Type {} for help, {} to exit\n",
        ":help".cyan(),
        ":quit".cyan()
    );

    let mut rl = DefaultEditor::new()?;
    let mut interpreter = Interpreter::new();

    loop {
        let readline = rl.readline(&format!("{} ", "ntnt>".blue().bold()));
        match readline {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let _ = rl.add_history_entry(line);

                // Handle REPL commands
                if line.starts_with(':') {
                    match line {
                        ":quit" | ":q" | ":exit" => {
                            println!("Goodbye!");
                            break;
                        }
                        ":help" | ":h" => {
                            print_repl_help();
                            continue;
                        }
                        ":clear" => {
                            interpreter = Interpreter::new();
                            println!("Environment cleared.");
                            continue;
                        }
                        ":env" => {
                            interpreter.print_environment();
                            continue;
                        }
                        _ if line.starts_with(":doc ") => {
                            let query = line[5..].trim();
                            if query.is_empty() {
                                println!("{}: Usage: :doc <function_name>", "Error".red());
                            } else if let Some(entry) = search_docs(query) {
                                let _ = show_doc_entry(entry, false);
                            } else {
                                // Fuzzy search: find functions containing the query
                                let matches: Vec<&docs::DocEntry> = get_docs()
                                    .iter()
                                    .filter(|d| {
                                        d.name.to_lowercase().contains(&query.to_lowercase())
                                    })
                                    .collect();
                                if matches.is_empty() {
                                    // Try Levenshtein "Did you mean?" suggestion
                                    let candidates: Vec<String> =
                                        get_docs().iter().map(|d| d.name.clone()).collect();
                                    if let Some(suggestion) =
                                        ntnt::error::find_suggestion(query, &candidates)
                                    {
                                        println!(
                                            "No documentation found for '{}'. Did you mean {}?",
                                            query,
                                            suggestion.green()
                                        );
                                    } else {
                                        println!("No documentation found for '{}'", query);
                                    }
                                } else if matches.len() == 1 {
                                    let _ = show_doc_entry(matches[0], false);
                                } else {
                                    println!(
                                        "Found {} matches for '{}':\n",
                                        matches.len().to_string().green(),
                                        query
                                    );
                                    for entry in &matches {
                                        let module = entry.module.as_deref().unwrap_or("builtin");
                                        println!(
                                            "  {} ({})",
                                            entry.name.yellow().bold(),
                                            module.dimmed()
                                        );
                                        if let Some(sig) = &entry.signature {
                                            println!("    {}", sig.cyan());
                                        }
                                    }
                                    println!("\nUse {} for full details", ":doc <name>".cyan());
                                }
                            }
                            continue;
                        }
                        _ if line.starts_with(":search ") => {
                            let query = line[8..].trim().trim_matches('"');
                            if query.is_empty() {
                                println!("{}: Usage: :search <query>", "Error".red());
                            } else {
                                let query_lower = query.to_lowercase();
                                let matches: Vec<&docs::DocEntry> = get_docs()
                                    .iter()
                                    .filter(|d| {
                                        d.name.to_lowercase().contains(&query_lower)
                                            || d.summary.to_lowercase().contains(&query_lower)
                                            || d.description
                                                .as_ref()
                                                .map(|desc| {
                                                    desc.to_lowercase().contains(&query_lower)
                                                })
                                                .unwrap_or(false)
                                    })
                                    .collect();
                                if matches.is_empty() {
                                    println!("No results for '{}'", query);
                                } else {
                                    println!(
                                        "Found {} results for '{}':\n",
                                        matches.len().to_string().green(),
                                        query
                                    );
                                    for entry in &matches {
                                        let module = entry.module.as_deref().unwrap_or("builtin");
                                        println!(
                                            "  {} ({})",
                                            entry.name.yellow().bold(),
                                            module.dimmed()
                                        );
                                        println!("    {}", entry.summary);
                                    }
                                }
                            }
                            continue;
                        }
                        _ => {
                            println!("{}: Unknown command: {}", "Error".red(), line);
                            continue;
                        }
                    }
                }

                // Parse and evaluate
                match evaluate(&mut interpreter, line) {
                    Ok(result) => {
                        if !result.is_empty() {
                            println!("{}", result.green());
                        }
                    }
                    Err(e) => {
                        println!("{}: {}", "Error".red(), e);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("Goodbye!");
                break;
            }
            Err(err) => {
                println!("{}: {:?}", "Error".red(), err);
                break;
            }
        }
    }

    Ok(())
}

fn print_repl_help() {
    println!("{}", "\nREPL Commands:".yellow().bold());
    println!("  {}       Show this help", ":help, :h".cyan());
    println!("  {}  Exit the REPL", ":quit, :q, :exit".cyan());
    println!("  {}      Clear the environment", ":clear".cyan());
    println!("  {}        Show current bindings", ":env".cyan());
    println!(
        "  {}   Look up function documentation",
        ":doc <name>".cyan()
    );
    println!(
        "  {}  Search docs by name or description",
        ":search <q>".cyan()
    );

    println!("\n{}", "Standard Library:".yellow().bold());
    println!(
        "  {}      split, join, trim, replace, contains, to_upper...",
        "std/string".cyan()
    );
    println!(
        "  {}        sin, cos, log, exp, random, random_int, PI, E...",
        "std/math".cyan()
    );
    println!(
        "  {} json, html, text, redirect, parse_form, parse_json",
        "std/http/server".cyan()
    );
    println!("  {}        fetch, download", "std/http".cyan());
    println!(
        "  {} push, pop, keys, values, first, last, has_key...",
        "std/collections".cyan()
    );
    println!(
        "  {}        parse_json, stringify, stringify_pretty",
        "std/json".cyan()
    );
    println!(
        "  {}         parse_csv, parse_with_headers, stringify...",
        "std/csv".cyan()
    );
    println!(
        "  {}          read_file, write_file, exists, mkdir, readdir",
        "std/fs".cyan()
    );
    println!(
        "  {}         get_env, set_env, load_env, args, cwd",
        "std/env".cyan()
    );
    println!(
        "  {}        join, dirname, basename, extname",
        "std/path".cyan()
    );
    println!(
        "  {}         encode, decode, parse_url, parse_query...",
        "std/url".cyan()
    );
    println!(
        "  {}        now, format, parse_datetime, add_days...",
        "std/time".cyan()
    );
    println!(
        "  {}  channel, send, recv, sleep_ms",
        "std/concurrent".cyan()
    );
    println!("  Run {} for full documentation", "ntnt docs".green());

    println!("\n{}", "Quick Reference:".yellow().bold());
    println!("  {}           Variable binding", "let x = 42".cyan());
    println!("  {}      Mutable variable", "let mut x = 0".cyan());
    println!("  {} Function definition", "fn add(a, b) { a + b }".cyan());
    println!("  {}   Map literal (not {{}})", r#"map { "a": 1 }"#.cyan());
    println!("  {}    String interpolation", r#""Hello, {name}""#.cyan());
    println!("  {}   Raw string (routes)", r#"r"/users/{id}""#.cyan());
    println!("  {}     For loop", "for i in 0..10 { }".cyan());
    println!("  {}    Pattern match", "match x { Some(v) => v }".cyan());

    println!("\n{}", "Global Builtins:".yellow().bold());
    println!("  len, str, int, float, type, print, push, assert, abs, min, max, round, floor");

    println!(
        "\n{}",
        "HTTP Server (globals - no import needed):".yellow().bold()
    );
    println!("  get, post, put, delete, listen, serve_static, routes, template, use_middleware");

    println!("\n{}", "Imports:".yellow().bold());
    println!("  {}", r#"import { split, join } from "std/string""#.cyan());
    println!("  {}", r#"import "std/math" as math"#.cyan());
    println!();
}

fn evaluate(interpreter: &mut Interpreter, source: &str) -> anyhow::Result<String> {
    let lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer.collect();

    let mut parser = IntentParser::new(tokens);
    let ast = parser.parse()?;

    let result = interpreter.eval(&ast)?;
    Ok(result.to_string())
}

fn run_file(path: &PathBuf, timeout: u64) -> anyhow::Result<()> {
    let source = fs::read_to_string(path)?;
    let mut interpreter = Interpreter::new();

    // Set the current file path for imports and hot-reload
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());
    let path_str = canonical_path.to_string_lossy();
    interpreter.set_current_file(&path_str);
    interpreter.set_main_source_file(&path_str);

    // Set request timeout for HTTP server
    interpreter.set_request_timeout(timeout);

    let lexer = Lexer::new(&source);
    let tokens: Vec<_> = lexer.collect();

    let mut parser = IntentParser::new(tokens);
    let ast = parser.parse()?;

    // Strict type checking: block execution if type errors found
    if let Some(errors) = ntnt::typechecker::strict_check_with_file(&ast, &source, Some(&path_str))
    {
        for diag in &errors {
            let location = if diag.line > 0 {
                format!(" (line {})", diag.line)
            } else {
                String::new()
            };
            eprintln!(
                "{}: {}{}",
                "type error".red().bold(),
                diag.message,
                location
            );
            if let Some(hint) = &diag.hint {
                eprintln!("  {}: {}", "hint".cyan(), hint);
            }
        }
        eprintln!(
            "\n{}: {} type error(s) found. Fix them or unset NTNT_STRICT to run anyway.",
            "blocked".red().bold(),
            errors.len()
        );
        std::process::exit(1);
    }

    interpreter.eval(&ast)?;
    Ok(())
}

/// Test mode: runs an HTTP server, makes requests, then exits
fn test_http_server(
    path: &PathBuf,
    get_requests: Vec<String>,
    post_requests: Vec<String>,
    put_requests: Vec<String>,
    delete_requests: Vec<String>,
    body: Option<String>,
    port: u16,
    verbose: bool,
) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    // Build list of requests to make
    let mut requests: Vec<(String, String, Option<String>)> = Vec::new();

    for path in get_requests {
        requests.push(("GET".to_string(), path, None));
    }
    for path in post_requests {
        requests.push(("POST".to_string(), path, body.clone()));
    }
    for path in put_requests {
        requests.push(("PUT".to_string(), path, body.clone()));
    }
    for path in delete_requests {
        requests.push(("DELETE".to_string(), path, None));
    }

    if requests.is_empty() {
        anyhow::bail!(
            "No requests specified. Use --get, --post, --put, or --delete to specify requests."
        );
    }

    println!("{}", "=== NTNT HTTP Test Mode ===".green().bold());
    println!();

    // Counters for tracking
    let requests_to_make = requests.len();
    let requests_completed = Arc::new(AtomicUsize::new(0));
    let shutdown_flag = Arc::new(AtomicBool::new(false));

    // Prepare results storage
    let results: Arc<std::sync::Mutex<Vec<(String, String, u16, String)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    // Clone for request thread
    let requests_completed_clone = requests_completed.clone();
    let shutdown_flag_clone = shutdown_flag.clone();
    let results_clone = results.clone();

    // Spawn thread to make HTTP requests after a short delay
    let request_handle = thread::spawn(move || {
        // Wait for server to start
        thread::sleep(Duration::from_millis(200));

        for (method, req_path, req_body) in requests {
            let path_with_slash = if req_path.starts_with('/') {
                req_path.clone()
            } else {
                format!("/{}", req_path)
            };

            let body_content = req_body.unwrap_or_default();
            let request = if body_content.is_empty() {
                format!(
                    "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
                    method, path_with_slash, port
                )
            } else {
                format!(
                    "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    method, path_with_slash, port, body_content.len(), body_content
                )
            };

            // Try to connect with retries
            let mut attempts = 0;
            let max_attempts = 10;
            let mut response_data = None;

            while attempts < max_attempts {
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
                                let headers = parts.get(0).unwrap_or(&"");
                                let body = parts.get(1).unwrap_or(&"").to_string();

                                let status_code = headers
                                    .lines()
                                    .next()
                                    .unwrap_or("")
                                    .split_whitespace()
                                    .nth(1)
                                    .unwrap_or("0")
                                    .parse::<u16>()
                                    .unwrap_or(0);

                                response_data =
                                    Some((method.clone(), req_path.clone(), status_code, body));
                                break;
                            }
                        }
                    }
                    Err(_) => {}
                }
                attempts += 1;
                thread::sleep(Duration::from_millis(100));
            }

            if let Some(data) = response_data {
                results_clone.lock().unwrap().push(data);
            } else {
                results_clone.lock().unwrap().push((
                    method.clone(),
                    req_path.clone(),
                    0,
                    "Connection failed".to_string(),
                ));
            }

            requests_completed_clone.fetch_add(1, Ordering::SeqCst);
        }

        // Signal shutdown after all requests complete
        shutdown_flag_clone.store(true, Ordering::SeqCst);
    });

    // Parse and run the server in main thread
    let source = fs::read_to_string(path)?;
    let mut interpreter = Interpreter::new();

    // Set the current file path for routes() and serve_static() path resolution
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());
    let path_str = canonical_path.to_string_lossy();
    interpreter.set_current_file(&path_str);
    interpreter.set_main_source_file(&path_str);

    interpreter.set_test_mode(port, requests_to_make, shutdown_flag.clone());

    let lexer = Lexer::new(&source);
    let tokens: Vec<_> = lexer.collect();

    let mut parser = IntentParser::new(tokens);
    let ast = parser.parse()?;

    // Run the server (will exit when shutdown_flag is set)
    let _ = interpreter.eval(&ast);

    // Wait for request thread to finish
    request_handle.join().ok();

    // Print results
    println!();
    let results_vec = results.lock().unwrap();
    let mut passed = 0;
    let mut failed = 0;

    for (i, (method, path, status, body)) in results_vec.iter().enumerate() {
        let req_num = i + 1;
        println!(
            "{}",
            format!("[REQUEST {}] {} {}", req_num, method, path)
                .cyan()
                .bold()
        );

        let is_success = *status >= 200 && *status < 400;

        if verbose {
            println!("{}", format!("[STATUS] {}", status).yellow());
        }

        let status_display = if is_success {
            format!("[RESPONSE] {} ({})", status, "OK".green())
        } else if *status == 0 {
            format!("[RESPONSE] {} ({})", "FAILED", "Connection Error".red())
        } else {
            format!("[RESPONSE] {} ({})", status, "ERROR".red())
        };
        println!("{}", status_display);

        // Pretty print JSON if possible
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
            println!(
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_else(|_| body.to_string())
            );
        } else {
            println!("{}", body);
        }

        if is_success {
            passed += 1;
        } else {
            failed += 1;
        }

        println!();
    }

    // Summary
    let total = results_vec.len();
    let summary = format!(
        "=== {} requests, {} passed, {} failed ===",
        total, passed, failed
    );
    if failed == 0 {
        println!("{}", summary.green().bold());
    } else {
        println!("{}", summary.red().bold());
    }

    println!("Server shutdown.");

    if failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}

fn parse_file(path: &PathBuf, json: bool) -> anyhow::Result<()> {
    let source = fs::read_to_string(path)?;

    let lexer = Lexer::new(&source);
    let tokens: Vec<_> = lexer.collect();

    let mut parser = IntentParser::new(tokens);
    let ast = parser.parse()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&ast)?);
    } else {
        println!("{:#?}", ast);
    }

    Ok(())
}

fn lex_file(path: &PathBuf) -> anyhow::Result<()> {
    let source = fs::read_to_string(path)?;

    let lexer = Lexer::new(&source);
    for token in lexer {
        println!("{:?}", token);
    }

    Ok(())
}

fn check_file(path: &PathBuf) -> anyhow::Result<()> {
    let source = fs::read_to_string(path)?;

    let lexer = Lexer::new(&source);
    let tokens: Vec<_> = lexer.collect();

    let mut parser = IntentParser::new(tokens);
    let _ast = parser.parse()?;

    println!("{} No errors found in {}", "✓".green(), path.display());
    Ok(())
}
/// Inspect a project and output JSON structure
///
/// This extracts metadata from NTNT files including:
/// - Functions (name, params, return type, contracts, line number)
/// - HTTP routes (method, path, handler, line number)
/// - Middleware registrations
/// - Static file directories
/// - Structs and enums
/// - Imports/exports
fn inspect_project(path: &PathBuf, pretty: bool) -> anyhow::Result<()> {
    use ntnt::ast::Statement;
    use serde_json::{json, Value as JsonValue};

    // Collect all .tnt files
    let files = collect_tnt_files(path)?;

    let mut functions: Vec<JsonValue> = Vec::new();
    let mut routes: Vec<JsonValue> = Vec::new();
    let mut structs: Vec<JsonValue> = Vec::new();
    let mut enums: Vec<JsonValue> = Vec::new();
    let mut imports: Vec<JsonValue> = Vec::new();
    let mut middleware: Vec<JsonValue> = Vec::new();
    let mut static_dirs: Vec<JsonValue> = Vec::new();

    for file_path in &files {
        let source = fs::read_to_string(file_path)?;
        let lexer = Lexer::new(&source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = IntentParser::new(tokens);

        let ast = match parser.parse() {
            Ok(ast) => ast,
            Err(e) => {
                eprintln!(
                    "{}: Failed to parse {}: {}",
                    "Warning".yellow(),
                    file_path.display(),
                    e
                );
                continue;
            }
        };

        let relative_path = file_path
            .strip_prefix(path.parent().unwrap_or(path))
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();

        // Build a map of function names to line numbers by scanning source
        let line_map = build_line_number_map(&source);

        for stmt in &ast.statements {
            match unwrap_located(stmt) {
                Statement::Function {
                    name,
                    params,
                    return_type,
                    contract,
                    attributes,
                    ..
                } => {
                    let line = line_map.get(&format!("fn {}", name)).copied();
                    let func_json = json!({
                        "name": name,
                        "file": relative_path,
                        "line": line,
                        "params": params.iter().map(|p| param_to_json(p)).collect::<Vec<_>>(),
                        "return_type": return_type.as_ref().map(|t| type_to_string(t)),
                        "contracts": contract_to_json(contract),
                        "attributes": attributes.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
                    });
                    functions.push(func_json);
                }
                Statement::Struct {
                    name,
                    fields,
                    type_params,
                    ..
                } => {
                    let line = line_map.get(&format!("struct {}", name)).copied();
                    let struct_json = json!({
                        "name": name,
                        "file": relative_path,
                        "line": line,
                        "fields": fields.iter().map(|f| json!({
                            "name": f.name,
                            "type": type_to_string(&f.type_annotation),
                            "public": f.public,
                        })).collect::<Vec<_>>(),
                        "type_params": type_params.iter().map(|tp| tp.name.clone()).collect::<Vec<_>>(),
                    });
                    structs.push(struct_json);
                }
                Statement::Enum {
                    name,
                    variants,
                    type_params,
                    ..
                } => {
                    let line = line_map.get(&format!("enum {}", name)).copied();
                    let enum_json = json!({
                        "name": name,
                        "file": relative_path,
                        "line": line,
                        "variants": variants.iter().map(|v| v.name.clone()).collect::<Vec<_>>(),
                        "type_params": type_params.iter().map(|tp| tp.name.clone()).collect::<Vec<_>>(),
                    });
                    enums.push(enum_json);
                }
                Statement::Import {
                    items,
                    source,
                    alias,
                } => {
                    let import_json = json!({
                        "source": source,
                        "items": items.iter().map(|i| i.name.clone()).collect::<Vec<_>>(),
                        "alias": alias,
                        "file": relative_path,
                    });
                    imports.push(import_json);
                }
                // Detect HTTP route, middleware, and static registrations
                Statement::Expression(expr) => {
                    if let Some(route) = extract_route_with_line(expr, &relative_path, &source) {
                        routes.push(route);
                    }
                    if let Some(mw) = extract_middleware(expr, &relative_path, &source) {
                        middleware.push(mw);
                    }
                    if let Some(sd) = extract_static_dir(expr, &relative_path, &source) {
                        static_dirs.push(sd);
                    }
                }
                _ => {}
            }
        }

        // Detect file-based routes (functions named get, post, etc. in routes/ directory)
        if relative_path.contains("/routes/") || relative_path.starts_with("routes/") {
            let url_path = file_path_to_url(&relative_path);
            let http_methods = ["get", "post", "put", "delete", "patch", "head", "options"];

            for stmt in &ast.statements {
                if let Statement::Function { name, .. } = unwrap_located(stmt) {
                    let method = name.to_lowercase();
                    if http_methods.contains(&method.as_str()) {
                        let line = line_map.get(&format!("fn {}", name)).copied();
                        let route = json!({
                            "method": method.to_uppercase(),
                            "path": url_path,
                            "file": relative_path.clone(),
                            "line": line,
                            "routing": "file-based",
                        });
                        routes.push(route);
                    }
                }
            }
        }
    }

    let output = json!({
        "files": files.iter().map(|f| f.strip_prefix(path.parent().unwrap_or(path))
            .unwrap_or(f).to_string_lossy().to_string()).collect::<Vec<_>>(),
        "functions": functions,
        "routes": routes,
        "middleware": middleware,
        "static": static_dirs,
        "structs": structs,
        "enums": enums,
        "imports": imports,
        "syntax_reference": {
            "critical_rules": {
                "map_literals": "Use `map { \"key\": value }` NOT `{ \"key\": value }` - bare {} creates blocks",
                "route_patterns": "Route builtins auto-detect {param} patterns: `get(\"/users/{id}\", handler)` - raw strings optional",
                "string_interpolation": "Use `\"{variable}\"` for interpolation, NOT `${variable}` or backticks",
                "ranges": "Use `0..10` (exclusive) or `0..=10` (inclusive), NOT range()",
                "imports": "Use `import { x } from \"std/module\"` with `/` separator",
                "contracts": "Place requires/ensures AFTER return type, BEFORE body",
                "mutability": "Use `let mut x` for mutable variables"
            },
            "builtin_functions": ["print", "len", "str", "abs", "min", "max", "sqrt", "pow", "round", "floor", "ceil", "Some", "None", "Ok", "Err", "unwrap", "unwrap_or", "is_some", "is_none", "is_ok", "is_err"],
            "common_imports": {
                "std/string": ["split", "join", "trim", "replace", "replace_first", "contains", "starts_with", "ends_with", "to_lower", "to_upper"],
                "std/collections": ["push", "pop", "map", "filter", "reduce", "first", "last"],
                "std/http": ["fetch", "post", "put", "delete", "get_json", "post_json"],
                "std/http_server": ["listen", "get", "post", "json", "html", "text", "redirect", "serve_static"],
                "std/fs": ["read_file", "write_file", "exists", "mkdir", "readdir"],
                "std/json": ["parse", "stringify", "stringify_pretty"],
                "std/time": ["now", "format", "add_days"],
                "std/concurrent": ["channel", "send", "recv", "sleep_ms"]
            }
        }
    });

    if pretty {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", serde_json::to_string(&output)?);
    }

    Ok(())
}

/// Build a map of declaration patterns to line numbers
/// Unwrap a `Statement::Located` wrapper, returning the inner statement.
/// Allows all AST-walking code to be transparent to source-location annotations.
fn unwrap_located(stmt: &ntnt::ast::Statement) -> &ntnt::ast::Statement {
    match stmt {
        ntnt::ast::Statement::Located { stmt, .. } => unwrap_located(stmt),
        other => other,
    }
}

fn build_line_number_map(source: &str) -> std::collections::HashMap<String, usize> {
    use std::collections::HashMap;
    let mut map = HashMap::new();

    for (line_num, line) in source.lines().enumerate() {
        let trimmed = line.trim();

        // Match function declarations: "fn name(" or "fn name<"
        if trimmed.starts_with("fn ") {
            if let Some(name_end) =
                trimmed[3..].find(|c: char| c == '(' || c == '<' || c.is_whitespace())
            {
                let name = &trimmed[3..3 + name_end];
                map.insert(format!("fn {}", name), line_num + 1);
            }
        }

        // Match struct declarations
        if trimmed.starts_with("struct ") {
            if let Some(name_end) =
                trimmed[7..].find(|c: char| c == '{' || c == '<' || c.is_whitespace())
            {
                let name = &trimmed[7..7 + name_end];
                map.insert(format!("struct {}", name), line_num + 1);
            }
        }

        // Match enum declarations
        if trimmed.starts_with("enum ") {
            if let Some(name_end) =
                trimmed[5..].find(|c: char| c == '{' || c == '<' || c.is_whitespace())
            {
                let name = &trimmed[5..5 + name_end];
                map.insert(format!("enum {}", name), line_num + 1);
            }
        }

        // Match route registrations: get("/path", ...) etc
        for method in &["get", "post", "put", "delete", "patch", "head"] {
            let prefix = format!("{}(", method);
            if trimmed.starts_with(&prefix) || trimmed.contains(&format!(" {}(", method)) {
                // Extract the path string
                if let Some(start) = trimmed.find('"') {
                    if let Some(end) = trimmed[start + 1..].find('"') {
                        let path = &trimmed[start + 1..start + 1 + end];
                        map.insert(
                            format!("route {} {}", method.to_uppercase(), path),
                            line_num + 1,
                        );
                    }
                }
            }
        }

        // Match middleware registrations
        if trimmed.starts_with("middleware(") || trimmed.contains(" middleware(") {
            map.insert(format!("middleware@{}", line_num), line_num + 1);
        }

        // Match serve_static registrations
        if trimmed.starts_with("serve_static(") || trimmed.contains(" serve_static(") {
            map.insert(format!("static@{}", line_num), line_num + 1);
        }
    }

    map
}

/// Extract HTTP route with line number
fn extract_route_with_line(
    expr: &ntnt::ast::Expression,
    file: &str,
    source: &str,
) -> Option<serde_json::Value> {
    use ntnt::ast::Expression;
    use serde_json::json;

    if let Expression::Call {
        function,
        arguments,
    } = expr
    {
        if let Expression::Identifier(method) = function.as_ref() {
            let http_methods = ["get", "post", "put", "delete", "patch", "head"];
            if http_methods.contains(&method.as_str()) && arguments.len() >= 2 {
                let path = match &arguments[0] {
                    Expression::String(s) => s.clone(),
                    // Route auto-detection: reconstruct {param} patterns from
                    // InterpolatedString (same logic as interpreter's eval_route_pattern)
                    Expression::InterpolatedString(parts) => {
                        use ntnt::ast::StringPart;
                        let mut result = String::new();
                        for part in parts {
                            match part {
                                StringPart::Literal(s) => result.push_str(s),
                                StringPart::Expr(inner) => {
                                    if let Expression::Identifier(name) = inner {
                                        result.push('{');
                                        result.push_str(name);
                                        result.push('}');
                                    } else {
                                        // Complex expression — can't resolve statically
                                        return None;
                                    }
                                }
                            }
                        }
                        result
                    }
                    _ => return None,
                };
                let handler = match &arguments[1] {
                    Expression::Identifier(name) => name.clone(),
                    Expression::Lambda { .. } => "<lambda>".to_string(),
                    _ => "<handler>".to_string(),
                };

                // Find line number
                let line_map = build_line_number_map(source);
                let line = line_map
                    .get(&format!("route {} {}", method.to_uppercase(), path))
                    .copied();

                return Some(json!({
                    "method": method.to_uppercase(),
                    "path": path,
                    "handler": handler,
                    "file": file,
                    "line": line,
                }));
            }
        }
    }
    None
}

/// Extract middleware registration
fn extract_middleware(
    expr: &ntnt::ast::Expression,
    file: &str,
    source: &str,
) -> Option<serde_json::Value> {
    use ntnt::ast::Expression;
    use serde_json::json;

    if let Expression::Call {
        function,
        arguments,
    } = expr
    {
        if let Expression::Identifier(name) = function.as_ref() {
            // Check for both "middleware" and "use_middleware"
            if (name == "middleware" || name == "use_middleware") && !arguments.is_empty() {
                let handler = match &arguments[0] {
                    Expression::Identifier(name) => name.clone(),
                    Expression::Lambda { .. } => "<lambda>".to_string(),
                    _ => "<handler>".to_string(),
                };

                // Find approximate line by searching source
                let line = find_call_line(source, "middleware");

                return Some(json!({
                    "handler": handler,
                    "file": file,
                    "line": line,
                }));
            }
        }
    }
    None
}

/// Extract static directory registration
fn extract_static_dir(
    expr: &ntnt::ast::Expression,
    file: &str,
    source: &str,
) -> Option<serde_json::Value> {
    use ntnt::ast::Expression;
    use serde_json::json;

    if let Expression::Call {
        function,
        arguments,
    } = expr
    {
        if let Expression::Identifier(name) = function.as_ref() {
            if name == "serve_static" && arguments.len() >= 2 {
                let prefix = match &arguments[0] {
                    Expression::String(s) => s.clone(),
                    _ => return None,
                };
                let directory = match &arguments[1] {
                    Expression::String(s) => s.clone(),
                    Expression::Identifier(var) => format!("${}", var), // Variable reference
                    _ => "<dir>".to_string(),
                };

                let line = find_call_line(source, "serve_static");

                return Some(json!({
                    "prefix": prefix,
                    "directory": directory,
                    "file": file,
                    "line": line,
                }));
            }
        }
    }
    None
}

/// Convert a file path in routes/ directory to a URL pattern
///
/// Examples:
/// - routes/index.tnt → /
/// - routes/about.tnt → /about
/// - routes/api/users/index.tnt → /api/users
/// - routes/api/users/[id].tnt → /api/users/{id}
fn file_path_to_url(path: &str) -> String {
    // Remove routes/ prefix
    let path = path
        .strip_prefix("routes/")
        .or_else(|| path.rsplit("/routes/").next())
        .unwrap_or(path);

    // Split into segments and process
    let mut segments: Vec<String> = Vec::new();

    for segment in path.split('/') {
        // Remove .tnt extension
        let segment = segment.strip_suffix(".tnt").unwrap_or(segment);

        // Skip index (represents directory root)
        if segment == "index" {
            continue;
        }

        // Skip parent directory parts
        if segment.is_empty() || segment == ".." {
            continue;
        }

        // Convert [param] to {param}
        let segment = if segment.starts_with('[') && segment.ends_with(']') {
            let param = &segment[1..segment.len() - 1];
            format!("{{{}}}", param)
        } else {
            segment.to_string()
        };

        segments.push(segment);
    }

    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    }
}

/// Find the line number of a function call in source
fn find_call_line(source: &str, call_name: &str) -> Option<usize> {
    let pattern = format!("{}(", call_name);
    for (line_num, line) in source.lines().enumerate() {
        if line.contains(&pattern) {
            return Some(line_num + 1);
        }
    }
    None
}

/// Validate a project and output JSON errors
fn validate_project(path: &PathBuf) -> anyhow::Result<()> {
    use serde_json::{json, Value as JsonValue};

    let files = collect_tnt_files(path)?;

    let mut results: Vec<JsonValue> = Vec::new();
    let mut error_count = 0;
    let mut warning_count = 0;

    for file_path in &files {
        let relative_path = file_path
            .strip_prefix(path.parent().unwrap_or(path))
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();

        let source = match fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                results.push(json!({
                    "file": relative_path,
                    "valid": false,
                    "errors": [{"message": format!("Could not read file: {}", e), "line": null}],
                }));
                error_count += 1;
                continue;
            }
        };

        let lexer = Lexer::new(&source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = IntentParser::new(tokens);

        match parser.parse() {
            Ok(ast) => {
                // Check for potential issues
                let mut warnings = analyze_ast_warnings(&ast, &source);

                // Run type checker (with file path for cross-file import resolution)
                let file_path_str = file_path.to_string_lossy();
                let type_diagnostics =
                    ntnt::typechecker::check_program_with_file(&ast, &source, &file_path_str);
                let mut type_errors = Vec::new();
                for diag in type_diagnostics {
                    let entry = json!({
                        "message": diag.message,
                        "line": if diag.line > 0 { Some(diag.line) } else { None::<usize> },
                        "hint": diag.hint,
                        "rule": "type_check",
                    });
                    match diag.severity {
                        ntnt::typechecker::Severity::Error => type_errors.push(entry),
                        ntnt::typechecker::Severity::Warning => warnings.push(entry),
                    }
                }
                let num_type_errors = type_errors.len();
                let num_warnings = warnings.len();
                error_count += num_type_errors;
                warning_count += num_warnings;

                results.push(json!({
                    "file": relative_path,
                    "valid": num_type_errors == 0,
                    "errors": type_errors,
                    "warnings": warnings,
                }));

                // Print success indicator
                if num_type_errors == 0 && num_warnings == 0 {
                    eprintln!("{} {}", "✓".green(), relative_path);
                } else if num_type_errors > 0 {
                    eprintln!(
                        "{} {} ({} type errors)",
                        "✗".red(),
                        relative_path,
                        num_type_errors
                    );
                } else {
                    eprintln!(
                        "{} {} ({} warnings)",
                        "⚠".yellow(),
                        relative_path,
                        num_warnings
                    );
                }
            }
            Err(e) => {
                let error_msg = e.to_string();
                // Try to extract line number from error
                let line = extract_line_from_error(&error_msg);

                results.push(json!({
                    "file": relative_path,
                    "valid": false,
                    "errors": [{"message": error_msg, "line": line}],
                    "warnings": [],
                }));
                error_count += 1;

                eprintln!("{} {}", "✗".red(), relative_path);
            }
        }
    }

    // Summary
    eprintln!();
    if error_count == 0 {
        eprintln!("{}", "All files valid!".green().bold());
    } else {
        eprintln!("{}: {}", "Errors".red().bold(), error_count);
    }
    if warning_count > 0 {
        eprintln!("{}: {}", "Warnings".yellow().bold(), warning_count);
    }

    // Output JSON
    let output = json!({
        "files": results,
        "summary": {
            "total": files.len(),
            "valid": files.len() - error_count,
            "errors": error_count,
            "warnings": warning_count,
        }
    });

    println!("{}", serde_json::to_string_pretty(&output)?);

    // Exit with error code if any errors
    if error_count > 0 {
        std::process::exit(1);
    }

    Ok(())
}

/// Lint a project for common issues and style problems
fn lint_project(
    path: &PathBuf,
    quiet: bool,
    show_fixes: bool,
    strict_flag: bool,
    warn_untyped_flag: bool,
) -> anyhow::Result<()> {
    use serde_json::{json, Value as JsonValue};

    // Resolve lint mode: CLI flag > NTNT_LINT_MODE env > NTNT_STRICT env > project config > default
    let lint_mode = if strict_flag {
        ntnt::config::LintMode::Strict
    } else if warn_untyped_flag {
        ntnt::config::LintMode::Warn
    } else if std::env::var("NTNT_LINT_MODE").is_ok() {
        // NTNT_LINT_MODE was explicitly set — respect it, even if "default"
        ntnt::config::get_lint_mode()
    } else {
        // No NTNT_LINT_MODE set — fall back to legacy NTNT_STRICT and project config
        if ntnt::typechecker::is_strict_mode() || read_project_config_strict(path) {
            ntnt::config::LintMode::Strict
        } else {
            ntnt::config::LintMode::Default
        }
    };
    let files = collect_tnt_files(path)?;

    let mut results: Vec<JsonValue> = Vec::new();
    let mut error_count = 0;
    let mut warning_count = 0;
    let mut suggestion_count = 0;

    for file_path in &files {
        let relative_path = file_path
            .strip_prefix(path.parent().unwrap_or(path))
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();

        let source = match fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                results.push(json!({
                    "file": relative_path,
                    "issues": [{"severity": "error", "message": format!("Could not read file: {}", e), "line": null}],
                }));
                error_count += 1;
                continue;
            }
        };

        let lexer = Lexer::new(&source);
        let tokens: Vec<_> = lexer.collect();
        let mut parser = IntentParser::new(tokens);

        match parser.parse() {
            Ok(ast) => {
                // Run comprehensive lint checks
                let mut issues = lint_ast(&ast, &source, &relative_path);

                // Run type checker with lint mode
                let lint_file_path_str = file_path.to_string_lossy();
                let type_diagnostics = ntnt::typechecker::check_program_with_lint_mode(
                    &ast,
                    &source,
                    lint_mode,
                    Some(&lint_file_path_str),
                );
                for diag in type_diagnostics {
                    let severity = match diag.severity {
                        ntnt::typechecker::Severity::Error => "error",
                        ntnt::typechecker::Severity::Warning => "warning",
                    };
                    issues.push(json!({
                        "severity": severity,
                        "rule": "type_check",
                        "message": diag.message,
                        "line": if diag.line > 0 { Some(diag.line) } else { None::<usize> },
                        "hint": diag.hint,
                    }));
                }

                for issue in &issues {
                    let severity = issue["severity"].as_str().unwrap_or("warning");
                    match severity {
                        "error" => error_count += 1,
                        "warning" => warning_count += 1,
                        "suggestion" => suggestion_count += 1,
                        _ => {}
                    }
                }

                if !issues.is_empty() {
                    results.push(json!({
                        "file": relative_path,
                        "issues": issues,
                    }));

                    if !quiet {
                        let warn_str = if warning_count > 0 {
                            format!("{} warnings", warning_count)
                        } else {
                            String::new()
                        };
                        let sug_str = if suggestion_count > 0 {
                            format!("{} suggestions", suggestion_count)
                        } else {
                            String::new()
                        };
                        let parts: Vec<&str> = [warn_str.as_str(), sug_str.as_str()]
                            .iter()
                            .filter(|s| !s.is_empty())
                            .copied()
                            .collect();
                        eprintln!("{} {} ({})", "⚠".yellow(), relative_path, parts.join(", "));
                    }
                } else {
                    eprintln!("{} {}", "✓".green(), relative_path);
                }
            }
            Err(e) => {
                let error_msg = e.to_string();
                let line = extract_line_from_error(&error_msg);

                results.push(json!({
                    "file": relative_path,
                    "issues": [{
                        "severity": "error",
                        "rule": "parse_error",
                        "message": error_msg,
                        "line": line
                    }],
                }));
                error_count += 1;

                eprintln!("{} {}", "✗".red(), relative_path);
            }
        }
    }

    // Summary
    eprintln!();
    if error_count == 0 && warning_count == 0 && suggestion_count == 0 {
        eprintln!("{}", "No issues found!".green().bold());
    } else {
        if error_count > 0 {
            eprintln!("{}: {}", "Errors".red().bold(), error_count);
        }
        if warning_count > 0 && !quiet {
            eprintln!("{}: {}", "Warnings".yellow().bold(), warning_count);
        }
        if suggestion_count > 0 && !quiet {
            eprintln!("{}: {}", "Suggestions".cyan().bold(), suggestion_count);
        }
    }

    // Output JSON
    let mut output = json!({
        "files": results,
        "summary": {
            "total_files": files.len(),
            "errors": error_count,
            "warnings": warning_count,
            "suggestions": suggestion_count,
        }
    });

    // Add syntax quick reference for agents if there are issues
    if show_fixes && (error_count > 0 || warning_count > 0) {
        output["syntax_hints"] = json!({
            "map_literals": "Use `map { \"key\": value }` not `{ \"key\": value }`",
            "route_patterns": "Route builtins auto-detect {param} patterns - raw strings are optional",
            "string_interpolation": "Use `\"{variable}\"` not `\"${variable}\"`",
            "ranges": "Use `0..10` (exclusive) or `0..=10` (inclusive), not `range()`",
            "imports": "Use `import { x } from \"std/module\"` with `/` path separator",
        });
    }

    println!("{}", serde_json::to_string_pretty(&output)?);

    // Exit with error code if any errors
    if error_count > 0 {
        std::process::exit(1);
    }

    Ok(())
}

/// Comprehensive lint checks for NTNT code
fn lint_ast(ast: &ntnt::ast::Program, source: &str, _filename: &str) -> Vec<serde_json::Value> {
    use ntnt::ast::{Expression, Statement};
    use serde_json::json;

    let mut issues = Vec::new();
    let source_lines: Vec<&str> = source.lines().collect();

    // Track context
    let mut http_route_functions = std::collections::HashSet::new();
    http_route_functions.insert("get");
    http_route_functions.insert("post");
    http_route_functions.insert("put");
    http_route_functions.insert("delete");
    http_route_functions.insert("patch");
    http_route_functions.insert("options");
    http_route_functions.insert("head");

    fn find_line_number(source_lines: &[&str], pattern: &str) -> Option<usize> {
        for (i, line) in source_lines.iter().enumerate() {
            if line.contains(pattern) {
                return Some(i + 1);
            }
        }
        None
    }

    fn check_expr_for_issues(
        expr: &Expression,
        source_lines: &[&str],
        issues: &mut Vec<serde_json::Value>,
        http_route_functions: &std::collections::HashSet<&str>,
    ) {
        match expr {
            // Check for route patterns without raw strings
            Expression::Call {
                function,
                arguments,
            } => {
                if let Expression::Identifier(name) = function.as_ref() {
                    if http_route_functions.contains(name.as_str()) {
                        // First argument should be a route pattern
                        if let Some(first_arg) = arguments.first() {
                            match first_arg {
                                Expression::String(_s) => {
                                    // Route builtins auto-detect {param} patterns at runtime,
                                    // so raw strings are optional. No warning needed.
                                }
                                Expression::InterpolatedString(_parts) => {
                                    // Route builtins auto-detect {param} patterns at runtime.
                                    // InterpolatedString in route calls is handled by
                                    // eval_route_pattern() which preserves {name} as literal
                                    // route parameters. No warning needed.
                                }
                                _ => {}
                            }
                        }
                    }
                }

                // Recurse into function and arguments
                check_expr_for_issues(function, source_lines, issues, http_route_functions);
                for arg in arguments {
                    check_expr_for_issues(arg, source_lines, issues, http_route_functions);
                }
            }

            // Recurse into other expression types
            Expression::Binary { left, right, .. } => {
                check_expr_for_issues(left, source_lines, issues, http_route_functions);
                check_expr_for_issues(right, source_lines, issues, http_route_functions);
            }
            Expression::Unary { operand, .. } => {
                check_expr_for_issues(operand, source_lines, issues, http_route_functions);
            }
            Expression::Array(items) => {
                for item in items {
                    check_expr_for_issues(item, source_lines, issues, http_route_functions);
                }
            }
            Expression::MapLiteral(pairs) => {
                for (k, v) in pairs {
                    check_expr_for_issues(k, source_lines, issues, http_route_functions);
                    check_expr_for_issues(v, source_lines, issues, http_route_functions);
                }
            }
            Expression::Lambda { body, .. } => {
                for stmt in &body.statements {
                    check_stmt_for_issues(stmt, source_lines, issues, http_route_functions);
                }
            }
            Expression::Block(block) => {
                for stmt in &block.statements {
                    check_stmt_for_issues(stmt, source_lines, issues, http_route_functions);
                }
            }
            Expression::IfExpr {
                condition,
                then_branch,
                else_branch,
            } => {
                check_expr_for_issues(condition, source_lines, issues, http_route_functions);
                check_expr_for_issues(then_branch, source_lines, issues, http_route_functions);
                check_expr_for_issues(else_branch, source_lines, issues, http_route_functions);
            }
            Expression::Match { scrutinee, arms } => {
                check_expr_for_issues(scrutinee, source_lines, issues, http_route_functions);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        check_expr_for_issues(guard, source_lines, issues, http_route_functions);
                    }
                    check_expr_for_issues(&arm.body, source_lines, issues, http_route_functions);
                }
            }
            Expression::MethodCall {
                object, arguments, ..
            } => {
                check_expr_for_issues(object, source_lines, issues, http_route_functions);
                for arg in arguments {
                    check_expr_for_issues(arg, source_lines, issues, http_route_functions);
                }
            }
            Expression::FieldAccess { object, .. } => {
                check_expr_for_issues(object, source_lines, issues, http_route_functions);
            }
            Expression::Index { object, index } => {
                check_expr_for_issues(object, source_lines, issues, http_route_functions);
                check_expr_for_issues(index, source_lines, issues, http_route_functions);
            }
            Expression::Range { start, end, .. } => {
                check_expr_for_issues(start, source_lines, issues, http_route_functions);
                check_expr_for_issues(end, source_lines, issues, http_route_functions);
            }
            Expression::Assign { target, value } => {
                check_expr_for_issues(target, source_lines, issues, http_route_functions);
                check_expr_for_issues(value, source_lines, issues, http_route_functions);
            }
            Expression::Await(inner) | Expression::Try(inner) => {
                check_expr_for_issues(inner, source_lines, issues, http_route_functions);
            }
            Expression::StructLiteral { fields, .. } => {
                for (_, v) in fields {
                    check_expr_for_issues(v, source_lines, issues, http_route_functions);
                }
            }
            Expression::EnumVariant { arguments, .. } => {
                for arg in arguments {
                    check_expr_for_issues(arg, source_lines, issues, http_route_functions);
                }
            }
            _ => {}
        }
    }

    fn check_stmt_for_issues(
        stmt: &Statement,
        source_lines: &[&str],
        issues: &mut Vec<serde_json::Value>,
        http_route_functions: &std::collections::HashSet<&str>,
    ) {
        match unwrap_located(stmt) {
            Statement::Expression(expr) => {
                check_expr_for_issues(expr, source_lines, issues, http_route_functions);
            }
            Statement::Let { value, .. } => {
                if let Some(expr) = value {
                    check_expr_for_issues(expr, source_lines, issues, http_route_functions);
                }
            }
            Statement::Function {
                body,
                contract,
                name,
                ..
            } => {
                // Check for functions without contracts (suggestion only for exported ones)
                if contract.is_none() {
                    let line = find_line_number(source_lines, &format!("fn {}", name));
                    issues.push(json!({
                        "severity": "suggestion",
                        "rule": "function_no_contract",
                        "message": format!("Function '{}' has no contracts. Consider adding requires/ensures for better documentation and safety.", name),
                        "line": line,
                    }));
                }

                for s in &body.statements {
                    check_stmt_for_issues(s, source_lines, issues, http_route_functions);
                }
            }
            Statement::If {
                condition,
                then_branch,
                else_branch,
            } => {
                check_expr_for_issues(condition, source_lines, issues, http_route_functions);
                for s in &then_branch.statements {
                    check_stmt_for_issues(s, source_lines, issues, http_route_functions);
                }
                if let Some(eb) = else_branch {
                    for s in &eb.statements {
                        check_stmt_for_issues(s, source_lines, issues, http_route_functions);
                    }
                }
            }
            Statement::While { condition, body } => {
                check_expr_for_issues(condition, source_lines, issues, http_route_functions);
                for s in &body.statements {
                    check_stmt_for_issues(s, source_lines, issues, http_route_functions);
                }
            }
            Statement::ForIn { iterable, body, .. } => {
                check_expr_for_issues(iterable, source_lines, issues, http_route_functions);
                for s in &body.statements {
                    check_stmt_for_issues(s, source_lines, issues, http_route_functions);
                }
            }
            Statement::Loop { body } => {
                for s in &body.statements {
                    check_stmt_for_issues(s, source_lines, issues, http_route_functions);
                }
            }
            Statement::Return(Some(expr)) => {
                check_expr_for_issues(expr, source_lines, issues, http_route_functions);
            }
            Statement::Defer(expr) => {
                check_expr_for_issues(expr, source_lines, issues, http_route_functions);
            }
            Statement::Impl {
                methods,
                invariants,
                ..
            } => {
                for method in methods {
                    check_stmt_for_issues(method, source_lines, issues, http_route_functions);
                }
                for inv in invariants {
                    check_expr_for_issues(inv, source_lines, issues, http_route_functions);
                }
            }
            Statement::Module { body, .. } => {
                for s in body {
                    check_stmt_for_issues(s, source_lines, issues, http_route_functions);
                }
            }
            Statement::Export { statement, .. } => {
                if let Some(s) = statement {
                    check_stmt_for_issues(s, source_lines, issues, http_route_functions);
                }
            }
            _ => {}
        }
    }

    // Run checks on all statements
    for stmt in &ast.statements {
        check_stmt_for_issues(stmt, &source_lines, &mut issues, &http_route_functions);
    }

    // Check for import name collisions (same name imported from different modules)
    {
        // Find line numbers for each import by searching for the module source string
        fn find_import_line(source_lines: &[&str], module: &str, after_line: usize) -> usize {
            for (i, line) in source_lines.iter().enumerate() {
                if i + 1 <= after_line {
                    continue;
                }
                if line.contains("import") && line.contains(module) {
                    return i + 1;
                }
            }
            0
        }

        let mut imported_names: std::collections::HashMap<String, (String, usize)> =
            std::collections::HashMap::new();
        let mut last_import_line: usize = 0;

        for stmt in &ast.statements {
            if let Statement::Import { items, source, .. } = unwrap_located(stmt) {
                let current_line = find_import_line(&source_lines, source, last_import_line);
                if current_line > 0 {
                    last_import_line = current_line;
                }

                for item in items {
                    let local_name = item.alias.as_ref().unwrap_or(&item.name);

                    if let Some((prev_source, prev_line)) = imported_names.get(local_name) {
                        if prev_source != source {
                            issues.push(json!({
                                "severity": "warning",
                                "rule": "import_collision",
                                "message": format!(
                                    "'{}' imported from both \"{}\" (line {}) and \"{}\" (line {}). The second import shadows the first. Consider using an alias:\n    import {{ {} as {}_alias }} from \"{}\"",
                                    local_name, prev_source, prev_line, source, current_line,
                                    local_name, local_name, source
                                ),
                                "line": current_line,
                            }));
                        }
                    }
                    imported_names.insert(local_name.clone(), (source.clone(), current_line));
                }
            }
        }
    }

    // Also run the existing unused import analysis
    let ast_warnings = analyze_ast_warnings(ast, source);
    for w in ast_warnings {
        issues.push(json!({
            "severity": "warning",
            "rule": w["type"].as_str().unwrap_or("unknown"),
            "message": w["message"],
            "line": null,
        }));
    }

    // Check source-level patterns that might indicate issues
    // These are heuristic checks on the raw source
    for (line_num, line) in source_lines.iter().enumerate() {
        // Check for JavaScript-style template strings
        if line.contains("${") && line.contains("`") {
            issues.push(json!({
                "severity": "warning",
                "rule": "javascript_template_string",
                "message": "Possible JavaScript-style template string detected. NTNT uses \"{variable}\" for interpolation, not `${variable}`.",
                "line": line_num + 1,
                "fix": {
                    "description": "Replace `${var}` with \"{var}\" and remove backticks"
                }
            }));
        }

        // Check for Python-style range() calls
        if line.contains("range(") && (line.contains("for ") || line.contains("for\t")) {
            issues.push(json!({
                "severity": "warning", 
                "rule": "python_style_range",
                "message": "Possible Python-style range() detected. NTNT uses `0..10` for exclusive ranges or `0..=10` for inclusive.",
                "line": line_num + 1,
                "fix": {
                    "description": "Replace range(n) with 0..n or range(a, b) with a..b"
                }
            }));
        }

        // Check for Rust/Python-style imports (heuristic)
        let trimmed = line.trim();
        if trimmed.starts_with("from ") && trimmed.contains(" import ") {
            issues.push(json!({
                "severity": "error",
                "rule": "python_import_syntax",
                "message": "Python-style import detected. NTNT uses `import {{ x }} from \"module\"`.",
                "line": line_num + 1,
                "fix": {
                    "description": "Rewrite as: import { x } from \"std/module\""
                }
            }));
        }

        if trimmed.starts_with("use ") && trimmed.contains("::") {
            issues.push(json!({
                "severity": "error",
                "rule": "rust_import_syntax", 
                "message": "Rust-style import detected. NTNT uses `import {{ x }} from \"module\"`.",
                "line": line_num + 1,
                "fix": {
                    "description": "Rewrite as: import { x } from \"std/module\""
                }
            }));
        }

        // NOTE: NTNT DOES support escape sequences in regular strings!
        // The lexer handles: \n \t \r \\ \" \' \{ \}
        // Previous versions had incorrect warnings here - those have been removed.

        // Check for unnecessary semicolons (not inside triple-quoted template strings)
        // Only flag lines that end with a semicolon and are not inside template strings or embedded JS/CSS
        if trimmed.ends_with(';') && !trimmed.starts_with("//") {
            // Heuristic: skip lines that look like embedded CSS/JS (common inside template strings)
            let looks_like_css_js = trimmed.contains('{')
                || trimmed.contains('}')
                || trimmed.starts_with("const ")
                || trimmed.starts_with("var ")
                || trimmed.starts_with("let ") && trimmed.contains("= ") && trimmed.contains(".")
                || trimmed.starts_with("return ") && trimmed.contains(".")
                || trimmed.contains("font-family")
                || trimmed.contains("margin")
                || trimmed.contains("padding")
                || trimmed.contains("color:")
                || trimmed.contains("text-align")
                || trimmed.contains("max-width")
                || trimmed.contains("border")
                || trimmed.contains("background")
                || trimmed.contains("box-shadow")
                || trimmed.contains("max-height");

            if !looks_like_css_js {
                issues.push(json!({
                    "severity": "warning",
                    "rule": "unnecessary_semicolon",
                    "message": "Semicolons are not needed in NTNT. Remove the trailing semicolon.",
                    "line": line_num + 1,
                    "fix": {
                        "description": "Remove the semicolon at the end of the line"
                    }
                }));
            }
        }
    }

    issues
}

/// Run intent-driven development commands
fn run_intent_command(cmd: IntentCommands) -> anyhow::Result<()> {
    match cmd {
        IntentCommands::Check {
            file,
            intent_file,
            port,
            verbose,
            json,
        } => run_intent_check_command(&file, intent_file.as_ref(), port, verbose as usize, json),
        IntentCommands::Coverage { file, intent_file } => {
            run_intent_coverage_command(&file, intent_file.as_ref())
        }
        IntentCommands::Init {
            intent_file,
            output,
        } => run_intent_init_command(&intent_file, output.as_ref()),
        IntentCommands::Studio {
            intent_file,
            port,
            app_port,
            no_open,
        } => run_intent_studio_command(&intent_file, port, app_port, no_open),
    }
}

/// Run the intent check command
///
/// Verbosity levels:
/// - 0: Summary only (feature pass/fail counts)
/// - 1: Show scenarios (current default behavior)
/// - 2+: Show assertions and term resolution
fn run_intent_check_command(
    input_path: &PathBuf,
    explicit_intent_path: Option<&PathBuf>,
    port: u16,
    verbosity: usize,
    json_output: bool,
) -> anyhow::Result<()> {
    // Suppress banner output in JSON mode
    if !json_output {
        println!("{}", "=== NTNT Intent Check ===".cyan().bold());
        println!();
    }

    // Verify file exists
    if !input_path.exists() {
        anyhow::bail!("File not found: {}", input_path.display());
    }

    // Resolve both .intent and .tnt paths from either input
    let (intent_path_opt, tnt_path_opt) = if let Some(explicit) = explicit_intent_path {
        // User explicitly provided intent file
        (Some(explicit.clone()), Some(input_path.clone()))
    } else {
        intent::resolve_intent_tnt_pair(input_path)
    };

    // We need both files for check
    let intent_file_path = match intent_path_opt {
        Some(p) => p,
        None => {
            let stem = input_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            anyhow::bail!(
                "No intent file found. Create {}.intent or specify with --intent",
                stem
            );
        }
    };

    let ntnt_path = match tnt_path_opt {
        Some(p) => p,
        None => {
            let stem = input_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            anyhow::bail!(
                "No .tnt file found. Create {}.tnt to run tests against",
                stem
            );
        }
    };

    if !json_output {
        println!("Source: {}", ntnt_path.display().to_string().green());
        println!("Intent: {}", intent_file_path.display().to_string().green());
        println!();
    }

    // Parse intent file (new IAL format)
    let intent_file = match intent::IntentFile::parse(&intent_file_path) {
        Ok(intent) => intent,
        Err(e) => {
            anyhow::bail!("Failed to parse intent file: {}", e);
        }
    };

    // Collect all source files for annotation checking
    let project_dir = ntnt_path.parent().unwrap_or(std::path::Path::new("."));
    let source_files: Vec<(String, String)> = collect_tnt_files(&project_dir.to_path_buf())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| {
            let content = std::fs::read_to_string(&p).ok()?;
            Some((p.to_string_lossy().to_string(), content))
        })
        .collect();

    // Run tests against the app server (same as Intent Studio)
    if !json_output {
        println!("Starting server on port {}...", port);
    }

    // Get the current executable path to run ntnt
    let current_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ntnt"));

    // Start the NTNT app server as a subprocess
    use std::process::Command;
    let mut app_process = Command::new(&current_exe)
        .arg("run")
        .arg(&ntnt_path)
        .env("NTNT_LISTEN_PORT", port.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to start app server: {}", e))?;

    // Wait for the server to be ready (TCP connect poll, up to 30 seconds)
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(30);
    let mut server_ready = false;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    while start.elapsed() < timeout {
        std::thread::sleep(std::time::Duration::from_millis(500));
        // Check if subprocess died
        if let Some(status) = app_process.try_wait().ok().flatten() {
            let _ = app_process.kill();
            anyhow::bail!(
                "Server process exited with status {} before becoming ready",
                status
            );
        }
        // Try TCP connect with a short timeout
        if std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(500))
            .is_ok()
        {
            server_ready = true;
            break;
        }
    }
    if !server_ready {
        let _ = app_process.kill();
        anyhow::bail!("Server failed to start within 30 seconds on port {}", port);
    }

    // Run tests using the new IAL engine
    let results = intent::run_tests_against_server(&intent_file, port, &source_files);

    // JSON output mode: print JSON and exit
    if json_output {
        // Kill app server before output
        let _ = app_process.kill();

        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| anyhow::anyhow!("Failed to serialize results: {}", e))?;
        println!("{}", json);

        // Exit with appropriate code
        if results.failed_assertions > 0 {
            std::process::exit(1);
        }
        return Ok(());
    }

    // Print results based on verbosity level
    // 0: Summary only (feature names + totals)
    // 1: Show scenarios
    // 2+: Show assertions and term resolution
    println!();
    println!("{}", "=== Test Results ===".cyan().bold());
    println!();

    // Calculate stats
    let mut scenarios_passed = 0;
    let mut scenarios_failed = 0;
    let mut scenarios_skipped = 0;
    let mut features_failed = 0;

    // Count scenarios for all features (needed for summary)
    for feature in &results.features {
        for scenario in &feature.scenarios {
            match scenario.status.as_str() {
                "pass" => scenarios_passed += 1,
                "fail" => scenarios_failed += 1,
                "skip" => scenarios_skipped += 1,
                _ => {}
            }
        }
        if !feature.passed {
            features_failed += 1;
        }
    }
    for component in &results.components {
        for scenario in &component.scenarios {
            match scenario.status.as_str() {
                "pass" => scenarios_passed += 1,
                "fail" => scenarios_failed += 1,
                "skip" => scenarios_skipped += 1,
                _ => {}
            }
        }
    }

    // Verbosity 0: Summary only - just feature names with pass/fail counts
    if verbosity == 0 {
        for feature in &results.features {
            let status_icon = if feature.passed {
                "✓".green()
            } else {
                "✗".red()
            };
            let scenario_count = feature.scenarios.len() + feature.tests.len();
            let passed_count = feature
                .scenarios
                .iter()
                .filter(|s| s.status == "pass")
                .count()
                + feature.tests.iter().filter(|t| t.passed).count();

            if feature.passed {
                println!(
                    "{} {}  {} scenarios",
                    status_icon,
                    feature.feature_name.bold(),
                    scenario_count
                );
            } else {
                println!(
                    "{} {}  {}/{} scenarios passed",
                    status_icon,
                    feature.feature_name.bold(),
                    passed_count,
                    scenario_count
                );
            }
        }
        for component in &results.components {
            let status_icon = if component.passed {
                "✓".green()
            } else {
                "✗".red()
            };
            println!(
                "{} Component: {}",
                status_icon,
                component.component_name.bold()
            );
        }
        println!();
    } else {
        // Verbosity 1+: Show scenarios
        for feature in &results.features {
            let status_icon = if feature.passed {
                "✓".green()
            } else {
                "✗".red()
            };
            println!("{} Feature: {}", status_icon, feature.feature_name.bold());

            for scenario in &feature.scenarios {
                let icon = match scenario.status.as_str() {
                    "pass" => "  ✓".green(),
                    "fail" => "  ✗".red(),
                    "skip" => "  ⏭️ ".yellow(),
                    _ => "  ⧗".yellow(),
                };
                println!("{} {}", icon, scenario.name);

                // Verbosity 2+: Show assertions and term resolution
                if verbosity >= 2 {
                    if let Some(ref given) = scenario.given_clause {
                        println!("      Given {}", given.dimmed());
                    }
                    println!("      When {}", scenario.when_clause.dimmed());
                    for outcome in &scenario.outcomes {
                        println!("      → {}", outcome.dimmed());
                    }
                    if let Some(ref test_result) = scenario.test_result {
                        for assertion in &test_result.assertions {
                            let a_icon = if assertion.passed {
                                "✓".green()
                            } else {
                                "✗".red()
                            };
                            println!("        {} {}", a_icon, assertion.assertion_text.dimmed());
                            // Show failure message if present
                            if !assertion.passed {
                                if let Some(ref msg) = assertion.message {
                                    println!("          {}", msg.red());
                                }
                            }
                        }
                    }
                }
                // Always show details for failed scenarios (even at verbosity 1)
                else if scenario.status == "fail" {
                    if let Some(ref test_result) = scenario.test_result {
                        for assertion in &test_result.assertions {
                            if !assertion.passed {
                                println!("      ✗ {}", assertion.assertion_text.red());
                                if let Some(ref msg) = assertion.message {
                                    println!("        {}", msg.red());
                                }
                            }
                        }
                    }
                }
            }

            for test in &feature.tests {
                let icon = if test.passed {
                    "  ✓".green()
                } else {
                    "  ✗".red()
                };
                println!("{} {} {}", icon, test.method, test.path);

                // Verbosity 2+: Show all assertions
                if verbosity >= 2 {
                    for assertion in &test.assertions {
                        let a_icon = if assertion.passed {
                            "✓".green()
                        } else {
                            "✗".red()
                        };
                        println!("      {} {}", a_icon, assertion.assertion_text.dimmed());
                        if !assertion.passed {
                            if let Some(ref msg) = assertion.message {
                                println!("        {}", msg.red());
                            }
                        }
                    }
                }
                // Always show details for failed tests (even at verbosity 1)
                else if !test.passed {
                    for assertion in &test.assertions {
                        if !assertion.passed {
                            println!("      ✗ {}", assertion.assertion_text.red());
                            if let Some(ref msg) = assertion.message {
                                println!("        {}", msg.red());
                            }
                        }
                    }
                }
            }

            println!();
        }

        for component in &results.components {
            let status_icon = if component.passed {
                "✓".green()
            } else {
                "✗".red()
            };
            println!(
                "{} Component: {}",
                status_icon,
                component.component_name.bold()
            );

            for scenario in &component.scenarios {
                let icon = match scenario.status.as_str() {
                    "pass" => "  ✓".green(),
                    "fail" => "  ✗".red(),
                    "skip" => "  ⏭️ ".yellow(),
                    _ => "  ⧗".yellow(),
                };
                println!("{} {}", icon, scenario.name);
            }
            println!();
        }
    }

    println!("{}", "=== Summary ===".cyan().bold());
    println!(
        "Features: {} total, {} passed, {} failed",
        results.total_features,
        results.total_features - features_failed,
        features_failed
    );
    println!(
        "Scenarios: {} passed, {} failed, {} skipped",
        scenarios_passed, scenarios_failed, scenarios_skipped
    );
    println!(
        "Assertions: {} passed, {} failed",
        results.passed_assertions, results.failed_assertions
    );
    println!();

    // Cleanup: kill the app server
    let _ = app_process.kill();

    if scenarios_failed > 0 || features_failed > 0 {
        anyhow::bail!("Some tests failed");
    }

    Ok(())
}

/// Run the intent coverage command
fn run_intent_coverage_command(
    input_path: &PathBuf,
    explicit_intent_path: Option<&PathBuf>,
) -> anyhow::Result<()> {
    // Verify file exists
    if !input_path.exists() {
        anyhow::bail!("File not found: {}", input_path.display());
    }

    // Resolve both .intent and .tnt paths from either input
    let (intent_path_opt, tnt_path_opt) = if let Some(explicit) = explicit_intent_path {
        // User explicitly provided intent file
        (Some(explicit.clone()), Some(input_path.clone()))
    } else {
        intent::resolve_intent_tnt_pair(input_path)
    };

    // We need both files for coverage
    let intent_file_path = match intent_path_opt {
        Some(p) => p,
        None => {
            let stem = input_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            anyhow::bail!(
                "No intent file found. Create {}.intent or specify with --intent",
                stem
            );
        }
    };

    let ntnt_path = match tnt_path_opt {
        Some(p) => p,
        None => {
            let stem = input_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            anyhow::bail!("No .tnt file found. Create {}.tnt to check coverage", stem);
        }
    };

    // Parse intent file
    let intent_file = intent::IntentFile::parse(&intent_file_path)
        .map_err(|e| anyhow::anyhow!("Failed to parse intent file: {}", e))?;

    // Read source file(s)
    let mut source_files = Vec::new();

    // Add main .tnt file
    let source_content = fs::read_to_string(&ntnt_path)?;
    source_files.push((ntnt_path.to_string_lossy().to_string(), source_content));

    // Also scan routes/ directory for file-based routing
    let routes_dir = ntnt_path
        .parent()
        .map(|p| p.join("routes"))
        .unwrap_or_else(|| PathBuf::from("routes"));

    if routes_dir.exists() && routes_dir.is_dir() {
        // Recursively collect all .tnt files from routes directory
        fn collect_route_files(dir: &PathBuf, files: &mut Vec<(String, String)>) {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        collect_route_files(&path, files);
                    } else if path.extension().map(|e| e == "tnt").unwrap_or(false) {
                        if let Ok(content) = fs::read_to_string(&path) {
                            files.push((path.to_string_lossy().to_string(), content));
                        }
                    }
                }
            }
        }

        collect_route_files(&routes_dir, &mut source_files);
    }

    // Generate and print coverage report
    let report = intent::generate_coverage_report(&intent_file, &source_files);
    intent::print_coverage_report(&report);

    // Exit with error if coverage is 0%
    if report.covered_features == 0 && report.total_features > 0 {
        std::process::exit(1);
    }

    Ok(())
}

/// Run the intent init command
fn run_intent_init_command(
    input_path: &PathBuf,
    output_path: Option<&PathBuf>,
) -> anyhow::Result<()> {
    // Verify file exists
    if !input_path.exists() {
        anyhow::bail!("File not found: {}", input_path.display());
    }

    // Resolve to find intent file (allows passing either .tnt or .intent)
    let (intent_path_opt, _tnt_path_opt) = intent::resolve_intent_tnt_pair(input_path);

    let intent_path = match intent_path_opt {
        Some(p) => p,
        None => {
            let stem = input_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            anyhow::bail!("No intent file found. Create {}.intent first", stem);
        }
    };

    // Parse intent file
    let intent_file = intent::IntentFile::parse(&intent_path)
        .map_err(|e| anyhow::anyhow!("Failed to parse intent file: {}", e))?;

    // Generate scaffolding
    let scaffolding = intent::generate_scaffolding(&intent_file);

    // Output
    if let Some(output) = output_path {
        fs::write(output, &scaffolding)?;
        println!(
            "{}",
            format!("Generated {} from intent file", output.display()).green()
        );
        println!();
        println!("Next steps:");
        println!("  1. Implement the TODO functions in {}", output.display());
        println!(
            "  2. Run {} to verify",
            format!("ntnt intent check {}", output.display()).cyan()
        );
    } else {
        // Print to stdout
        println!("{}", scaffolding);
    }

    Ok(())
}

/// Run the intent studio command - starts a visual preview server AND the app server
fn run_intent_studio_command(
    input_path: &PathBuf,
    port: u16,
    app_port: u16,
    no_open: bool,
) -> anyhow::Result<()> {
    use std::process::{Child, Command};
    use std::sync::Arc;

    // Verify the file exists
    if !input_path.exists() {
        anyhow::bail!("File not found: {}", input_path.display());
    }

    // Resolve both .intent and .tnt paths from either input
    let (intent_path_opt, tnt_path_opt) = intent::resolve_intent_tnt_pair(input_path);

    // We need an .intent file to show features/tests in Studio
    let intent_path = match intent_path_opt {
        Some(p) => p,
        None => {
            let stem = input_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            eprintln!();
            eprintln!("{}", "  ⚠️  No .intent file found".yellow().bold());
            eprintln!();
            eprintln!("  Intent Studio requires a .intent file to display features and run tests.");
            eprintln!(
                "  Expected: {}.intent",
                input_path
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join(&stem)
                    .display()
            );
            eprintln!();
            eprintln!("  {} Create one with:", "💡".yellow());
            eprintln!("     ntnt intent init {}.intent", stem);
            eprintln!();
            anyhow::bail!("No .intent file found for Intent Studio");
        }
    };

    // .tnt file is optional (Studio can still show intent without running tests)
    let tnt_path = tnt_path_opt;

    let addr = format!("127.0.0.1:{}", port);

    println!();
    println!("{}", "  🎨 Intent Studio".cyan().bold());
    println!();
    println!("  {} {}", "File:".dimmed(), intent_path.display());
    println!("  {} http://{}", "URL:".dimmed(), addr);
    println!("  {} http://127.0.0.1:{}", "App:".dimmed(), app_port);
    println!();

    // Start the app server if .tnt file exists
    let mut app_process: Option<Child> = None;

    if let Some(ref tnt_file) = tnt_path {
        // Get the current executable path to run ntnt
        let current_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ntnt"));

        // Set up environment to override the listen port
        println!(
            "  {} Starting app from {}",
            "🚀".green(),
            tnt_file.display()
        );

        match Command::new(&current_exe)
            .arg("run")
            .arg(tnt_file)
            .env("NTNT_LISTEN_PORT", app_port.to_string())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .spawn()
        {
            Ok(child) => {
                app_process = Some(child);
                println!(
                    "  {} App server starting on port {}",
                    "✅".green(),
                    app_port
                );

                // Give it a moment to start
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(e) => {
                println!("  {} Failed to start app: {}", "⚠️".yellow(), e);
                println!(
                    "  {} You can start it manually: ntnt run {}",
                    "💡".dimmed(),
                    tnt_file.display()
                );
            }
        }
    } else {
        let expected_tnt = intent_path.with_extension("tnt");
        println!(
            "  {} No .tnt file found at {}",
            "⚠️".yellow(),
            expected_tnt.display()
        );
        println!(
            "  {} Start your app manually: ntnt run <your-app>.tnt",
            "💡".dimmed()
        );
    }

    println!();
    println!("  {} Live test execution enabled!", "✅".green());
    println!();
    println!(
        "  {} Watching for changes (auto-refresh every 2s)",
        "👀".dimmed()
    );
    println!("  {} Press Ctrl+C to stop", "📝".dimmed());
    println!();

    // Open browser before starting server
    if !no_open {
        let url = format!("http://{}", addr);
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open").arg(&url).spawn();
        }
        #[cfg(target_os = "linux")]
        {
            let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
        }
        #[cfg(target_os = "windows")]
        {
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", &url])
                .spawn();
        }
    }

    // Create async server state
    let state = Arc::new(intent_studio_server::StudioState::new(
        intent_path.clone(),
        tnt_path.clone(),
        app_port,
    ));

    // Build Tokio runtime for the async server
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create Tokio runtime: {}", e))?;

    // Run the async server with graceful shutdown
    let server_result = runtime.block_on(async {
        use tokio::signal;

        let app = intent_studio_server::create_router(state);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to bind to {}: {}", addr, e))?;

        // Serve with graceful shutdown on Ctrl+C
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                signal::ctrl_c()
                    .await
                    .expect("Failed to install Ctrl+C handler");
                println!();
            })
            .await
            .map_err(|e| anyhow::anyhow!("Server error: {}", e))
    });

    // Clean up: kill the app process if we started it
    if let Some(mut child) = app_process {
        println!("  {} Stopping app server...", "🛑".red());
        let _ = child.kill();
        let _ = child.wait();
    }

    println!("  {} Intent Studio stopped", "👋".dimmed());

    server_result
}
/// Collect all .tnt files from a path (file or directory)
/// Read project config from ntnt.toml (searches path's directory and ancestors)
fn read_project_config_strict(path: &PathBuf) -> bool {
    // Start from the path (or its parent if it's a file) and walk up
    let start_dir = if path.is_file() {
        path.parent().unwrap_or(path).to_path_buf()
    } else {
        path.to_path_buf()
    };

    let mut dir = Some(start_dir.as_path());
    while let Some(d) = dir {
        let config_path = d.join("ntnt.toml");
        if config_path.exists() {
            if let Ok(content) = fs::read_to_string(&config_path) {
                if let Ok(config) = content.parse::<toml::Value>() {
                    return config
                        .get("lint")
                        .and_then(|l| l.get("strict"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                }
            }
            return false;
        }
        dir = d.parent();
    }
    false
}

fn collect_tnt_files(path: &PathBuf) -> anyhow::Result<Vec<PathBuf>> {
    use std::ffi::OsStr;

    let mut files = Vec::new();

    if path.is_file() {
        if path.extension() == Some(OsStr::new("tnt")) {
            files.push(path.clone());
        }
    } else if path.is_dir() {
        fn collect_recursive(dir: &PathBuf, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() && path.extension() == Some(std::ffi::OsStr::new("tnt")) {
                    files.push(path);
                } else if path.is_dir() {
                    // Skip common non-source directories
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if matches!(
                            name,
                            "node_modules"
                                | ".git"
                                | "target"
                                | "dist"
                                | "build"
                                | "__pycache__"
                                | ".venv"
                                | "venv"
                                | "vendor"
                                | "repos"
                                | "static-archive"
                                | ".next"
                                | "coverage"
                                | ".cache"
                        ) {
                            continue;
                        }
                    }
                    collect_recursive(&path, files)?;
                }
            }
            Ok(())
        }
        collect_recursive(path, &mut files)?;
    }

    files.sort();
    Ok(files)
}

/// Convert a parameter to JSON
fn param_to_json(param: &ntnt::ast::Parameter) -> serde_json::Value {
    use serde_json::json;
    json!({
        "name": param.name,
        "type": param.type_annotation.as_ref().map(|t| type_to_string(t)),
        "has_default": param.default.is_some(),
    })
}

/// Convert a type expression to a readable string
fn type_to_string(t: &ntnt::ast::TypeExpr) -> String {
    use ntnt::ast::TypeExpr;
    match t {
        TypeExpr::Named(name) => name.clone(),
        TypeExpr::Array(inner) => format!("[{}]", type_to_string(inner)),
        TypeExpr::Map {
            key_type,
            value_type,
        } => {
            format!(
                "Map<{}, {}>",
                type_to_string(key_type),
                type_to_string(value_type)
            )
        }
        TypeExpr::Tuple(types) => {
            format!(
                "({})",
                types
                    .iter()
                    .map(type_to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        TypeExpr::Function {
            params,
            return_type,
        } => {
            format!(
                "({}) -> {}",
                params
                    .iter()
                    .map(type_to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                type_to_string(return_type)
            )
        }
        TypeExpr::Generic { name, args } => {
            format!(
                "{}<{}>",
                name,
                args.iter()
                    .map(type_to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        TypeExpr::Optional(inner) => format!("{}?", type_to_string(inner)),
        TypeExpr::Union(types) => types
            .iter()
            .map(type_to_string)
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

/// Convert contract to JSON
fn contract_to_json(contract: &Option<ntnt::ast::Contract>) -> serde_json::Value {
    use serde_json::json;
    match contract {
        Some(c) => json!({
            "requires": c.requires.iter().map(|e| expr_to_string(e)).collect::<Vec<_>>(),
            "ensures": c.ensures.iter().map(|e| expr_to_string(e)).collect::<Vec<_>>(),
        }),
        None => json!(null),
    }
}

/// Convert an expression to a readable string (simplified)
fn expr_to_string(expr: &ntnt::ast::Expression) -> String {
    use ntnt::ast::Expression;
    match expr {
        Expression::Identifier(name) => name.clone(),
        Expression::Integer(n) => n.to_string(),
        Expression::Float(n) => n.to_string(),
        Expression::String(s) => format!("\"{}\"", s),
        Expression::Bool(b) => b.to_string(),
        Expression::Binary {
            left,
            operator,
            right,
        } => {
            format!(
                "{} {:?} {}",
                expr_to_string(left),
                operator,
                expr_to_string(right)
            )
        }
        Expression::FieldAccess { object, field } => {
            format!("{}.{}", expr_to_string(object), field)
        }
        Expression::MethodCall {
            object,
            method,
            arguments,
        } => {
            format!(
                "{}.{}({})",
                expr_to_string(object),
                method,
                arguments
                    .iter()
                    .map(expr_to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        Expression::Call {
            function,
            arguments,
        } => {
            format!(
                "{}({})",
                expr_to_string(function),
                arguments
                    .iter()
                    .map(expr_to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        _ => "<expr>".to_string(),
    }
}

/// Analyze AST for common warnings
fn analyze_ast_warnings(ast: &ntnt::ast::Program, _source: &str) -> Vec<serde_json::Value> {
    use ntnt::ast::Statement;
    use serde_json::json;

    let mut warnings = Vec::new();

    // Track declared but unused imports
    let mut imports: Vec<String> = Vec::new();
    let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    for stmt in &ast.statements {
        match unwrap_located(stmt) {
            Statement::Import { items, .. } => {
                for item in items {
                    imports.push(item.name.clone());
                }
            }
            _ => {
                // Collect used identifiers (simplified - just looks at expressions)
                collect_used_names(stmt, &mut used_names);
            }
        }
    }

    // Check for unused imports
    for import in &imports {
        if !used_names.contains(import) {
            warnings.push(json!({
                "type": "unused_import",
                "message": format!("Unused import: '{}'", import),
            }));
        }
    }

    warnings
}

/// Collect used identifiers from a statement (comprehensive AST traversal)
fn collect_used_names(stmt: &ntnt::ast::Statement, names: &mut std::collections::HashSet<String>) {
    use ntnt::ast::{Expression, Statement, StringPart};
    let stmt = unwrap_located(stmt);

    fn collect_from_expr(expr: &Expression, names: &mut std::collections::HashSet<String>) {
        match expr {
            // Identifiers - the core of what we're tracking
            Expression::Identifier(name) => {
                names.insert(name.clone());
            }

            // Function calls - both the function name and all arguments
            Expression::Call {
                function,
                arguments,
            } => {
                collect_from_expr(function, names);
                for arg in arguments {
                    collect_from_expr(arg, names);
                }
            }

            // Method calls - object and arguments (method name is not a used identifier)
            Expression::MethodCall {
                object, arguments, ..
            } => {
                collect_from_expr(object, names);
                for arg in arguments {
                    collect_from_expr(arg, names);
                }
            }

            // Binary operations
            Expression::Binary { left, right, .. } => {
                collect_from_expr(left, names);
                collect_from_expr(right, names);
            }

            // Unary operations
            Expression::Unary { operand, .. } => {
                collect_from_expr(operand, names);
            }

            // Field access - object contains identifier
            Expression::FieldAccess { object, .. } => {
                collect_from_expr(object, names);
            }

            // Index access
            Expression::Index { object, index } => {
                collect_from_expr(object, names);
                collect_from_expr(index, names);
            }

            // Array literals
            Expression::Array(items) => {
                for item in items {
                    collect_from_expr(item, names);
                }
            }

            // Map literals
            Expression::MapLiteral(pairs) => {
                for (key, value) in pairs {
                    collect_from_expr(key, names);
                    collect_from_expr(value, names);
                }
            }

            // Range expressions
            Expression::Range { start, end, .. } => {
                collect_from_expr(start, names);
                collect_from_expr(end, names);
            }

            // Interpolated strings - expressions inside {}
            Expression::InterpolatedString(parts) => {
                for part in parts {
                    if let StringPart::Expr(expr) = part {
                        collect_from_expr(expr, names);
                    }
                }
            }

            // Template strings - expressions inside {{}}
            Expression::TemplateString(parts) => {
                use ntnt::ast::TemplatePart;
                fn collect_from_template_parts(
                    parts: &[TemplatePart],
                    names: &mut std::collections::HashSet<String>,
                    collect_fn: &dyn Fn(&Expression, &mut std::collections::HashSet<String>),
                ) {
                    for part in parts {
                        match part {
                            TemplatePart::Literal(_) => {}
                            TemplatePart::Expr(expr) | TemplatePart::RawExpr(expr) => {
                                collect_fn(expr, names);
                            }
                            TemplatePart::FilteredExpr { expr, filters }
                            | TemplatePart::RawFilteredExpr { expr, filters } => {
                                collect_fn(expr, names);
                                for filter in filters {
                                    for arg in &filter.args {
                                        collect_fn(arg, names);
                                    }
                                }
                            }
                            TemplatePart::ForLoop {
                                iterable,
                                body,
                                empty_body,
                                ..
                            } => {
                                collect_fn(iterable, names);
                                collect_from_template_parts(body, names, collect_fn);
                                collect_from_template_parts(empty_body, names, collect_fn);
                            }
                            TemplatePart::IfBlock {
                                condition,
                                then_parts,
                                elif_chains,
                                else_parts,
                            } => {
                                collect_fn(condition, names);
                                collect_from_template_parts(then_parts, names, collect_fn);
                                for (elif_cond, elif_body) in elif_chains {
                                    collect_fn(elif_cond, names);
                                    collect_from_template_parts(elif_body, names, collect_fn);
                                }
                                collect_from_template_parts(else_parts, names, collect_fn);
                            }
                            TemplatePart::Partial { data_expr, .. } => {
                                if let Some(expr) = data_expr {
                                    collect_fn(expr, names);
                                }
                            }
                        }
                    }
                }
                collect_from_template_parts(parts, names, &collect_from_expr);
            }

            // Struct literals - the struct name and field values
            Expression::StructLiteral { name, fields } => {
                names.insert(name.clone());
                for (_, value) in fields {
                    collect_from_expr(value, names);
                }
            }

            // Enum variants
            Expression::EnumVariant {
                enum_name,
                arguments,
                ..
            } => {
                names.insert(enum_name.clone());
                for arg in arguments {
                    collect_from_expr(arg, names);
                }
            }

            // Lambda/closures - recurse into body
            Expression::Lambda { body, .. } => {
                for s in &body.statements {
                    collect_used_names(s, names);
                }
            }

            // Block expressions
            Expression::Block(block) => {
                for s in &block.statements {
                    collect_used_names(s, names);
                }
            }

            // If expressions
            Expression::IfExpr {
                condition,
                then_branch,
                else_branch,
            } => {
                collect_from_expr(condition, names);
                collect_from_expr(then_branch, names);
                collect_from_expr(else_branch, names);
            }

            // Match expressions
            Expression::Match { scrutinee, arms } => {
                collect_from_expr(scrutinee, names);
                for arm in arms {
                    // Collect from pattern (might reference types)
                    collect_from_pattern(&arm.pattern, names);
                    if let Some(guard) = &arm.guard {
                        collect_from_expr(guard, names);
                    }
                    collect_from_expr(&arm.body, names);
                }
            }

            // Assignment
            Expression::Assign { target, value } => {
                collect_from_expr(target, names);
                collect_from_expr(value, names);
            }

            // Await
            Expression::Await(inner) => {
                collect_from_expr(inner, names);
            }

            // Try
            Expression::Try(inner) => {
                collect_from_expr(inner, names);
            }

            // TryCatch
            Expression::TryCatch { body } => {
                for s in &body.statements {
                    collect_used_names(s, names);
                }
            }

            // Literals - no identifiers to collect
            Expression::Integer(_)
            | Expression::Float(_)
            | Expression::String(_)
            | Expression::Bool(_)
            | Expression::Unit => {}
        }
    }

    fn collect_from_pattern(
        pattern: &ntnt::ast::Pattern,
        names: &mut std::collections::HashSet<String>,
    ) {
        use ntnt::ast::Pattern;
        match pattern {
            Pattern::Struct { name, fields } => {
                names.insert(name.clone());
                for (_, p) in fields {
                    collect_from_pattern(p, names);
                }
            }
            Pattern::Variant { name, fields, .. } => {
                names.insert(name.clone());
                if let Some(fs) = fields {
                    for p in fs {
                        collect_from_pattern(p, names);
                    }
                }
            }
            Pattern::Tuple(patterns) => {
                for p in patterns {
                    collect_from_pattern(p, names);
                }
            }
            Pattern::Array { elements, .. } => {
                for p in elements {
                    collect_from_pattern(p, names);
                }
            }
            Pattern::Map { fields, .. } => {
                for (_, p) in fields {
                    collect_from_pattern(p, names);
                }
            }
            Pattern::Literal(expr) => {
                collect_from_expr(expr, names);
            }
            Pattern::Variable(_) | Pattern::Wildcard => {}
        }
    }

    match stmt {
        Statement::Expression(expr) => collect_from_expr(expr, names),
        Statement::Let { value, pattern, .. } => {
            if let Some(expr) = value {
                collect_from_expr(expr, names);
            }
            if let Some(pat) = pattern {
                collect_from_pattern(pat, names);
            }
        }
        Statement::Function { body, contract, .. } => {
            // Collect from function body
            for s in &body.statements {
                collect_used_names(s, names);
            }
            // Collect from contracts too
            if let Some(c) = contract {
                for req in &c.requires {
                    collect_from_expr(req, names);
                }
                for ens in &c.ensures {
                    collect_from_expr(ens, names);
                }
            }
        }
        Statement::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_from_expr(condition, names);
            for s in &then_branch.statements {
                collect_used_names(s, names);
            }
            if let Some(eb) = else_branch {
                for s in &eb.statements {
                    collect_used_names(s, names);
                }
            }
        }
        Statement::While { condition, body } => {
            collect_from_expr(condition, names);
            for s in &body.statements {
                collect_used_names(s, names);
            }
        }
        Statement::ForIn { iterable, body, .. } => {
            collect_from_expr(iterable, names);
            for s in &body.statements {
                collect_used_names(s, names);
            }
        }
        Statement::Loop { body } => {
            for s in &body.statements {
                collect_used_names(s, names);
            }
        }
        Statement::Return(Some(expr)) => collect_from_expr(expr, names),
        Statement::Defer(expr) => collect_from_expr(expr, names),
        Statement::Impl {
            methods,
            invariants,
            ..
        } => {
            for method in methods {
                collect_used_names(method, names);
            }
            for inv in invariants {
                collect_from_expr(inv, names);
            }
        }
        Statement::Module { body, .. } => {
            for s in body {
                collect_used_names(s, names);
            }
        }
        Statement::Export { statement, .. } => {
            if let Some(s) = statement {
                collect_used_names(s, names);
            }
        }
        Statement::Intent { target, .. } => {
            collect_used_names(target, names);
        }
        // These don't contain expressions to analyze
        Statement::Return(None)
        | Statement::Break
        | Statement::Continue
        | Statement::Struct { .. }
        | Statement::Enum { .. }
        | Statement::Trait { .. }
        | Statement::TypeAlias { .. }
        | Statement::Use { .. }
        | Statement::Import { .. } => {}
        // Server block contains expressions in port, directives, routes, and groups
        Statement::Server {
            port,
            directives,
            routes,
            groups,
        } => {
            collect_from_expr(port, names);
            for directive in directives {
                match directive {
                    ntnt::ast::ServerDirective::Cors(expr)
                    | ntnt::ast::ServerDirective::Middleware(expr) => {
                        collect_from_expr(expr, names);
                    }
                    ntnt::ast::ServerDirective::Static { .. } => {}
                }
            }
            for route in routes {
                collect_from_expr(&route.handler, names);
            }
            fn collect_from_groups(
                groups: &[ntnt::ast::ServerGroup],
                names: &mut std::collections::HashSet<String>,
            ) {
                for group in groups {
                    for mw in &group.middleware {
                        collect_from_expr(mw, names);
                    }
                    for route in &group.routes {
                        collect_from_expr(&route.handler, names);
                    }
                    collect_from_groups(&group.groups, names);
                }
            }
            collect_from_groups(groups, names);
        }
        Statement::Located { stmt, .. } => collect_used_names(stmt, names),
    }
}

/// Try to extract line number from error message
fn extract_line_from_error(error: &str) -> Option<usize> {
    // Look for patterns like "line 42" or "Line 42:"
    let error_lower = error.to_lowercase();
    if let Some(idx) = error_lower.find("line ") {
        let start = idx + 5;
        let rest = &error[start..];
        let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        num_str.parse().ok()
    } else {
        None
    }
}

// ============================================================================
// Shell Completions
// ============================================================================

/// Generate shell completion scripts
fn generate_completions(shell: Shell) {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    generate(shell, &mut cmd, name, &mut io::stdout());
}

// ============================================================================
// Documentation Command
// ============================================================================

/// Structured doc types (generated by build.rs from // @ntnt source comments)
///
/// KEEP IN SYNC with build.rs DocEntry (Serialize side).
/// If you add/remove/rename a field in build.rs, update this module too.
mod docs {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    pub struct DocEntry {
        pub name: String,
        pub module: Option<String>,
        pub signature: Option<String>,
        pub summary: String,
        pub description: Option<String>,
        #[serde(default)]
        pub params: Vec<ParamDoc>,
        pub returns: Option<String>,
        #[serde(default)]
        pub examples: Vec<ExampleDoc>,
        #[serde(default)]
        pub see_also: Vec<String>,
        pub since: Option<String>,
        #[serde(default)]
        pub tags: Vec<String>,
        #[serde(default)]
        pub errors: Vec<ErrorDoc>,
        #[serde(default)]
        pub gotchas: Vec<String>,
        pub module_description: Option<String>,
        pub source_file: String,
        pub source_line: usize,
    }

    #[derive(Debug, Deserialize)]
    pub struct ExampleDoc {
        pub code: String,
        pub expected: Option<String>,
        pub description: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    pub struct ParamDoc {
        pub name: String,
        pub description: String,
    }

    #[derive(Debug, Deserialize)]
    pub struct ErrorDoc {
        pub error_type: String,
        pub message: String,
        pub fix: Option<String>,
    }
}

// Structured doc data generated by build.rs from // @ntnt source comments
const EMBEDDED_DOC_DATA: &str = include_str!(concat!(env!("OUT_DIR"), "/doc_data.json"));

/// Lazily parse the embedded doc data JSON (parsed once, cached)
fn get_docs() -> &'static [docs::DocEntry] {
    use std::sync::OnceLock;
    static DOCS: OnceLock<Vec<docs::DocEntry>> = OnceLock::new();
    DOCS.get_or_init(|| {
        serde_json::from_str(EMBEDDED_DOC_DATA)
            .expect("BUG: embedded doc_data.json is malformed — rebuild with `cargo build`")
    })
}

/// Search the new doc data for a function by name (exact match)
fn search_docs(query: &str) -> Option<&'static docs::DocEntry> {
    get_docs().iter().find(|d| d.name == query)
}

/// Display a new-format doc entry
fn show_doc_entry(entry: &docs::DocEntry, json_output: bool) -> anyhow::Result<()> {
    if json_output {
        let output = serde_json::json!({
            "name": entry.name,
            "module": entry.module,
            "signature": entry.signature,
            "summary": entry.summary,
            "description": entry.description,
            "params": entry.params.iter().map(|p| serde_json::json!({
                "name": p.name, "description": p.description
            })).collect::<Vec<_>>(),
            "returns": entry.returns,
            "examples": entry.examples.iter().map(|e| serde_json::json!({
                "code": e.code, "expected": e.expected, "description": e.description
            })).collect::<Vec<_>>(),
            "see_also": entry.see_also,
            "since": entry.since,
            "tags": entry.tags,
            "errors": entry.errors.iter().map(|e| serde_json::json!({
                "error_type": e.error_type, "message": e.message, "fix": e.fix
            })).collect::<Vec<_>>(),
            "gotchas": entry.gotchas,
            "source_file": entry.source_file,
            "source_line": entry.source_line,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // Signature (or name as fallback)
    if let Some(sig) = &entry.signature {
        println!("{}", sig.green().bold());
    } else {
        println!("{}", entry.name.green().bold());
    }

    // Module
    if let Some(module) = &entry.module {
        println!("Module: {}", module.cyan());
    }

    // Since version
    if let Some(since) = &entry.since {
        println!("Since: {}", since.dimmed());
    }

    println!();

    // Summary
    println!("{}", entry.summary);

    // Extended description
    if let Some(desc) = &entry.description {
        println!();
        println!("{}", desc);
    }

    // Parameters
    if !entry.params.is_empty() {
        println!();
        println!("{}", "Parameters:".cyan().bold());
        for param in &entry.params {
            println!("  {} — {}", param.name.yellow(), param.description);
        }
    }

    // Returns
    if let Some(returns) = &entry.returns {
        println!();
        println!("{} {}", "Returns:".cyan().bold(), returns);
    }

    // Examples
    if !entry.examples.is_empty() {
        println!();
        println!("{}", "Examples:".cyan().bold());
        for ex in &entry.examples {
            if ex.code.contains('\n') {
                // Multi-line example
                if let Some(desc) = &ex.description {
                    println!("  {}", desc.dimmed());
                }
                for code_line in ex.code.lines() {
                    println!("    {}", code_line);
                }
                if let Some(expected) = &ex.expected {
                    println!("    => {}", expected);
                }
            } else {
                // Single-line example
                let mut line = format!("  {}", ex.code);
                if let Some(expected) = &ex.expected {
                    line.push_str(&format!(" => {}", expected));
                }
                if let Some(desc) = &ex.description {
                    line.push_str(&format!("  {}", desc.dimmed()));
                }
                println!("{}", line);
            }
        }
    }

    // Errors
    if !entry.errors.is_empty() {
        println!();
        println!("{}", "Errors:".yellow().bold());
        for err in &entry.errors {
            print!("  {} — {}", err.error_type.red(), err.message);
            if let Some(fix) = &err.fix {
                print!(" (fix: {})", fix);
            }
            println!();
        }
    }

    // Gotchas
    if !entry.gotchas.is_empty() {
        println!();
        println!("{}", "Gotchas:".yellow().bold());
        for gotcha in &entry.gotchas {
            println!("  - {}", gotcha);
        }
    }

    // See also
    if !entry.see_also.is_empty() {
        println!();
        println!(
            "{} {}",
            "See also:".dimmed(),
            entry.see_also.join(", ").dimmed()
        );
    }

    // Tags
    if !entry.tags.is_empty() {
        println!("{} {}", "Tags:".dimmed(), entry.tags.join(", ").dimmed());
    }

    Ok(())
}

/// Show a module's documentation from embedded JSON data
fn show_module_from_json(module_name: &str, json_output: bool) -> anyhow::Result<()> {
    let entries: Vec<&docs::DocEntry> = get_docs()
        .iter()
        .filter(|d| d.module.as_deref() == Some(module_name))
        .collect();

    if entries.is_empty() {
        return Err(anyhow::anyhow!(
            "No documentation found for module '{}'",
            module_name
        ));
    }

    if json_output {
        let func_names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        let output = serde_json::json!({
            "name": module_name,
            "functions": func_names,
            "count": entries.len()
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("{}", module_name.green().bold());
    println!();

    // Show import example
    println!("{}", "Import:".cyan().bold());
    let import_list: Vec<&str> = entries.iter().take(3).map(|e| e.name.as_str()).collect();
    println!(
        "  import {{ {} }} from \"{}\"",
        import_list.join(", "),
        module_name
    );
    println!();

    // Show functions
    println!("{}", "Functions:".cyan().bold());
    let mut sorted_entries: Vec<&&docs::DocEntry> = entries.iter().collect();
    sorted_entries.sort_by_key(|e| &e.name);
    for entry in sorted_entries {
        if let Some(sig) = &entry.signature {
            println!("  {}", sig.yellow());
        } else {
            println!("  {}", entry.name.yellow());
        }
        println!("    {}", entry.summary.dimmed());
    }

    Ok(())
}

/// List all modules and builtins from embedded JSON data
fn list_modules_from_json(json_output: bool) -> anyhow::Result<()> {
    let docs = get_docs();

    // Group by module
    let mut modules: std::collections::BTreeMap<String, Vec<&docs::DocEntry>> =
        std::collections::BTreeMap::new();
    let mut builtins: Vec<&docs::DocEntry> = Vec::new();

    for entry in docs {
        match &entry.module {
            Some(m) => modules.entry(m.clone()).or_default().push(entry),
            None => builtins.push(entry),
        }
    }

    if json_output {
        let mut output = serde_json::Map::new();
        output.insert(
            "builtins".to_string(),
            serde_json::json!({ "count": builtins.len() }),
        );
        let mut mods = serde_json::Map::new();
        for (name, entries) in &modules {
            mods.insert(
                name.clone(),
                serde_json::json!({ "functions": entries.len() }),
            );
        }
        output.insert("modules".to_string(), serde_json::Value::Object(mods));
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    let version = env!("CARGO_PKG_VERSION");
    println!("{}", "NTNT Standard Library".green().bold());
    println!("Version: {}", version);
    println!();

    // List builtins
    println!(
        "{} ({} functions)",
        "Global Builtins".cyan().bold(),
        builtins.len()
    );
    println!("  Available everywhere without importing");
    println!("  Run: ntnt docs <function_name>");
    println!();

    // List modules
    println!("{}", "Modules".cyan().bold());
    for (name, entries) in &modules {
        println!("  {} ({} functions)", name.yellow(), entries.len());
    }

    println!();
    println!(
        "Run {} for details on a module or function.",
        "ntnt docs <name>".cyan()
    );

    Ok(())
}

/// Search for a function or module using embedded JSON data
fn search_docs_from_json(query: &str, json_output: bool) -> anyhow::Result<()> {
    let query_lower = query.to_lowercase();

    // Try exact match first
    if let Some(entry) = search_docs(query) {
        return show_doc_entry(entry, json_output);
    }

    // Check if it's a module name (e.g., "std/string")
    let module_entries: Vec<&docs::DocEntry> = get_docs()
        .iter()
        .filter(|d| d.module.as_deref() == Some(query))
        .collect();
    if !module_entries.is_empty() {
        return show_module_from_json(query, json_output);
    }

    // Fuzzy search by name, summary, and description
    let matches: Vec<&docs::DocEntry> = get_docs()
        .iter()
        .filter(|d| {
            d.name.to_lowercase().contains(&query_lower)
                || d.summary.to_lowercase().contains(&query_lower)
        })
        .collect();

    if matches.is_empty() {
        // Try Levenshtein "Did you mean?" suggestion
        let candidates: Vec<String> = get_docs().iter().map(|d| d.name.clone()).collect();
        if let Some(suggestion) = ntnt::error::find_suggestion(query, &candidates) {
            println!(
                "{}: No documentation found for '{}'. Did you mean {}?",
                "Not found".red(),
                query,
                suggestion.green()
            );
        } else {
            println!(
                "{}: No documentation found for '{}'",
                "Not found".red(),
                query
            );
        }
        return Ok(());
    }

    if json_output {
        let results: Vec<serde_json::Value> = matches
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "name": entry.name,
                    "module": entry.module,
                    "signature": entry.signature,
                    "summary": entry.summary
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }

    println!(
        "Found {} matches for '{}':",
        matches.len().to_string().green(),
        query
    );
    println!();
    for entry in &matches {
        let module = entry.module.as_deref().unwrap_or("builtin");
        println!("{} ({})", entry.name.yellow().bold(), module.dimmed());
        if let Some(sig) = &entry.signature {
            println!("  {}", sig.cyan());
        }
        println!("  {}", entry.summary);
        println!();
    }

    Ok(())
}

/// Generate a GitHub-compatible anchor from a heading string.
fn github_anchor(heading: &str) -> String {
    heading
        .to_lowercase()
        .replace('/', "")
        .replace(' ', "-")
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "")
}

/// Write a detailed doc section for a single function into `md`.
fn write_function_detail(md: &mut String, entry: &docs::DocEntry) {
    md.push_str(&format!("#### `{}`\n\n", entry.name));

    // Signature
    if let Some(sig) = &entry.signature {
        md.push_str("```ntnt\n");
        md.push_str(sig);
        md.push('\n');
        md.push_str("```\n\n");
    }

    // Summary
    if !entry.summary.is_empty() {
        md.push_str(&entry.summary);
        md.push_str("\n\n");
    }

    // Extended description
    if let Some(desc) = &entry.description {
        md.push_str(desc);
        md.push_str("\n\n");
    }

    // Parameters
    if !entry.params.is_empty() {
        md.push_str("**Parameters:**\n\n");
        for p in &entry.params {
            md.push_str(&format!("- `{}` — {}\n", p.name, p.description));
        }
        md.push('\n');
    }

    // Returns
    if let Some(ret) = &entry.returns {
        md.push_str(&format!("**Returns:** {}\n\n", ret));
    }

    // Examples
    if !entry.examples.is_empty() {
        md.push_str("**Examples:**\n\n");
        md.push_str("```ntnt\n");
        for ex in &entry.examples {
            if ex.code.contains('\n') {
                // Multi-line example
                if let Some(desc) = &ex.description {
                    md.push_str(&format!("// {}\n", desc));
                }
                md.push_str(&ex.code);
                md.push('\n');
                if let Some(exp) = &ex.expected {
                    md.push_str(&format!("// => {}\n", exp));
                }
            } else {
                // Single-line example
                let mut line = ex.code.clone();
                if let Some(exp) = &ex.expected {
                    line.push_str(&format!("  // => {}", exp));
                }
                if let Some(desc) = &ex.description {
                    line.push_str(&format!("  // {}", desc));
                }
                md.push_str(&line);
                md.push('\n');
            }
        }
        md.push_str("```\n\n");
    }

    // Errors
    if !entry.errors.is_empty() {
        md.push_str("**Errors:**\n\n");
        for err in &entry.errors {
            let mut line = format!("- **{}**: {}", err.error_type, err.message);
            if let Some(fix) = &err.fix {
                line.push_str(&format!(" — *Fix: {}*", fix));
            }
            md.push_str(&line);
            md.push('\n');
        }
        md.push('\n');
    }

    // Gotchas
    if !entry.gotchas.is_empty() {
        md.push_str("**Gotchas:**\n\n");
        for g in &entry.gotchas {
            md.push_str(&format!("- {}\n", g));
        }
        md.push('\n');
    }

    // See also
    if !entry.see_also.is_empty() {
        let links: Vec<String> = entry.see_also.iter().map(|s| format!("`{}`", s)).collect();
        md.push_str(&format!("**See also:** {}\n\n", links.join(", ")));
    }

    // Since
    if let Some(since) = &entry.since {
        md.push_str(&format!("*Since {}*\n\n", since));
    }

    md.push_str("---\n\n");
}

/// Generate STDLIB_REFERENCE.md from embedded JSON doc data
fn generate_stdlib_markdown_from_json(output_dir: &std::path::Path) -> anyhow::Result<()> {
    let all_docs = get_docs();
    let version = env!("CARGO_PKG_VERSION");

    // Group by module
    let mut modules: std::collections::BTreeMap<String, Vec<&docs::DocEntry>> =
        std::collections::BTreeMap::new();
    let mut builtins: Vec<&docs::DocEntry> = Vec::new();

    for entry in all_docs {
        match &entry.module {
            Some(m) => modules.entry(m.clone()).or_default().push(entry),
            None => builtins.push(entry),
        }
    }

    // Collect module descriptions (first entry per module that has one)
    let mut module_descriptions: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for (name, entries) in &modules {
        for entry in entries {
            if let Some(desc) = &entry.module_description {
                module_descriptions.insert(name.clone(), desc.clone());
                break;
            }
        }
    }

    let mut md = String::new();

    // Header
    md.push_str("# NTNT Standard Library Reference\n\n");
    md.push_str("> **Auto-generated from source code doc comments** - Do not edit directly.\n");
    md.push_str(">\n");
    md.push_str(&format!("> Last updated: v{}\n\n", version));

    // Table of Contents
    md.push_str("## Table of Contents\n\n");
    md.push_str("- [Global Builtins](#global-builtins)\n");
    for name in modules.keys() {
        let anchor = github_anchor(name);
        md.push_str(&format!("- [{}](#{})\n", name, anchor));
    }
    md.push_str("\n---\n\n");

    // ---- Global Builtins ----
    md.push_str("## Global Builtins\n\n");
    md.push_str("These functions are available everywhere without importing.\n\n");

    builtins.sort_by_key(|e| &e.name);

    // Summary table with links to detail sections
    md.push_str("| Function | Description |\n");
    md.push_str("|----------|-------------|\n");
    for entry in &builtins {
        let display_name = entry
            .signature
            .as_ref()
            .map(|s| s.split("->").next().unwrap_or(s).trim().to_string())
            .unwrap_or_else(|| entry.name.clone())
            .replace('|', "\\|");
        let anchor = github_anchor(&entry.name);
        let desc = entry.summary.replace('|', "\\|");
        md.push_str(&format!(
            "| [`{}`](#{}) | {} |\n",
            display_name, anchor, desc
        ));
    }
    md.push('\n');

    // Detail sections for each builtin
    for entry in &builtins {
        write_function_detail(&mut md, entry);
    }

    // ---- Modules ----
    for (name, entries) in &modules {
        md.push_str(&format!("## {}\n\n", name));

        // Module description
        if let Some(desc) = module_descriptions.get(name) {
            md.push_str(desc);
            md.push_str("\n\n");
        }

        // Import example
        md.push_str("```ntnt\n");
        let import_list: Vec<&str> = entries.iter().take(3).map(|e| e.name.as_str()).collect();
        md.push_str(&format!(
            "import {{ {} }} from \"{}\"\n",
            import_list.join(", "),
            name
        ));
        md.push_str("```\n\n");

        // Summary table with links
        let mut sorted: Vec<&&docs::DocEntry> = entries.iter().collect();
        sorted.sort_by_key(|e| &e.name);

        md.push_str("### Functions\n\n");
        md.push_str("| Function | Description |\n");
        md.push_str("|----------|-------------|\n");
        for entry in &sorted {
            let anchor = github_anchor(&entry.name);
            let desc = entry.summary.replace('|', "\\|");
            md.push_str(&format!("| [`{}`](#{}) | {} |\n", entry.name, anchor, desc));
        }
        md.push('\n');

        // Detail sections for each function
        for entry in &sorted {
            write_function_detail(&mut md, entry);
        }
    }

    // Write to file
    let output_path = output_dir.join("STDLIB_REFERENCE.md");
    fs::write(&output_path, &md)?;

    // Count totals
    let total_funcs: usize = builtins.len() + modules.values().map(|v| v.len()).sum::<usize>();
    println!(
        "{} Generated {}",
        "✓".green(),
        output_path.display().to_string().cyan()
    );
    println!(
        "  {} builtins, {} modules, {} total functions",
        builtins.len().to_string().cyan(),
        modules.len().to_string().cyan(),
        total_funcs.to_string().cyan()
    );

    Ok(())
}

/// Run the docs command
fn run_docs_command(
    query: Option<String>,
    validate: bool,
    generate_md: bool,
    json_output: bool,
) -> anyhow::Result<()> {
    if generate_md {
        let docs_dir = find_docs_dir()?;
        // Stdlib reference from embedded JSON — no TOML needed
        generate_stdlib_markdown_from_json(&docs_dir)?;
        // Syntax, IAL, runtime still generated from TOML
        if docs_dir.join("syntax.toml").exists() {
            generate_syntax_markdown(&docs_dir)?;
            generate_ial_markdown(&docs_dir)?;
            generate_runtime_markdown(&docs_dir)?;
        } else {
            println!(
                "  {} TOML files not found, skipping syntax/IAL/runtime docs",
                "!".yellow()
            );
        }
        // Sync agent instruction files from AI_AGENT_GUIDE.md
        sync_agent_files(&docs_dir)?;

        return Ok(());
    }

    if validate {
        return validate_docs_from_json();
    }

    match query {
        None => list_modules_from_json(json_output),
        Some(q) => search_docs_from_json(&q, json_output),
    }
}

/// Find the docs/ directory
fn find_docs_dir() -> anyhow::Result<PathBuf> {
    let candidates = [
        PathBuf::from("docs"),
        PathBuf::from("../docs"),
        PathBuf::from("ntnt/docs"),
    ];
    for path in &candidates {
        if path.is_dir() {
            return Ok(path.clone());
        }
    }
    // Try relative to executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let path = parent.join("docs");
            if path.is_dir() {
                return Ok(path);
            }
            if let Some(grandparent) = parent.parent() {
                let path = grandparent.join("share/ntnt/docs");
                if path.is_dir() {
                    return Ok(path);
                }
            }
        }
    }
    // Try home directory starter kit
    if let Ok(home) = std::env::var("HOME") {
        let path = PathBuf::from(home).join("ntnt/docs");
        if path.is_dir() {
            return Ok(path);
        }
    }
    Err(anyhow::anyhow!(
        "Could not find docs/ directory. Run from the NTNT project directory."
    ))
}

/// Generate SYNTAX_REFERENCE.md from syntax.toml
fn generate_syntax_markdown(docs_dir: &std::path::Path) -> anyhow::Result<()> {
    let syntax_path = docs_dir.join("syntax.toml");
    if !syntax_path.exists() {
        println!(
            "  {} syntax.toml not found, skipping SYNTAX_REFERENCE.md",
            "!".yellow()
        );
        return Ok(());
    }

    let content = fs::read_to_string(&syntax_path)?;
    let syntax: toml::Value = toml::from_str(&content)?;

    let mut md = String::new();
    let version = env!("CARGO_PKG_VERSION");

    // Header
    md.push_str("# NTNT Syntax Reference\n\n");
    md.push_str("> **Auto-generated from [syntax.toml](syntax.toml)** - Do not edit directly.\n");
    md.push_str(">\n");
    md.push_str(&format!("> Last updated: v{}\n\n", version));

    // Table of Contents
    md.push_str("## Table of Contents\n\n");
    md.push_str("- [Keywords](#keywords)\n");
    md.push_str("- [Operators](#operators)\n");
    md.push_str("- [Literals](#literals)\n");
    md.push_str("- [Escape Sequences](#escape-sequences)\n");
    md.push_str("- [String Interpolation](#string-interpolation)\n");
    md.push_str("- [Template Strings](#template-strings)\n");
    md.push_str("- [Truthy/Falsy Values](#truthyfalsy-values)\n");
    md.push_str("- [Contracts](#contracts)\n");
    md.push_str("- [Types](#types)\n");
    md.push_str("- [Imports](#imports)\n");
    md.push_str("- [Match Expressions](#match-expressions)\n");
    md.push_str("- [Destructuring Patterns](#destructuring-patterns)\n");
    md.push_str("- [Function Parameters](#function-parameters)\n");
    md.push_str("\n---\n\n");

    // Keywords
    if let Some(keywords) = syntax.get("keywords") {
        md.push_str("## Keywords\n\n");
        if let Some(desc) = keywords.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        let categories = [
            ("contracts", "Contracts"),
            ("functions", "Functions"),
            ("variables", "Variables"),
            ("control_flow", "Control Flow"),
            ("error_handling", "Error Handling"),
            ("types", "Types"),
            ("modules", "Modules"),
            ("literals", "Literals"),
        ];

        for (key, title) in &categories {
            if let Some(cat) = keywords.get(*key) {
                md.push_str(&format!("### {}\n\n", title));
                if let Some(words) = cat.get("words").and_then(|v| v.as_array()) {
                    let word_list: Vec<_> = words
                        .iter()
                        .filter_map(|w| w.as_str())
                        .map(|w| format!("`{}`", w))
                        .collect();
                    md.push_str(&format!("{}\n\n", word_list.join(", ")));
                }
                if let Some(desc) = cat.get("description").and_then(|v| v.as_str()) {
                    md.push_str(&format!("_{}_\n\n", desc));
                }
            }
        }
        md.push_str("---\n\n");
    }

    // Operators
    if let Some(operators) = syntax.get("operators") {
        md.push_str("## Operators\n\n");
        if let Some(desc) = operators.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        md.push_str("| Category | Operators | Description | Example |\n");
        md.push_str("|----------|-----------|-------------|----------|\n");

        let op_categories = [
            "assignment",
            "logical_or",
            "logical_and",
            "comparison",
            "arithmetic",
            "unary",
            "range",
            "null_coalesce",
            "postfix",
            "member",
            "pipe",
        ];

        for cat in &op_categories {
            if let Some(op) = operators.get(*cat) {
                let symbols = op
                    .get("symbols")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str())
                            .map(|s| format!("`{}`", s))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                let desc = op.get("description").and_then(|v| v.as_str()).unwrap_or("");
                let example = op.get("example").and_then(|v| v.as_str()).unwrap_or("");
                md.push_str(&format!(
                    "| {} | {} | {} | `{}` |\n",
                    cat.replace("_", " "),
                    symbols,
                    desc,
                    example
                ));
            }
        }
        md.push_str("\n---\n\n");
    }

    // Literals
    if let Some(literals) = syntax.get("literals") {
        md.push_str("## Literals\n\n");
        if let Some(desc) = literals.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        md.push_str("| Type | Syntax | Description |\n");
        md.push_str("|------|--------|-------------|\n");

        let lit_types = [
            "integers",
            "floats",
            "strings",
            "raw_strings",
            "template_strings",
            "booleans",
            "arrays",
            "maps",
            "ranges",
            "closures",
            "if_expression",
        ];

        for lit in &lit_types {
            if let Some(l) = literals.get(*lit) {
                let syntax_str = l.get("syntax").and_then(|v| v.as_str()).unwrap_or("");
                let desc = l.get("description").and_then(|v| v.as_str()).unwrap_or("");
                md.push_str(&format!("| {} | `{}` | {} |\n", lit, syntax_str, desc));
            }
        }
        md.push_str("\n---\n\n");
    }

    // Escape Sequences
    if let Some(escapes) = syntax.get("escapes") {
        md.push_str("## Escape Sequences\n\n");
        if let Some(desc) = escapes.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        if let Some(seqs) = escapes.get("sequences").and_then(|v| v.as_table()) {
            md.push_str("| Escape | Result |\n");
            md.push_str("|--------|--------|\n");
            for (escape, result) in seqs {
                if let Some(r) = result.as_str() {
                    md.push_str(&format!("| `{}` | {} |\n", escape, r));
                }
            }
        }
        md.push_str("\n---\n\n");
    }

    // String Interpolation
    if let Some(interp) = syntax.get("interpolation") {
        md.push_str("## String Interpolation\n\n");
        if let Some(desc) = interp.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        if let Some(regular) = interp.get("regular") {
            md.push_str("### Regular Strings\n\n");
            if let Some(syntax_str) = regular.get("syntax").and_then(|v| v.as_str()) {
                md.push_str(&format!("Syntax: `{}`\n\n", syntax_str));
            }
            if let Some(desc) = regular.get("description").and_then(|v| v.as_str()) {
                md.push_str(&format!("{}\n\n", desc));
            }
        }

        if let Some(template) = interp.get("template") {
            md.push_str("### Template Strings\n\n");
            if let Some(syntax_str) = template.get("syntax").and_then(|v| v.as_str()) {
                md.push_str(&format!("Syntax: `{}`\n\n", syntax_str));
            }
            if let Some(desc) = template.get("description").and_then(|v| v.as_str()) {
                md.push_str(&format!("{}\n\n", desc));
            }
        }
        md.push_str("---\n\n");
    }

    // Template Strings section
    if let Some(templates) = syntax.get("templates") {
        md.push_str("## Template Strings\n\n");
        if let Some(desc) = templates.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        md.push_str("| Feature | Syntax | Description |\n");
        md.push_str("|---------|--------|-------------|\n");

        let features = [
            "interpolation",
            "filters",
            "loops",
            "empty_fallback",
            "conditionals",
            "if_else",
            "elif",
            "comments",
            "escape_braces",
            "partials",
            "partials_data",
        ];

        for feat in &features {
            if let Some(f) = templates.get(*feat) {
                let syntax_str = f.get("syntax").and_then(|v| v.as_str()).unwrap_or("");
                let desc = f.get("description").and_then(|v| v.as_str()).unwrap_or("");
                // Escape pipes in table
                let syntax_escaped = syntax_str.replace("|", "\\|");
                md.push_str(&format!("| {} | `{}` | {} |\n", feat, syntax_escaped, desc));
            }
        }

        // Filters list
        if let Some(filters) = templates.get("filters") {
            if let Some(available) = filters.get("available_filters").and_then(|v| v.as_array()) {
                md.push_str("\n### Available Filters\n\n");
                let filter_list: Vec<_> = available
                    .iter()
                    .filter_map(|f| f.as_str())
                    .map(|f| format!("`{}`", f))
                    .collect();
                md.push_str(&format!("{}\n", filter_list.join(", ")));
            }
        }

        // Loop metadata
        if let Some(loops) = templates.get("loops") {
            if let Some(metadata) = loops.get("metadata_vars").and_then(|v| v.as_array()) {
                md.push_str("\n### Loop Metadata Variables\n\n");
                for var in metadata {
                    if let Some(v) = var.as_str() {
                        md.push_str(&format!("- `{}`\n", v));
                    }
                }
            }
        }
        md.push_str("\n---\n\n");
    }

    // Truthy/Falsy
    if let Some(tf) = syntax.get("truthy_falsy") {
        md.push_str("## Truthy/Falsy Values\n\n");
        if let Some(desc) = tf.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        if let Some(truthy) = tf.get("truthy") {
            md.push_str("### Truthy\n\n");
            if let Some(values) = truthy.get("values").and_then(|v| v.as_array()) {
                for val in values {
                    if let Some(v) = val.as_str() {
                        md.push_str(&format!("- `{}`\n", v));
                    }
                }
            }
            if let Some(note) = truthy.get("note").and_then(|v| v.as_str()) {
                md.push_str(&format!("\n**Note:** {}\n", note));
            }
            md.push_str("\n");
        }

        if let Some(falsy) = tf.get("falsy") {
            md.push_str("### Falsy\n\n");
            if let Some(values) = falsy.get("values").and_then(|v| v.as_array()) {
                for val in values {
                    if let Some(v) = val.as_str() {
                        md.push_str(&format!("- `{}`\n", v));
                    }
                }
            }
            md.push_str("\n");
        }
        md.push_str("---\n\n");
    }

    // Contracts
    if let Some(contracts) = syntax.get("contracts") {
        md.push_str("## Contracts\n\n");
        if let Some(desc) = contracts.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        md.push_str("| Keyword | Syntax | Description |\n");
        md.push_str("|---------|--------|-------------|\n");

        let keywords = ["requires", "ensures", "old", "result", "invariant"];
        for kw in &keywords {
            if let Some(c) = contracts.get(*kw) {
                let syntax_str = c.get("syntax").and_then(|v| v.as_str()).unwrap_or("");
                let desc = c.get("description").and_then(|v| v.as_str()).unwrap_or("");
                md.push_str(&format!("| `{}` | `{}` | {} |\n", kw, syntax_str, desc));
            }
        }

        if let Some(placement) = contracts.get("placement") {
            md.push_str("\n### Placement\n\n");
            if let Some(desc) = placement.get("description").and_then(|v| v.as_str()) {
                md.push_str(&format!("{}\n\n", desc));
            }
            if let Some(example) = placement.get("example").and_then(|v| v.as_str()) {
                md.push_str("```ntnt\n");
                md.push_str(example);
                md.push_str("\n```\n");
            }
        }
        md.push_str("\n---\n\n");
    }

    // Types
    if let Some(types) = syntax.get("types") {
        md.push_str("## Types\n\n");
        if let Some(desc) = types.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        let type_categories = [
            "primitives",
            "compound",
            "option_result",
            "union",
            "annotation",
            "optional_shorthand",
            "type_alias",
            "function_type",
            "array_type",
            "generics",
        ];
        for cat in &type_categories {
            if let Some(t) = types.get(*cat) {
                md.push_str(&format!("### {}\n\n", cat.replace("_", " ").to_uppercase()));
                if let Some(type_list) = t.get("types").and_then(|v| v.as_array()) {
                    let types_str: Vec<_> = type_list
                        .iter()
                        .filter_map(|t| t.as_str())
                        .map(|t| format!("`{}`", t))
                        .collect();
                    md.push_str(&format!("{}\n\n", types_str.join(", ")));
                }
                if let Some(syntax_str) = t.get("syntax").and_then(|v| v.as_str()) {
                    md.push_str(&format!("Syntax: `{}`\n\n", syntax_str));
                }
                if let Some(desc) = t.get("description").and_then(|v| v.as_str()) {
                    md.push_str(&format!("{}\n\n", desc));
                }
                // Render functions table if present
                if let Some(functions) = t.get("functions").and_then(|v| v.as_array()) {
                    if !functions.is_empty() {
                        md.push_str("| Function | Description | Example |\n");
                        md.push_str("|----------|-------------|---------|\n");
                        for func in functions {
                            let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let desc = func
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let example =
                                func.get("example").and_then(|v| v.as_str()).unwrap_or("");
                            md.push_str(&format!("| `{}` | {} | `{}` |\n", name, desc, example));
                        }
                        md.push_str("\n");
                    }
                }
            }
        }
        md.push_str("---\n\n");
    }

    // Imports
    if let Some(imports) = syntax.get("imports") {
        md.push_str("## Imports\n\n");
        if let Some(desc) = imports.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        md.push_str("| Style | Syntax | Example |\n");
        md.push_str("|-------|--------|----------|\n");

        let styles = ["named", "aliased", "namespace", "local"];
        for style in &styles {
            if let Some(s) = imports.get(*style) {
                let syntax_str = s.get("syntax").and_then(|v| v.as_str()).unwrap_or("");
                let example = s.get("example").and_then(|v| v.as_str()).unwrap_or("");
                md.push_str(&format!(
                    "| {} | `{}` | `{}` |\n",
                    style, syntax_str, example
                ));
            }
        }
        md.push_str("\n---\n\n");
    }

    // Match
    if let Some(match_expr) = syntax.get("match") {
        md.push_str("## Match Expressions\n\n");
        if let Some(desc) = match_expr.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        md.push_str("| Feature | Syntax | Description |\n");
        md.push_str("|---------|--------|-------------|\n");

        let features = ["basic", "guards", "wildcard", "binding"];
        for feat in &features {
            if let Some(f) = match_expr.get(*feat) {
                let syntax_str = f.get("syntax").and_then(|v| v.as_str()).unwrap_or("");
                let desc = f.get("description").and_then(|v| v.as_str()).unwrap_or("");
                md.push_str(&format!("| {} | `{}` | {} |\n", feat, syntax_str, desc));
            }
        }
        md.push_str("\n---\n\n");
    }

    // Destructuring Patterns
    if let Some(destructuring) = syntax.get("destructuring") {
        md.push_str("## Destructuring Patterns\n\n");

        md.push_str("| Pattern | Syntax | Description |\n");
        md.push_str("|---------|--------|-------------|\n");

        let patterns = [
            "map_basic",
            "map_rename",
            "map_nested",
            "array",
            "array_rest",
            "map_rest",
            "for_loop",
            "spread_token",
        ];
        for pat in &patterns {
            if let Some(p) = destructuring.get(*pat) {
                let syntax_str = p.get("syntax").and_then(|v| v.as_str()).unwrap_or("");
                let desc = p.get("description").and_then(|v| v.as_str()).unwrap_or("");
                md.push_str(&format!(
                    "| {} | `{}` | {} |\n",
                    pat.replace("_", " "),
                    syntax_str,
                    desc
                ));
            }
        }
        md.push_str("\n---\n\n");
    }

    // Function Parameters
    if let Some(func_params) = syntax.get("function_parameters") {
        md.push_str("## Function Parameters\n\n");
        if let Some(desc) = func_params.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        md.push_str("| Feature | Syntax | Description |\n");
        md.push_str("|---------|--------|-------------|\n");

        let features = [
            "basic",
            "typed",
            "default_values",
            "default_typed",
            "default_reference",
        ];
        for feat in &features {
            if let Some(f) = func_params.get(*feat) {
                let syntax_str = f.get("syntax").and_then(|v| v.as_str()).unwrap_or("");
                let desc = f.get("description").and_then(|v| v.as_str()).unwrap_or("");
                md.push_str(&format!(
                    "| {} | `{}` | {} |\n",
                    feat.replace("_", " "),
                    syntax_str,
                    desc
                ));
            }
        }
        md.push_str("\n");
    }

    // Write to file
    let output_path = docs_dir.join("SYNTAX_REFERENCE.md");
    fs::write(&output_path, &md)?;

    println!(
        "{} Generated {}",
        "✓".green(),
        output_path.display().to_string().cyan()
    );

    Ok(())
}

/// Generate IAL_REFERENCE.md from ial.toml
fn generate_ial_markdown(docs_dir: &std::path::Path) -> anyhow::Result<()> {
    let ial_path = docs_dir.join("ial.toml");
    if !ial_path.exists() {
        println!(
            "  {} ial.toml not found, skipping IAL_REFERENCE.md",
            "!".yellow()
        );
        return Ok(());
    }

    let content = fs::read_to_string(&ial_path)?;
    let ial: toml::Value = toml::from_str(&content)?;

    let mut md = String::new();
    let version = env!("CARGO_PKG_VERSION");

    // Header
    md.push_str("# Intent Assertion Language (IAL) Reference\n\n");
    md.push_str("> **Auto-generated from [ial.toml](ial.toml)** - Do not edit directly.\n");
    md.push_str(">\n");
    md.push_str(&format!("> Last updated: v{}\n\n", version));

    if let Some(desc) = ial
        .get("meta")
        .and_then(|m| m.get("description"))
        .and_then(|v| v.as_str())
    {
        md.push_str(&format!("{}\n\n", desc));
    }

    // Table of Contents
    md.push_str("## Table of Contents\n\n");
    md.push_str("- [Primitives](#primitives)\n");
    md.push_str("- [Check Operations](#check-operations)\n");
    md.push_str("- [Standard Terms](#standard-terms)\n");
    md.push_str("- [Context Paths](#context-paths)\n");
    md.push_str("- [Glossary System](#glossary-system)\n");
    md.push_str("- [Intent File Format](#intent-file-format)\n");
    md.push_str("- [Commands](#commands)\n");
    md.push_str("\n---\n\n");

    // Primitives
    if let Some(primitives) = ial.get("primitives") {
        md.push_str("## Primitives\n\n");
        if let Some(desc) = primitives.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        md.push_str("| Primitive | Description | Context Sets |\n");
        md.push_str("|-----------|-------------|---------------|\n");

        let prim_names = [
            "http",
            "cli",
            "code_quality",
            "read_file",
            "function_call",
            "property_check",
            "check",
        ];
        for prim in &prim_names {
            if let Some(p) = primitives.get(*prim) {
                let name = p.get("name").and_then(|v| v.as_str()).unwrap_or(*prim);
                let desc = p.get("description").and_then(|v| v.as_str()).unwrap_or("");
                let context = p
                    .get("context_sets")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str())
                            .map(|s| format!("`{}`", s))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                md.push_str(&format!("| **{}** | {} | {} |\n", name, desc, context));
            }
        }
        md.push_str("\n---\n\n");
    }

    // Check Operations
    if let Some(check_ops) = ial.get("check_operations") {
        md.push_str("## Check Operations\n\n");
        if let Some(desc) = check_ops.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        let categories = [
            ("equality", "Equality"),
            ("containment", "Containment"),
            ("pattern", "Pattern Matching"),
            ("existence", "Existence"),
            ("comparison", "Comparison"),
            ("type", "Type Checks"),
        ];

        for (key, title) in &categories {
            if let Some(cat) = check_ops.get(*key).and_then(|v| v.as_table()) {
                md.push_str(&format!("### {}\n\n", title));
                md.push_str("| Operation | Description |\n");
                md.push_str("|-----------|-------------|\n");
                for (op, desc) in cat {
                    if let Some(d) = desc.as_str() {
                        md.push_str(&format!("| `{}` | {} |\n", op, d));
                    }
                }
                md.push_str("\n");
            }
        }
        md.push_str("---\n\n");
    }

    // Standard Terms
    if let Some(terms) = ial.get("standard_terms") {
        md.push_str("## Standard Terms\n\n");
        if let Some(desc) = terms.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        let categories = [
            ("http_status", "HTTP Status"),
            ("http_body", "HTTP Body"),
            ("http_headers", "HTTP Headers"),
            ("content_type", "Content-Type Shortcuts"),
            ("response_time", "Response Time"),
            ("cli", "CLI"),
            ("code_quality", "Code Quality"),
            ("unit_test", "Unit Test Results"),
            ("properties", "Function Properties"),
            ("string_checks", "String/Value Checks"),
            ("bounds", "Bounds"),
        ];

        for (key, title) in &categories {
            if let Some(cat) = terms.get(*key).and_then(|v| v.as_table()) {
                md.push_str(&format!("### {}\n\n", title));

                // Get description if present
                if let Some(desc) = cat.get("description").and_then(|v| v.as_str()) {
                    md.push_str(&format!("_{}_\n\n", desc));
                }

                md.push_str("| Term | Resolves To |\n");
                md.push_str("|------|-------------|\n");

                for (term, value) in cat {
                    if term == "description" {
                        continue;
                    }
                    if let Some(t) = value.as_table() {
                        let resolves = t.get("resolves_to").and_then(|v| v.as_str()).unwrap_or("");
                        md.push_str(&format!("| `{}` | `{}` |\n", term, resolves));
                    }
                }
                md.push_str("\n");
            }
        }
        md.push_str("---\n\n");
    }

    // Context Paths
    if let Some(context) = ial.get("context") {
        md.push_str("## Context Paths\n\n");
        if let Some(desc) = context.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        let sections = ["response", "cli", "code_quality", "result", "file"];
        for section in &sections {
            if let Some(sec) = context.get(*section).and_then(|v| v.as_table()) {
                md.push_str(&format!("### {}\n\n", section));
                md.push_str("| Path | Description |\n");
                md.push_str("|------|-------------|\n");
                for (path, desc) in sec {
                    if let Some(d) = desc.as_str() {
                        md.push_str(&format!("| `{}` | {} |\n", path, d));
                    }
                }
                md.push_str("\n");
            }
        }
        md.push_str("---\n\n");
    }

    // Glossary System
    if let Some(glossary) = ial.get("glossary") {
        md.push_str("## Glossary System\n\n");
        if let Some(desc) = glossary.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        if let Some(format) = glossary.get("format") {
            if let Some(syntax) = format.get("syntax").and_then(|v| v.as_str()) {
                md.push_str("### Format\n\n");
                md.push_str("```intent\n");
                md.push_str(syntax);
                md.push_str("\n```\n\n");
            }
        }

        if let Some(params) = glossary.get("parameters") {
            md.push_str("### Parameters\n\n");
            if let Some(desc) = params.get("description").and_then(|v| v.as_str()) {
                md.push_str(&format!("{}\n\n", desc));
            }
            if let Some(example) = params.get("example").and_then(|v| v.as_str()) {
                md.push_str(&format!("Example: `{}`\n\n", example));
            }
        }

        // Keyword Syntax for Unit Tests
        if let Some(keywords) = glossary.get("keywords") {
            md.push_str("### Keyword Syntax for Unit Tests\n\n");
            if let Some(desc) = keywords.get("description").and_then(|v| v.as_str()) {
                md.push_str(&format!("{}\n\n", desc));
            }
            if let Some(example) = keywords.get("example").and_then(|v| v.as_str()) {
                md.push_str("```intent\n");
                md.push_str(example);
                md.push_str("```\n\n");
            }

            if let Some(table) = keywords.get("table").and_then(|v| v.as_table()) {
                md.push_str("**Keywords:**\n\n");
                md.push_str("| Keyword | Description | Example |\n");
                md.push_str("|---------|-------------|---------|\n");
                for (keyword, info) in table {
                    if let Some(info_table) = info.as_table() {
                        let desc = info_table
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let example = info_table
                            .get("example")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        md.push_str(&format!("| `{}` | {} | `{}` |\n", keyword, desc, example));
                    }
                }
                md.push_str("\n");
            }

            if let Some(usage) = keywords.get("usage") {
                md.push_str("**Usage in Scenarios:**\n\n");
                if let Some(example) = usage.get("example").and_then(|v| v.as_str()) {
                    md.push_str("```intent\n");
                    md.push_str(example);
                    md.push_str("```\n\n");
                }
                if let Some(note) = usage.get("note").and_then(|v| v.as_str()) {
                    md.push_str(&format!("{}\n\n", note));
                }
            }
        }

        if let Some(resolution) = glossary.get("resolution") {
            md.push_str("### Resolution Order\n\n");
            if let Some(order) = resolution.get("order").and_then(|v| v.as_array()) {
                for step in order {
                    if let Some(s) = step.as_str() {
                        md.push_str(&format!(
                            "{}. {}\n",
                            s.chars().next().unwrap_or('1'),
                            &s[3..]
                        ));
                    }
                }
                md.push_str("\n");
            }
        }
        md.push_str("---\n\n");
    }

    // Intent File Format
    if let Some(intent_file) = ial.get("intent_file") {
        md.push_str("## Intent File Format\n\n");
        if let Some(desc) = intent_file.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        if let Some(structure) = intent_file.get("structure") {
            if let Some(example) = structure.get("example").and_then(|v| v.as_str()) {
                md.push_str("### Structure\n\n");
                md.push_str("```intent\n");
                md.push_str(example);
                md.push_str("\n```\n\n");
            }
        }

        if let Some(linking) = intent_file.get("linking") {
            md.push_str("### File Linking\n\n");
            if let Some(desc) = linking.get("description").and_then(|v| v.as_str()) {
                md.push_str(&format!("{}\n\n", desc));
            }
            if let Some(examples) = linking.get("examples").and_then(|v| v.as_array()) {
                for ex in examples {
                    if let Some(e) = ex.as_str() {
                        md.push_str(&format!("- `{}`\n", e));
                    }
                }
                md.push_str("\n");
            }
        }

        if let Some(annotations) = intent_file.get("annotations").and_then(|v| v.as_table()) {
            md.push_str("### Code Annotations\n\n");
            if let Some(desc) = annotations.get("description").and_then(|v| v.as_str()) {
                md.push_str(&format!("{}\n\n", desc));
            }
            md.push_str("| Annotation | Purpose |\n");
            md.push_str("|------------|----------|\n");
            for (ann, purpose) in annotations {
                if ann == "description" || ann == "example" {
                    continue;
                }
                if let Some(p) = purpose.as_str() {
                    md.push_str(&format!("| `{}` | {} |\n", ann, p));
                }
            }
            md.push_str("\n");
        }
        md.push_str("---\n\n");
    }

    // Commands
    if let Some(commands) = ial.get("commands") {
        md.push_str("## Commands\n\n");
        if let Some(desc) = commands.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        md.push_str("| Command | Description |\n");
        md.push_str("|---------|-------------|\n");

        let cmd_names = ["check", "coverage", "init", "studio"];
        for cmd in &cmd_names {
            if let Some(c) = commands.get(*cmd) {
                let command = c.get("command").and_then(|v| v.as_str()).unwrap_or("");
                let desc = c.get("description").and_then(|v| v.as_str()).unwrap_or("");
                md.push_str(&format!("| `{}` | {} |\n", command, desc));
            }
        }
        md.push_str("\n");
    }

    // Write to file
    let output_path = docs_dir.join("IAL_REFERENCE.md");
    fs::write(&output_path, &md)?;

    println!(
        "{} Generated {}",
        "✓".green(),
        output_path.display().to_string().cyan()
    );

    Ok(())
}

/// Generate RUNTIME_REFERENCE.md from runtime.toml
fn generate_runtime_markdown(docs_dir: &std::path::Path) -> anyhow::Result<()> {
    let runtime_path = docs_dir.join("runtime.toml");
    if !runtime_path.exists() {
        println!(
            "  {} runtime.toml not found, skipping RUNTIME_REFERENCE.md",
            "!".yellow()
        );
        return Ok(());
    }

    let content = fs::read_to_string(&runtime_path)?;
    let runtime: toml::Value = toml::from_str(&content)?;

    let mut md = String::new();
    let version = env!("CARGO_PKG_VERSION");

    // Header
    md.push_str("# NTNT Runtime & CLI Reference\n\n");
    md.push_str("> **Auto-generated from [runtime.toml](runtime.toml)** - Do not edit directly.\n");
    md.push_str(">\n");
    md.push_str(&format!("> Last updated: v{}\n\n", version));

    // Description
    if let Some(desc) = runtime
        .get("meta")
        .and_then(|m| m.get("description"))
        .and_then(|v| v.as_str())
    {
        md.push_str(&format!("{}\n\n", desc));
    }

    // Table of Contents
    md.push_str("## Table of Contents\n\n");
    md.push_str("- [Environment Variables](#environment-variables)\n");
    md.push_str("- [Hot-Reload](#hot-reload)\n");
    md.push_str("- [HTTP Server](#http-server)\n");
    md.push_str("- [File-Based Routing](#file-based-routing)\n");
    md.push_str("- [Project Structure](#project-structure)\n");
    md.push_str("- [CLI Commands](#cli-commands)\n");
    md.push_str("\n---\n\n");

    // Environment Variables
    if let Some(env_vars) = runtime.get("env_vars") {
        md.push_str("## Environment Variables\n\n");
        if let Some(desc) = env_vars.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        md.push_str("| Variable | Values | Default | Description |\n");
        md.push_str("|----------|--------|---------|-------------|\n");

        // Iterate all env vars from runtime.toml (skip the "description" key)
        let env_table = env_vars.as_table().cloned().unwrap_or_default();
        let mut env_var_names: Vec<&str> = env_table
            .keys()
            .filter(|k| k.as_str() != "description")
            .map(|k| k.as_str())
            .collect();
        env_var_names.sort();
        for var_name in &env_var_names {
            if let Some(env) = env_vars.get(*var_name) {
                // "values" (array) takes priority over "type" (string) for the Values column
                let values_col = env
                    .get("values")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(|s| format!("`{}`", s))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .or_else(|| env.get("type").and_then(|v| v.as_str()).map(String::from))
                    .unwrap_or_else(|| "-".to_string());
                let default = env.get("default").and_then(|v| v.as_str()).unwrap_or("-");
                let desc = env
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-");
                md.push_str(&format!(
                    "| `{}` | {} | {} | {} |\n",
                    var_name, values_col, default, desc
                ));
            }
        }

        // Examples from runtime.toml
        md.push_str("\n### Examples\n\n```bash\n");
        for var_name in &env_var_names {
            if let Some(env) = env_vars.get(*var_name) {
                if let Some(example) = env.get("example").and_then(|v| v.as_str()) {
                    let desc = env
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Usage");
                    // Use first sentence of description as comment
                    let comment = desc.split(". ").next().unwrap_or(desc);
                    md.push_str(&format!("# {}\n{}\n\n", comment, example));
                }
            }
        }
        md.push_str("```\n\n");
        md.push_str("---\n\n");
    }

    // Type Safety Modes section (DD-009)
    if let Some(tsm) = runtime.get("type_safety_modes") {
        md.push_str("## Type Safety Modes\n\n");
        if let Some(content) = tsm.get("content").and_then(|v| v.as_str()) {
            md.push_str(content);
            md.push_str("\n\n");
        }
        md.push_str("---\n\n");
    }

    // Hot-Reload
    if let Some(hot_reload) = runtime.get("hot_reload") {
        md.push_str("## Hot-Reload\n\n");
        if let Some(desc) = hot_reload.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }
        if let Some(default) = hot_reload.get("default").and_then(|v| v.as_str()) {
            md.push_str(&format!("**Default:** {}\n\n", default));
        }
        if let Some(disable) = hot_reload.get("disable").and_then(|v| v.as_str()) {
            md.push_str(&format!("**Disable:** {}\n\n", disable));
        }

        if let Some(tracked) = hot_reload.get("tracked_files") {
            md.push_str("### Tracked Files\n\n");
            if let Some(files) = tracked.get("files").and_then(|v| v.as_array()) {
                for file in files {
                    if let Some(f) = file.as_str() {
                        md.push_str(&format!("- {}\n", f));
                    }
                }
                md.push_str("\n");
            }
        }

        if let Some(behavior) = hot_reload.get("behavior") {
            md.push_str("### Behavior\n\n");
            if let Some(trigger) = behavior.get("trigger").and_then(|v| v.as_str()) {
                md.push_str(&format!("- **Trigger:** {}\n", trigger));
            }
            if let Some(action) = behavior.get("action").and_then(|v| v.as_str()) {
                md.push_str(&format!("- **Action:** {}\n", action));
            }
            if let Some(output) = behavior.get("output").and_then(|v| v.as_str()) {
                md.push_str(&format!("- **Output:** `{}`\n", output));
            }
            md.push_str("\n");
        }
        md.push_str("---\n\n");
    }

    // HTTP Server
    if let Some(http) = runtime.get("http_server") {
        md.push_str("## HTTP Server\n\n");
        if let Some(desc) = http.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        if let Some(req_obj) = http.get("request_object") {
            md.push_str("### Request Object Properties\n\n");
            if let Some(props) = req_obj.get("properties").and_then(|v| v.as_table()) {
                md.push_str("| Property | Description |\n");
                md.push_str("|----------|-------------|\n");
                for (name, desc) in props {
                    if let Some(d) = desc.as_str() {
                        md.push_str(&format!("| `req.{}` | {} |\n", name, d));
                    }
                }
                md.push_str("\n");
            }
        }

        if let Some(defaults) = http.get("defaults").and_then(|v| v.as_table()) {
            md.push_str("### Defaults\n\n");
            for (name, value) in defaults {
                if let Some(v) = value.as_str() {
                    md.push_str(&format!("- **{}:** {}\n", name, v));
                }
            }
            md.push_str("\n");
        }
        md.push_str("---\n\n");
    }

    // File-Based Routing
    if let Some(fbr) = runtime.get("file_based_routing") {
        md.push_str("## File-Based Routing\n\n");
        if let Some(desc) = fbr.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        if let Some(conv) = fbr.get("conventions").and_then(|v| v.as_table()) {
            md.push_str("### Conventions\n\n");
            md.push_str("| Convention | Description |\n");
            md.push_str("|------------|-------------|\n");
            for (name, value) in conv {
                if let Some(v) = value.as_str() {
                    md.push_str(&format!("| `{}` | {} |\n", name, v));
                }
            }
            md.push_str("\n");
        }

        if let Some(handlers) = fbr.get("handler_functions") {
            md.push_str("### Handler Functions\n\n");
            if let Some(desc) = handlers.get("description").and_then(|v| v.as_str()) {
                md.push_str(&format!("{}\n\n", desc));
            }
            if let Some(methods) = handlers.get("methods").and_then(|v| v.as_array()) {
                let method_list: Vec<_> = methods
                    .iter()
                    .filter_map(|m| m.as_str())
                    .map(|m| format!("`{}`", m))
                    .collect();
                md.push_str(&format!(
                    "**Supported methods:** {}\n\n",
                    method_list.join(", ")
                ));
            }
        }

        if let Some(mw) = fbr.get("middleware") {
            md.push_str("### Middleware\n\n");
            if let Some(desc) = mw.get("description").and_then(|v| v.as_str()) {
                md.push_str(&format!("{}\n\n", desc));
            }
            if let Some(naming) = mw.get("naming").and_then(|v| v.as_str()) {
                md.push_str(&format!("- **Naming:** {}\n", naming));
            }
            if let Some(func) = mw.get("function").and_then(|v| v.as_str()) {
                md.push_str(&format!("- **Function:** {}\n", func));
            }
            md.push_str("\n");
        }
        md.push_str("---\n\n");
    }

    // Project Structure
    if let Some(ps) = runtime.get("project_structure") {
        md.push_str("## Project Structure\n\n");
        if let Some(desc) = ps.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        if let Some(layout) = ps.get("layout") {
            if let Some(example) = layout.get("example").and_then(|v| v.as_str()) {
                md.push_str("### Recommended Layout\n\n```\n");
                md.push_str(example.trim());
                md.push_str("\n```\n\n");
            }
        }

        if let Some(intent) = ps.get("intent_files") {
            md.push_str("### Intent Files\n\n");
            if let Some(desc) = intent.get("description").and_then(|v| v.as_str()) {
                md.push_str(&format!("{}\n\n", desc));
            }
            if let Some(conv) = intent.get("convention").and_then(|v| v.as_str()) {
                md.push_str(&format!("- **Convention:** `{}`\n", conv));
            }
            if let Some(rec) = intent.get("recommendation").and_then(|v| v.as_str()) {
                md.push_str(&format!("- **Recommendation:** {}\n", rec));
            }
            md.push_str("\n");
        }
        md.push_str("---\n\n");
    }

    // CLI Commands
    if let Some(cli) = runtime.get("cli") {
        md.push_str("## CLI Commands\n\n");
        if let Some(desc) = cli.get("description").and_then(|v| v.as_str()) {
            md.push_str(&format!("{}\n\n", desc));
        }

        // List of command keys in order
        let commands = [
            ("run", "Run"),
            ("lint", "Lint"),
            ("validate", "Validate"),
            ("inspect", "Inspect"),
            ("test", "Test"),
            ("docs", "Docs"),
            ("completions", "Completions"),
            ("intent_check", "Intent Check"),
            ("intent_coverage", "Intent Coverage"),
            ("intent_init", "Intent Init"),
            ("intent_studio", "Intent Studio"),
        ];

        for (key, title) in &commands {
            if let Some(cmd) = cli.get(*key) {
                // Skip internal commands
                if cmd
                    .get("internal")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    continue;
                }

                md.push_str(&format!("### {}\n\n", title));

                if let Some(command) = cmd.get("command").and_then(|v| v.as_str()) {
                    md.push_str(&format!("```\n{}\n```\n\n", command));
                }

                if let Some(desc) = cmd.get("description").and_then(|v| v.as_str()) {
                    md.push_str(&format!("{}\n\n", desc));
                }

                if let Some(options) = cmd.get("options").and_then(|v| v.as_array()) {
                    if !options.is_empty() {
                        md.push_str("**Options:**\n\n");
                        md.push_str("| Option | Type | Default | Description |\n");
                        md.push_str("|--------|------|---------|-------------|\n");
                        for opt in options {
                            let name = opt.get("name").and_then(|v| v.as_str()).unwrap_or("-");
                            let short = opt.get("short").and_then(|v| v.as_str()).unwrap_or("");
                            let typ = opt.get("type").and_then(|v| v.as_str()).unwrap_or("-");
                            let default =
                                opt.get("default").and_then(|v| v.as_str()).unwrap_or("-");
                            let desc = opt
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("-");
                            let opt_str = if short.is_empty() {
                                format!("`{}`", name)
                            } else {
                                format!("`{}`, `{}`", name, short)
                            };
                            md.push_str(&format!(
                                "| {} | {} | {} | {} |\n",
                                opt_str, typ, default, desc
                            ));
                        }
                        md.push_str("\n");
                    }
                }

                // Handle single example
                if let Some(example) = cmd.get("example").and_then(|v| v.as_str()) {
                    md.push_str(&format!("**Example:**\n```bash\n{}\n```\n\n", example));
                }

                // Handle multiple examples
                if let Some(examples) = cmd.get("examples").and_then(|v| v.as_array()) {
                    md.push_str("**Examples:**\n```bash\n");
                    for ex in examples {
                        if let Some(e) = ex.as_str() {
                            md.push_str(&format!("{}\n", e));
                        }
                    }
                    md.push_str("```\n\n");
                }
            }
        }
    }

    // Write to file
    let output_path = docs_dir.join("RUNTIME_REFERENCE.md");
    fs::write(&output_path, &md)?;

    println!(
        "{} Generated {}",
        "✓".green(),
        output_path.display().to_string().cyan()
    );

    Ok(())
}

/// Validate documentation coverage using embedded JSON data
fn validate_docs_from_json() -> anyhow::Result<()> {
    let all_docs = get_docs();

    // Count by category
    let mut builtin_count = 0usize;
    let mut by_module: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    for entry in all_docs {
        match &entry.module {
            Some(m) => *by_module.entry(m.clone()).or_default() += 1,
            None => builtin_count += 1,
        }
    }

    let total = all_docs.len();

    println!("{}", "Documentation Validation".green().bold());
    println!();
    println!(
        "  {} {} global builtins documented",
        "✓".green(),
        builtin_count.to_string().cyan()
    );
    for (module, count) in &by_module {
        println!(
            "  {} {} — {} functions",
            "✓".green(),
            module.cyan(),
            count.to_string().cyan()
        );
    }
    println!();
    println!(
        "  Total: {} functions across {} modules + builtins",
        total.to_string().cyan(),
        by_module.len().to_string().cyan()
    );
    println!();

    // Quality checks
    let missing_sig = all_docs.iter().filter(|d| d.signature.is_none()).count();
    let missing_examples = all_docs.iter().filter(|d| d.examples.is_empty()).count();
    let missing_params = all_docs
        .iter()
        .filter(|d| {
            d.params.is_empty()
                && d.signature
                    .as_ref()
                    .map(|s| {
                        if let Some(start) = s.find('(') {
                            if let Some(end) = s.find(')') {
                                return !s[start + 1..end].trim().is_empty();
                            }
                        }
                        false
                    })
                    .unwrap_or(false)
        })
        .count();

    if missing_sig > 0 {
        println!(
            "  {} {} functions missing @signature",
            "⚠".yellow(),
            missing_sig.to_string().yellow()
        );
    }
    if missing_examples > 0 {
        println!(
            "  {} {} functions missing @example",
            "⚠".yellow(),
            missing_examples.to_string().yellow()
        );
    }
    if missing_params > 0 {
        println!(
            "  {} {} functions missing @param",
            "⚠".yellow(),
            missing_params.to_string().yellow()
        );
    }

    // Check docs/learn/ files
    println!();
    let learn_rules = std::path::Path::new("docs/learn/critical-rules.md");
    if learn_rules.exists() {
        println!("  {} docs/learn/critical-rules.md exists", "✓".green());
    } else {
        println!(
            "  {} docs/learn/critical-rules.md missing (needed by ntnt learn)",
            "⚠".yellow()
        );
    }

    println!();
    println!(
        "{}",
        "  Note: Coverage enforcement happens at compile time via build.rs.".dimmed()
    );
    println!(
        "{}",
        "  All NativeFunction inserts in annotated files must have @ntnt blocks.".dimmed()
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// ntnt learn — agent config generation
// ---------------------------------------------------------------------------

const CRITICAL_RULES: &str = include_str!("../docs/learn/critical-rules.md");
const FULL_GUIDE: &str = include_str!("../docs/AI_AGENT_GUIDE.md");
const LEARN_VERSION_PREFIX: &str = "# Generated by ntnt v";

/// All files that `ntnt learn` can generate, grouped by platform.
const LEARN_PLATFORMS: &[(&str, &[&str])] = &[
    (
        "claude-code",
        &[".claude/CLAUDE.md", ".claude/rules/ntnt.md"],
    ),
    ("cursor", &[".cursorrules"]),
    ("codex", &["AGENTS.md"]),
    ("copilot", &[".github/copilot-instructions.md"]),
];

fn extract_learn_version(content: &str) -> Option<&str> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix(LEARN_VERSION_PREFIX) {
            return rest.split_whitespace().next();
        }
    }
    None
}

fn learn_version_header(platform: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    format!(
        "# Generated by ntnt v{version} — do not edit manually\n\
         # Regenerate: ntnt learn {platform}\n\
         # Check for updates: ntnt learn --check\n"
    )
}

fn run_learn_command(platform: Option<String>, check: bool, update: bool) -> anyhow::Result<()> {
    if check {
        return learn_check();
    }
    if update {
        return learn_update();
    }
    match platform.as_deref() {
        None => learn_stdout(),
        Some("claude-code") => learn_claude_code(),
        Some("cursor") => learn_cursor(),
        Some("codex") => learn_codex(),
        Some("copilot") => learn_copilot(),
        Some(other) => {
            eprintln!(
                "{} Unknown platform: {}",
                "error:".red().bold(),
                other.yellow()
            );
            eprintln!();
            eprintln!("Supported platforms:");
            eprintln!("  claude-code  — .claude/CLAUDE.md + .claude/rules/ntnt.md");
            eprintln!("  cursor       — .cursorrules");
            eprintln!("  codex        — AGENTS.md");
            eprintln!("  copilot      — .github/copilot-instructions.md");
            eprintln!();
            eprintln!(
                "Or run {} to print rules to stdout (works with any agent).",
                "ntnt learn".cyan()
            );
            std::process::exit(1);
        }
    }
}

fn learn_stdout() -> anyhow::Result<()> {
    print!("{}", CRITICAL_RULES);
    Ok(())
}

fn write_learn_file(path: &str, content: &str) -> anyhow::Result<bool> {
    let p = std::path::Path::new(path);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Check if file already has identical content
    if p.exists() {
        let existing = std::fs::read_to_string(p)?;
        if existing == content {
            return Ok(false); // no change
        }
    }
    std::fs::write(p, content)?;
    Ok(true)
}

fn learn_claude_code() -> anyhow::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    println!(
        "\n{}",
        format!("Setting up ntnt for Claude Code (v{version})...")
            .cyan()
            .bold()
    );
    println!();

    // 1. .claude/CLAUDE.md — critical rules + pointer to full reference
    let claude_md = format!(
        "{}\n\
         {}\n\
         \n\
         For the full ntnt language reference (all stdlib functions, patterns, and examples),\n\
         see `.claude/rules/ntnt.md` which is automatically loaded when editing `.tnt` files.\n",
        learn_version_header("claude-code"),
        CRITICAL_RULES,
    );
    let wrote = write_learn_file(".claude/CLAUDE.md", &claude_md)?;
    if wrote {
        println!(
            "  {} Created {} (critical syntax rules)",
            "✓".green(),
            ".claude/CLAUDE.md".cyan()
        );
    } else {
        println!(
            "  {} {} (already up to date)",
            "✓".green(),
            ".claude/CLAUDE.md".dimmed()
        );
    }

    // 2. .claude/rules/ntnt.md — full guide
    let rules_md = format!("{}\n{}", learn_version_header("claude-code"), FULL_GUIDE,);
    let wrote = write_learn_file(".claude/rules/ntnt.md", &rules_md)?;
    if wrote {
        println!(
            "  {} Created {} (full language reference)",
            "✓".green(),
            ".claude/rules/ntnt.md".cyan()
        );
    } else {
        println!(
            "  {} {} (already up to date)",
            "✓".green(),
            ".claude/rules/ntnt.md".dimmed()
        );
    }

    println!();
    println!("Your agent will automatically load these rules when editing .tnt files.");
    println!(
        "Run {} after updating ntnt to verify rules are current.",
        "ntnt learn --check".cyan()
    );
    println!();
    Ok(())
}

fn learn_cursor() -> anyhow::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    println!(
        "\n{}",
        format!("Setting up ntnt for Cursor (v{version})...")
            .cyan()
            .bold()
    );
    println!();

    let content = format!("{}\n{}\n", learn_version_header("cursor"), CRITICAL_RULES,);
    let wrote = write_learn_file(".cursorrules", &content)?;
    if wrote {
        println!(
            "  {} Created {} (critical syntax rules)",
            "✓".green(),
            ".cursorrules".cyan()
        );
    } else {
        println!(
            "  {} {} (already up to date)",
            "✓".green(),
            ".cursorrules".dimmed()
        );
    }

    println!();
    println!("Cursor will load these rules for all files in this project.");
    println!(
        "Run {} after updating ntnt to verify rules are current.",
        "ntnt learn --check".cyan()
    );
    println!();
    Ok(())
}

fn learn_codex() -> anyhow::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    println!(
        "\n{}",
        format!("Setting up ntnt for Codex (v{version})...")
            .cyan()
            .bold()
    );
    println!();

    let content = format!("{}\n{}\n", learn_version_header("codex"), CRITICAL_RULES,);
    let wrote = write_learn_file("AGENTS.md", &content)?;
    if wrote {
        println!(
            "  {} Created {} (critical syntax rules)",
            "✓".green(),
            "AGENTS.md".cyan()
        );
    } else {
        println!(
            "  {} {} (already up to date)",
            "✓".green(),
            "AGENTS.md".dimmed()
        );
    }

    println!();
    println!("Codex will load AGENTS.md as project instructions.");
    println!(
        "Run {} after updating ntnt to verify rules are current.",
        "ntnt learn --check".cyan()
    );
    println!();
    Ok(())
}

fn learn_copilot() -> anyhow::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    println!(
        "\n{}",
        format!("Setting up ntnt for GitHub Copilot (v{version})...")
            .cyan()
            .bold()
    );
    println!();

    let path = ".github/copilot-instructions.md";
    let p = std::path::Path::new(path);

    let content = if p.exists() {
        // Check if it already has ntnt learn content
        let existing = std::fs::read_to_string(p)?;
        if extract_learn_version(&existing).is_some() {
            // Replace the entire file (it was generated by us)
            format!("{}\n{}\n", learn_version_header("copilot"), CRITICAL_RULES,)
        } else {
            // Append ntnt section to existing file
            let separator = if existing.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            format!(
                "{}{}{}\n{}\n",
                existing,
                separator,
                learn_version_header("copilot"),
                CRITICAL_RULES,
            )
        }
    } else {
        format!("{}\n{}\n", learn_version_header("copilot"), CRITICAL_RULES,)
    };

    let wrote = write_learn_file(path, &content)?;
    if wrote {
        println!(
            "  {} Created {} (critical syntax rules)",
            "✓".green(),
            path.cyan()
        );
    } else {
        println!("  {} {} (already up to date)", "✓".green(), path.dimmed());
    }

    println!();
    println!("Copilot will load these instructions for this repository.");
    println!(
        "Run {} after updating ntnt to verify rules are current.",
        "ntnt learn --check".cyan()
    );
    println!();
    Ok(())
}

fn learn_check() -> anyhow::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    println!(
        "\n{}",
        format!("Checking ntnt learn files (v{version})...")
            .cyan()
            .bold()
    );
    println!();

    let mut found = 0u32;
    let mut stale = 0u32;

    for (_platform, files) in LEARN_PLATFORMS {
        for file in *files {
            let p = std::path::Path::new(file);
            if !p.exists() {
                continue;
            }
            found += 1;
            let content = std::fs::read_to_string(p)?;
            match extract_learn_version(&content) {
                Some(v) if v == version => {
                    println!("  {} {} — up to date (v{})", "✓".green(), file.cyan(), v);
                }
                Some(v) => {
                    stale += 1;
                    println!(
                        "  {} {} — stale (v{} -> v{})",
                        "⚠".yellow(),
                        file.yellow(),
                        v,
                        version
                    );
                }
                None => {
                    // File exists but wasn't generated by ntnt learn
                    println!(
                        "  {} {} — not managed by ntnt learn",
                        "-".dimmed(),
                        file.dimmed()
                    );
                }
            }
        }
    }

    println!();
    if found == 0 {
        println!(
            "No ntnt learn files found. Run {} to generate them.",
            "ntnt learn <platform>".cyan()
        );
    } else if stale > 0 {
        println!(
            "Run {} to refresh stale files.",
            "ntnt learn --update".cyan()
        );
    } else {
        println!("{}", "All files are up to date.".green());
    }
    println!();

    if stale > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn learn_update() -> anyhow::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    println!(
        "\n{}",
        format!("Updating ntnt learn files to v{version}...")
            .cyan()
            .bold()
    );
    println!();

    let mut found = false;

    for (platform, files) in LEARN_PLATFORMS {
        let has_managed = files.iter().any(|f| {
            let p = std::path::Path::new(f);
            if !p.exists() {
                return false;
            }
            let content = std::fs::read_to_string(p).unwrap_or_default();
            extract_learn_version(&content).is_some()
        });
        if has_managed {
            found = true;
            let result = match *platform {
                "claude-code" => learn_claude_code(),
                "cursor" => learn_cursor(),
                "codex" => learn_codex(),
                "copilot" => learn_copilot(),
                _ => Ok(()),
            };
            if let Err(e) = result {
                eprintln!("  {} Failed to update {}: {}", "✗".red(), platform, e);
            }
        }
    }

    if !found {
        println!(
            "No ntnt learn files found to update. Run {} first.",
            "ntnt learn <platform>".cyan()
        );
    }
    println!();
    Ok(())
}

// ---------------------------------------------------------------------------
// Agent file synchronization
// ---------------------------------------------------------------------------

const AGENT_BEGIN_MARKER: &str =
    "<!-- BEGIN NTNT CODING GUIDE (sourced from docs/AI_AGENT_GUIDE.md) -->";
const AGENT_END_MARKER: &str = "<!-- END NTNT CODING GUIDE -->";
const AGENT_SYNC_PREFIX: &str = "<!-- Last synced: ";

/// Agent files to sync: (relative path from project root, link prefix for doc references)
const AGENT_FILES: &[(&str, &str)] = &[
    ("CLAUDE.md", "docs/"),
    (".github/copilot-instructions.md", "../docs/"),
];

/// Doc filenames that need link rewriting per agent file location
const DOC_LINK_FILES: &[&str] = &[
    "STDLIB_REFERENCE.md",
    "SYNTAX_REFERENCE.md",
    "IAL_REFERENCE.md",
    "RUNTIME_REFERENCE.md",
    "AI_AGENT_GUIDE.md",
];

/// Sync agent instruction files (CLAUDE.md, copilot-instructions.md, etc.)
/// by injecting the coding guide content from AI_AGENT_GUIDE.md.
fn sync_agent_files(docs_dir: &std::path::Path) -> anyhow::Result<()> {
    let guide_path = docs_dir.join("AI_AGENT_GUIDE.md");
    if !guide_path.exists() {
        println!(
            "  {} AI_AGENT_GUIDE.md not found, skipping agent sync",
            "!".yellow()
        );
        return Ok(());
    }

    let guide_content = fs::read_to_string(&guide_path)?;
    let guide_body = match extract_guide_body(&guide_content) {
        Some(body) => body,
        None => {
            println!(
                "  {} AI_AGENT_GUIDE.md has no --- separator, skipping agent sync",
                "!".yellow()
            );
            return Ok(());
        }
    };

    let project_root = docs_dir.parent().unwrap_or(std::path::Path::new("."));
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    for (rel_path, link_prefix) in AGENT_FILES {
        let file_path = project_root.join(rel_path);
        if !file_path.exists() {
            println!(
                "  {} {} not found, skipping",
                "!".yellow(),
                rel_path.dimmed()
            );
            continue;
        }

        match update_agent_file(&file_path, &guide_body, link_prefix, &today) {
            Ok(true) => {
                println!("{} Synced {}", "  ✓".green(), rel_path);
            }
            Ok(false) => {
                println!("  {} already up to date", rel_path.dimmed());
            }
            Err(e) => {
                println!("  {} Failed to sync {}: {}", "✗".red(), rel_path, e);
            }
        }
    }

    Ok(())
}

/// Extract the coding guide body from AI_AGENT_GUIDE.md.
/// Returns everything after the first `---` line (the header separator).
fn extract_guide_body(content: &str) -> Option<String> {
    let mut lines = content.lines();
    // Find the first --- separator
    let mut found = false;
    for line in &mut lines {
        if line.trim() == "---" {
            found = true;
            break;
        }
    }
    if !found {
        return None;
    }
    // Collect everything after the separator
    let body: Vec<&str> = lines.collect();
    Some(body.join("\n"))
}

/// Update a single agent file by replacing content between BEGIN/END markers.
/// Returns Ok(true) if the file was written, Ok(false) if already up to date.
fn update_agent_file(
    path: &std::path::Path,
    guide_body: &str,
    link_prefix: &str,
    today: &str,
) -> anyhow::Result<bool> {
    let content = fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();

    // Find marker positions
    let begin_idx = lines.iter().position(|l| l.trim() == AGENT_BEGIN_MARKER);
    let end_idx = lines.iter().position(|l| l.trim() == AGENT_END_MARKER);

    let (begin, end) = match (begin_idx, end_idx) {
        (Some(b), Some(e)) if b < e => (b, e),
        _ => {
            return Err(anyhow::anyhow!(
                "Missing or malformed BEGIN/END NTNT CODING GUIDE markers"
            ));
        }
    };

    // Rewrite doc links for this file's location
    let rewritten_body = rewrite_doc_links(guide_body, link_prefix);

    // Build new file: preamble + marker + guide + marker + postamble
    let mut output = Vec::new();

    // Preamble (lines before BEGIN marker), updating the Last synced date
    for line in &lines[..begin] {
        if line.starts_with(AGENT_SYNC_PREFIX) {
            output.push(format!("{}{} -->", AGENT_SYNC_PREFIX, today));
        } else {
            output.push(line.to_string());
        }
    }

    // BEGIN marker + guide body + END marker
    output.push(AGENT_BEGIN_MARKER.to_string());
    output.push(rewritten_body);
    output.push(AGENT_END_MARKER.to_string());

    // Postamble (lines after END marker)
    if end + 1 < lines.len() {
        for line in &lines[end + 1..] {
            output.push(line.to_string());
        }
    }

    let new_content = output.join("\n") + "\n";

    // Only write if content actually changed
    if new_content == content {
        return Ok(false);
    }

    fs::write(path, &new_content)?;
    Ok(true)
}

/// Rewrite bare doc links like `](STDLIB_REFERENCE.md)` to `](prefix/STDLIB_REFERENCE.md)`.
/// The prefix accounts for the agent file's location relative to the docs/ directory.
fn rewrite_doc_links(content: &str, prefix: &str) -> String {
    let mut result = content.to_string();
    for filename in DOC_LINK_FILES {
        let bare = format!("]({})", filename);
        let prefixed = format!("]({}{})", prefix, filename);
        result = result.replace(&bare, &prefixed);
    }
    result
}
