//! NTNT Language CLI
//!
//! Command-line interface for the NTNT (Intent) programming language.

use clap::{Parser, Subcommand};
use colored::*;
use ntnt::{intent, interpreter::Interpreter, lexer::Lexer, parser::Parser as IntentParser};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ntnt")]
#[command(author = "NTNT Language Team")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "NTNT (Intent) - A programming language for AI-driven development", long_about = None)]
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
    Run {
        /// The source file to run
        #[arg(value_name = "FILE")]
        file: PathBuf,
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
    /// Parse and display the AST
    Parse {
        /// The source file to parse
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Tokenize and display tokens
    Lex {
        /// The source file to tokenize
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
    /// Check source for errors without running
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
}

/// Intent-Driven Development subcommands
#[derive(Subcommand)]
enum IntentCommands {
    /// Check that code matches its intent specification
    ///
    /// Runs all tests defined in the .intent file against the NTNT program.
    /// Looks for <name>.intent file automatically, or specify with --intent.
    ///
    /// Examples:
    ///   ntnt intent check server.tnt
    ///   ntnt intent check server.tnt --intent requirements.intent
    ///   ntnt intent check server.tnt --verbose
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

        /// Show verbose output including response bodies
        #[arg(long = "verbose", short = 'v')]
        verbose: bool,
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

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(Commands::Repl) => run_repl(),
        Some(Commands::Run { file }) => run_file(&file),
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
        Some(Commands::Lint { path, quiet, fix }) => lint_project(&path, quiet, fix),
        Some(Commands::Intent(intent_cmd)) => run_intent_command(intent_cmd),
        None => {
            if let Some(file) = cli.file {
                run_file(&file)
            } else {
                run_repl()
            }
        }
    };

    if let Err(e) = result {
        eprintln!("{}: {}", "Error".red().bold(), e);
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
    println!("  {}    - Show this help message", ":help, :h".cyan());
    println!("  {} - Exit the REPL", ":quit, :q, :exit".cyan());
    println!("  {}   - Clear the environment", ":clear".cyan());
    println!(
        "  {}     - Show current environment bindings",
        ":env".cyan()
    );
    println!();
    println!("{}", "Module System:".yellow().bold());
    println!(
        "  {} - Import specific functions",
        r#"import { split, join } from "std/string""#.cyan()
    );
    println!(
        "  {}    - Import module with alias",
        r#"import "std/math" as math"#.cyan()
    );
    println!();
    println!("{}", "Standard Library:".yellow().bold());
    println!(
        "  {} - std/string: split, join, trim, replace, to_upper, to_lower",
        "String".cyan()
    );
    println!(
        "  {}   - std/math: sin, cos, tan, log, exp, PI, E",
        "Math".cyan()
    );
    println!(
        "  {} - std/collections: push, pop, first, last, reverse, slice",
        "Collections".cyan()
    );
    println!(
        "  {}    - std/env: get_env, args, cwd",
        "Environment".cyan()
    );
    println!(
        "  {}     - std/fs: read_file, write_file, exists, mkdir, remove",
        "Files".cyan()
    );
    println!(
        "  {}     - std/path: join, dirname, basename, extension, resolve",
        "Paths".cyan()
    );
    println!(
        "  {}      - std/json: parse, stringify, stringify_pretty",
        "JSON".cyan()
    );
    println!(
        "  {}      - std/time: now, sleep, elapsed, format_timestamp",
        "Time".cyan()
    );
    println!(
        "  {}    - std/crypto: sha256, hmac_sha256, uuid, random_bytes",
        "Crypto".cyan()
    );
    println!(
        "  {}       - std/url: parse, encode, decode, build_query, join",
        "URL".cyan()
    );
    println!(
        "  {}      - std/http: fetch, post, put, delete, request, get_json",
        "HTTP".cyan()
    );
    println!();
    println!("{}", "Basic Examples:".yellow().bold());
    println!("  {}           - Variable binding", "let x = 42;".cyan());
    println!("  {}    - Arithmetic", "let y = x + 10;".cyan());
    println!(
        "  {} - Function definition",
        "fn add(a, b) { a + b }".cyan()
    );
    println!("  {}       - Function call", "add(1, 2)".cyan());
    println!();
    println!("{}", "Traits:".yellow().bold());
    println!(
        "  {} - Define a trait",
        "trait Display { fn show(self); }".cyan()
    );
    println!(
        "  {} - Implement trait",
        "impl Display for Point { ... }".cyan()
    );
    println!();
    println!("{}", "Loops & Iteration:".yellow().bold());
    println!(
        "  {}  - For-in loop",
        "for x in [1, 2, 3] { print(x); }".cyan()
    );
    println!(
        "  {}    - Range (exclusive)",
        "for i in 0..5 { print(i); }".cyan()
    );
    println!(
        "  {}   - Range (inclusive)",
        "for i in 0..=5 { print(i); }".cyan()
    );
    println!(
        "  {} - Iterate strings",
        r#"for c in "hello" { print(c); }"#.cyan()
    );
    println!();
    println!("{}", "Defer:".yellow().bold());
    println!(
        "  {} - Runs on scope exit",
        "defer print(\"cleanup\");".cyan()
    );
    println!();
    println!("{}", "Maps:".yellow().bold());
    println!(
        "  {} - Map literal",
        r#"let m = map { "a": 1, "b": 2 };"#.cyan()
    );
    println!();
    println!("{}", "String Interpolation:".yellow().bold());
    println!("  {} - Embed expressions", r#""Hello, {name}!""#.cyan());
    println!("  {} - With math", r#""Sum: {a + b}""#.cyan());
    println!();
    println!("{}", "Raw Strings:".yellow().bold());
    println!(
        "  {}   - No escape processing",
        r#"r"C:\path\to\file""#.cyan()
    );
    println!("  {} - With quotes inside", r##"r#"say "hi""#"##.cyan());
    println!();
    println!("{}", "Trait Bounds:".yellow().bold());
    println!(
        "  {} - Bounded generic",
        "fn sort<T: Comparable>(arr: [T])".cyan()
    );
    println!("  {} - Multiple bounds", "fn f<T: A + B>(x: T)".cyan());
    println!();
    println!("{}", "Option & Result Types:".yellow().bold());
    println!("  {}     - Create Some value", "let x = Some(42);".cyan());
    println!("  {}          - Create None", "let y = None;".cyan());
    println!("  {} - Unwrap with default", "unwrap_or(y, 0)".cyan());
    println!("  {}  - Create Ok result", "let r = Ok(100);".cyan());
    println!("  {} - Create Err result", r#"let e = Err("fail");"#.cyan());
    println!();
    println!("{}", "Pattern Matching:".yellow().bold());
    println!("  {}", "match x { Some(v) => v * 2, None => 0 }".cyan());
    println!();
    println!("{}", "Enums & Generics:".yellow().bold());
    println!("  {}", "enum Color { Red, Green, Blue }".cyan());
    println!(
        "  {} - Generic function",
        "fn id<T>(x: T) -> T { x }".cyan()
    );
    println!();
    println!("{}", "Contracts:".yellow().bold());
    println!(
        "  {} - Precondition",
        "fn div(a, b) requires b != 0 { a / b }".cyan()
    );
    println!(
        "  {} - Postcondition",
        "fn abs(x) ensures result >= 0 { ... }".cyan()
    );
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

fn run_file(path: &PathBuf) -> anyhow::Result<()> {
    let source = fs::read_to_string(path)?;
    let mut interpreter = Interpreter::new();

    // Set the current file path for imports and hot-reload
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());
    let path_str = canonical_path.to_string_lossy();
    interpreter.set_current_file(&path_str);
    interpreter.set_main_source_file(&path_str);

    let lexer = Lexer::new(&source);
    let tokens: Vec<_> = lexer.collect();

    let mut parser = IntentParser::new(tokens);
    let ast = parser.parse()?;

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
            match stmt {
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
                if let Statement::Function { name, .. } = stmt {
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
                "route_patterns": "Use raw strings for routes: `get(r\"/users/{id}\", handler)` - regular strings interpret {} as interpolation",
                "string_interpolation": "Use `\"{variable}\"` for interpolation, NOT `${variable}` or backticks",
                "ranges": "Use `0..10` (exclusive) or `0..=10` (inclusive), NOT range()",
                "imports": "Use `import { x } from \"std/module\"` with `/` separator",
                "contracts": "Place requires/ensures AFTER return type, BEFORE body",
                "mutability": "Use `let mut x` for mutable variables"
            },
            "builtin_functions": ["print", "len", "str", "abs", "min", "max", "sqrt", "pow", "round", "floor", "ceil", "Some", "None", "Ok", "Err", "unwrap", "unwrap_or", "is_some", "is_none", "is_ok", "is_err"],
            "common_imports": {
                "std/string": ["split", "join", "trim", "replace", "contains", "starts_with", "ends_with"],
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
                let warnings = analyze_ast_warnings(&ast, &source);
                warning_count += warnings.len();

                results.push(json!({
                    "file": relative_path,
                    "valid": true,
                    "errors": [],
                    "warnings": warnings,
                }));

                // Print success indicator
                if warnings.is_empty() {
                    eprintln!("{} {}", "✓".green(), relative_path);
                } else {
                    eprintln!(
                        "{} {} ({} warnings)",
                        "⚠".yellow(),
                        relative_path,
                        warnings.len()
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
fn lint_project(path: &PathBuf, quiet: bool, show_fixes: bool) -> anyhow::Result<()> {
    use serde_json::{json, Value as JsonValue};

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
                let issues = lint_ast(&ast, &source, &relative_path);

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
            "route_patterns": "Use raw strings `r\"/path/{id}\"` for routes with parameters",
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
    use ntnt::ast::{Expression, Statement, StringPart};
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
                                Expression::String(s) if s.contains('{') && s.contains('}') => {
                                    // Regular string with {} - likely needs raw string
                                    let line = find_line_number(source_lines, s);
                                    issues.push(json!({
                                        "severity": "warning",
                                        "rule": "route_pattern_needs_raw_string",
                                        "message": format!("Route pattern '{}' contains {{}} but is not a raw string. Use r\"{}\" to prevent interpolation.", s, s),
                                        "line": line,
                                        "fix": {
                                            "replacement": format!("r\"{}\"", s),
                                            "description": "Wrap route pattern in raw string"
                                        }
                                    }));
                                }
                                Expression::InterpolatedString(parts) => {
                                    // Interpolated string used as route - definitely wrong
                                    let has_route_params = parts.iter().any(|p| {
                                        if let StringPart::Literal(s) = p {
                                            s.contains("/{") || s.contains("}/")
                                        } else {
                                            false
                                        }
                                    });
                                    if has_route_params {
                                        issues.push(json!({
                                            "severity": "warning",
                                            "rule": "route_pattern_interpolation",
                                            "message": "Route pattern appears to use string interpolation where route parameters were intended. Use raw string r\"/path/{param}\" for route parameters.",
                                            "line": null,
                                            "fix": {
                                                "description": "Convert to raw string with route parameters"
                                            }
                                        }));
                                    }
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
                check_expr_for_issues(body, source_lines, issues, http_route_functions);
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
        match stmt {
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
        } => run_intent_check_command(&file, intent_file.as_ref(), port, verbose),
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
fn run_intent_check_command(
    input_path: &PathBuf,
    explicit_intent_path: Option<&PathBuf>,
    port: u16,
    verbose: bool,
) -> anyhow::Result<()> {
    println!("{}", "=== NTNT Intent Check ===".cyan().bold());
    println!();

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

    println!("Source: {}", ntnt_path.display().to_string().green());
    println!("Intent: {}", intent_file_path.display().to_string().green());
    println!();

    // Run intent check
    match intent::run_intent_check(
        ntnt_path.as_path(),
        intent_file_path.as_path(),
        port,
        verbose,
    ) {
        Ok(result) => {
            intent::print_intent_results(&result);

            if result.features_failed > 0 {
                std::process::exit(1);
            }
            Ok(())
        }
        Err(e) => {
            anyhow::bail!("Intent check failed: {}", e);
        }
    }
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
    let source_content = fs::read_to_string(&ntnt_path)?;
    let source_files = vec![(ntnt_path.to_string_lossy().to_string(), source_content)];

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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::{Child, Command};
    use std::time::SystemTime;

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

    let intent_path_str = intent_path.to_string_lossy().to_string();
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
        // We'll use a special env var that the interpreter checks
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

    // Set up Ctrl+C handler to clean up child process
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, std::sync::atomic::Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    // Try to open browser
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

    // Start simple HTTP server with non-blocking accepts
    let listener = TcpListener::bind(&addr)
        .map_err(|e| anyhow::anyhow!("Failed to bind to {}: {}", addr, e))?;
    listener.set_nonblocking(true)?;

    // Track file modification time for change detection
    let mut last_modified = fs::metadata(&intent_path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    // Main loop
    while running.load(std::sync::atomic::Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                // Set stream back to blocking for read/write
                stream.set_nonblocking(false)?;

                let mut buffer = [0; 4096];
                if stream.read(&mut buffer).is_ok() {
                    let request = String::from_utf8_lossy(&buffer);

                    // Parse request path
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/");

                    let response = if path == "/check-update" {
                        // Endpoint for checking if file has changed
                        let current_modified = fs::metadata(&intent_path)
                            .and_then(|m| m.modified())
                            .unwrap_or(SystemTime::UNIX_EPOCH);

                        let changed = current_modified != last_modified;
                        if changed {
                            last_modified = current_modified;
                        }

                        let body = if changed { "changed" } else { "unchanged" };
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nCache-Control: no-cache\r\n\r\n{}",
                            body.len(),
                            body
                        )
                    } else if path == "/app-status" {
                        // Check if app is responding and healthy
                        let app_url = format!("http://127.0.0.1:{}/", app_port);
                        let status = match reqwest::blocking::Client::builder()
                            .timeout(std::time::Duration::from_secs(2))
                            .build()
                            .and_then(|client| client.get(&app_url).send())
                        {
                            Ok(resp) => {
                                let status_code = resp.status().as_u16();
                                // Consider 404 and 5xx as "error" states (routes not registered or server error)
                                if status_code == 404 {
                                    format!(
                                        r#"{{"running": true, "healthy": false, "status": {}, "error": "No routes registered (404)"}}"#,
                                        status_code
                                    )
                                } else if status_code >= 500 {
                                    format!(
                                        r#"{{"running": true, "healthy": false, "status": {}, "error": "Server error"}}"#,
                                        status_code
                                    )
                                } else {
                                    format!(
                                        r#"{{"running": true, "healthy": true, "status": {}}}"#,
                                        status_code
                                    )
                                }
                            }
                            Err(e) => {
                                let error_msg = e.to_string().replace('"', "\\\"");
                                format!(
                                    r#"{{"running": false, "healthy": false, "error": "{}"}}"#,
                                    error_msg
                                )
                            }
                        };
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-cache\r\n\r\n{}",
                            status.len(),
                            status
                        )
                    } else if path == "/run-tests" {
                        // Run tests against the app server
                        match intent::IntentFile::parse(&intent_path) {
                            Ok(intent_file) => {
                                // Read source content for annotation checking
                                let source_content =
                                    tnt_path.as_ref().and_then(|p| fs::read_to_string(p).ok());
                                let results = intent::run_tests_against_server(
                                    &intent_file,
                                    app_port,
                                    source_content.as_deref(),
                                );
                                let json = serde_json::to_string(&results)
                                    .unwrap_or_else(|_| "{}".to_string());
                                format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-cache\r\n\r\n{}",
                                    json.len(),
                                    json
                                )
                            }
                            Err(e) => {
                                let error = format!(
                                    r#"{{"error": "{}"}}"#,
                                    e.to_string().replace('"', "\\\"")
                                );
                                format!(
                                    "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                                    error.len(),
                                    error
                                )
                            }
                        }
                    } else {
                        // Main page - render the intent file
                        match intent::IntentFile::parse(&intent_path) {
                            Ok(intent_file) => {
                                let html = render_intent_studio_html(
                                    &intent_file,
                                    &intent_path_str,
                                    app_port,
                                );
                                format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-cache\r\n\r\n{}",
                                    html.len(),
                                    html
                                )
                            }
                            Err(e) => {
                                let html =
                                    render_intent_studio_error(&e.to_string(), &intent_path_str);
                                format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
                                    html.len(),
                                    html
                                )
                            }
                        }
                    };

                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // No connection pending, sleep briefly and check shutdown
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                eprintln!("Connection error: {}", e);
            }
        }
    }

    // Clean up: kill the app process if we started it
    if let Some(mut child) = app_process {
        println!("\n  {} Stopping app server...", "🛑".red());
        let _ = child.kill();
        let _ = child.wait();
    }

    println!("  {} Intent Studio stopped", "👋".dimmed());

    Ok(())
}

/// Render the Intent Studio HTML page
fn render_intent_studio_html(
    intent_file: &intent::IntentFile,
    file_path: &str,
    app_port: u16,
) -> String {
    let mut features_html = String::new();

    for feature in &intent_file.features {
        let id = feature.id.as_deref().unwrap_or("no-id");
        let description = feature.description.as_deref().unwrap_or("No description");

        // Build test cases HTML
        let mut tests_html = String::new();
        for (test_idx, test) in feature.tests.iter().enumerate() {
            let mut assertions_html = String::new();
            for (assert_idx, assertion) in test.assertions.iter().enumerate() {
                let assertion_str = match assertion {
                    intent::Assertion::Status(code) => format!("status: {}", code),
                    intent::Assertion::BodyContains(text) => format!("body contains \"{}\"", text),
                    intent::Assertion::BodyNotContains(text) => {
                        format!("body not contains \"{}\"", text)
                    }
                    intent::Assertion::BodyMatches(pattern) => {
                        format!("body matches \"{}\"", pattern)
                    }
                    intent::Assertion::HeaderContains(name, value) => {
                        format!("header \"{}\" contains \"{}\"", name, value)
                    }
                };
                assertions_html.push_str(&format!(
                    r#"<div class="assertion" data-feature="{}" data-test="{}" data-assert="{}"><span class="assertion-icon">○</span> {}</div>"#,
                    html_escape(id),
                    test_idx,
                    assert_idx,
                    html_escape(&assertion_str)
                ));
            }

            let body_html = if let Some(body) = &test.body {
                format!(
                    r#"<div class="test-body">Body: <code>{}</code></div>"#,
                    html_escape(body)
                )
            } else {
                String::new()
            };

            tests_html.push_str(&format!(
                r#"<div class="test-case" data-feature="{}" data-test="{}">
                    <div class="test-request"><span class="method">{}</span> <span class="path">{}</span></div>
                    {}
                    <div class="assertions">{}</div>
                </div>"#,
                html_escape(id),
                test_idx,
                test.method,
                html_escape(&test.path),
                body_html,
                assertions_html
            ));
        }

        // Pick an icon based on the feature name
        let icon = if feature.name.to_lowercase().contains("login")
            || feature.name.to_lowercase().contains("auth")
        {
            "🔐"
        } else if feature.name.to_lowercase().contains("register")
            || feature.name.to_lowercase().contains("signup")
        {
            "📝"
        } else if feature.name.to_lowercase().contains("home")
            || feature.name.to_lowercase().contains("index")
        {
            "🏠"
        } else if feature.name.to_lowercase().contains("api")
            || feature.name.to_lowercase().contains("status")
        {
            "⚡"
        } else if feature.name.to_lowercase().contains("about") {
            "ℹ️"
        } else if feature.name.to_lowercase().contains("user")
            || feature.name.to_lowercase().contains("profile")
        {
            "👤"
        } else if feature.name.to_lowercase().contains("search") {
            "🔍"
        } else if feature.name.to_lowercase().contains("setting")
            || feature.name.to_lowercase().contains("config")
        {
            "⚙️"
        } else if feature.name.to_lowercase().contains("chart")
            || feature.name.to_lowercase().contains("graph")
        {
            "📊"
        } else if feature.name.to_lowercase().contains("data")
            || feature.name.to_lowercase().contains("database")
        {
            "💾"
        } else {
            "✨"
        };

        features_html.push_str(&format!(
            r#"<div class="feature-card" data-feature="{}">
                <div class="feature-header">
                    <span class="feature-icon">{}</span>
                    <span class="feature-name">{}</span>
                    <span class="feature-badge feature-status" data-feature="{}">pending</span>
                </div>
                <div class="feature-id"><code>{}</code></div>
                <div class="feature-description">{}</div>
                <div class="feature-tests">
                    <div class="tests-header">Acceptance Criteria</div>
                    {}
                </div>
            </div>"#,
            html_escape(id),
            icon,
            html_escape(&feature.name),
            html_escape(id),
            html_escape(id),
            html_escape(description),
            tests_html
        ));
    }

    let feature_count = intent_file.features.len();
    let test_count: usize = intent_file.features.iter().map(|f| f.tests.len()).sum();
    let assertion_count: usize = intent_file
        .features
        .iter()
        .flat_map(|f| f.tests.iter())
        .map(|t| t.assertions.len())
        .sum();

    let run_button_html =
        r#"<button class="run-tests-btn" onclick="runTests()">▶ Run Tests</button>"#;

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Intent Studio - {file_path}</title>
    <style>
        :root {{
            --bg-primary: #0d1117;
            --bg-secondary: #161b22;
            --bg-tertiary: #21262d;
            --border-color: #30363d;
            --text-primary: #e6edf3;
            --text-secondary: #8b949e;
            --text-muted: #6e7681;
            --accent-blue: #58a6ff;
            --accent-green: #3fb950;
            --accent-purple: #a371f7;
            --accent-orange: #d29922;
            --accent-red: #f85149;
        }}
        
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}
        
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'Noto Sans', Helvetica, Arial, sans-serif;
            background: var(--bg-primary);
            color: var(--text-primary);
            line-height: 1.6;
            min-height: 100vh;
        }}
        
        .header {{
            background: var(--bg-secondary);
            border-bottom: 1px solid var(--border-color);
            padding: 1rem 2rem;
            position: sticky;
            top: 0;
            z-index: 100;
        }}
        
        .header-content {{
            max-width: 1200px;
            margin: 0 auto;
            display: flex;
            justify-content: space-between;
            align-items: center;
            gap: 1rem;
            flex-wrap: wrap;
        }}
        
        .logo {{
            display: flex;
            align-items: center;
            gap: 0.75rem;
        }}
        
        .logo-icon {{
            width: 28px;
            height: 28px;
            display: flex;
            align-items: center;
            justify-content: center;
        }}
        
        .logo-icon img {{
            width: 100%;
            height: 100%;
            display: block;
        }}
        
        .logo-text {{
            font-size: 1.25rem;
            font-weight: 600;
            color: var(--text-primary);
        }}
        
        .logo-text span {{
            color: var(--accent-purple);
        }}
        
        .header-file {{
            color: var(--text-secondary);
            font-size: 0.875rem;
        }}
        
        .header-file code {{
            background: var(--bg-tertiary);
            padding: 0.25rem 0.5rem;
            border-radius: 4px;
            font-size: 0.8rem;
        }}
        
        .header-controls {{
            display: flex;
            align-items: center;
            gap: 0.75rem;
        }}
        
        .run-tests-btn {{
            background: var(--accent-green);
            color: var(--bg-primary);
            border: none;
            padding: 0.5rem 1rem;
            border-radius: 6px;
            font-weight: 600;
            font-size: 0.875rem;
            cursor: pointer;
            transition: background 0.2s, transform 0.1s;
        }}
        
        .run-tests-btn:hover:not(:disabled) {{
            background: #2ea043;
            transform: scale(1.02);
        }}
        
        .run-tests-btn:active:not(:disabled) {{
            transform: scale(0.98);
        }}
        
        .run-tests-btn.disabled {{
            background: var(--bg-tertiary);
            color: var(--text-muted);
            cursor: not-allowed;
        }}
        
        .run-tests-btn.running {{
            background: var(--accent-orange);
            cursor: wait;
        }}
        
        .open-app-btn {{
            background: var(--bg-tertiary);
            color: var(--text-primary);
            border: 1px solid var(--border-color);
            padding: 0.5rem 1rem;
            border-radius: 6px;
            font-weight: 500;
            font-size: 0.875rem;
            cursor: pointer;
            text-decoration: none;
            transition: background 0.2s, border-color 0.2s;
            display: flex;
            align-items: center;
            gap: 0.375rem;
        }}
        
        .open-app-btn:hover {{
            background: var(--bg-secondary);
            border-color: var(--accent-blue);
            color: var(--accent-blue);
        }}
        
        .status-badge {{
            display: flex;
            align-items: center;
            gap: 0.5rem;
            background: var(--bg-tertiary);
            padding: 0.375rem 0.75rem;
            border-radius: 20px;
            font-size: 0.8rem;
        }}
        
        .status-dot {{
            width: 8px;
            height: 8px;
            background: var(--accent-green);
            border-radius: 50%;
            animation: pulse 2s infinite;
        }}
        
        .app-status {{
            display: flex;
            align-items: center;
            gap: 0.5rem;
            background: var(--bg-tertiary);
            padding: 0.375rem 0.75rem;
            border-radius: 20px;
            font-size: 0.8rem;
        }}
        
        .app-status-dot {{
            width: 8px;
            height: 8px;
            border-radius: 50%;
            background: var(--text-muted);
        }}
        
        .app-status.running .app-status-dot {{
            background: var(--accent-green);
            animation: pulse 2s infinite;
        }}
        
        .app-status.error .app-status-dot {{
            background: var(--accent-red);
        }}
        
        .app-status.starting .app-status-dot {{
            background: var(--accent-orange);
            animation: pulse 1s infinite;
        }}
        
        @keyframes pulse {{
            0%, 100% {{ opacity: 1; }}
            50% {{ opacity: 0.5; }}
        }}
        
        .main {{
            max-width: 1200px;
            margin: 0 auto;
            padding: 2rem;
        }}
        
        .stats {{
            display: flex;
            gap: 1rem;
            margin-bottom: 2rem;
            flex-wrap: wrap;
        }}
        
        .stat {{
            background: var(--bg-secondary);
            border: 1px solid var(--border-color);
            border-radius: 8px;
            padding: 1rem 1.5rem;
            min-width: 120px;
        }}
        
        .stat-value {{
            font-size: 2rem;
            font-weight: 600;
            color: var(--accent-blue);
        }}
        
        .stat-label {{
            color: var(--text-secondary);
            font-size: 0.875rem;
        }}
        
        .stat.passing .stat-value {{
            color: var(--accent-green);
        }}
        
        .stat.failing .stat-value {{
            color: var(--accent-red);
        }}
        
        .features-grid {{
            display: grid;
            gap: 1.5rem;
        }}
        
        .feature-card {{
            background: var(--bg-secondary);
            border: 1px solid var(--border-color);
            border-radius: 12px;
            padding: 1.5rem;
            transition: border-color 0.2s, transform 0.2s;
        }}
        
        .feature-card:hover {{
            border-color: var(--accent-purple);
            transform: translateY(-2px);
        }}
        
        .feature-card.passed {{
            border-color: var(--accent-green);
        }}
        
        .feature-card.failed {{
            border-color: var(--accent-red);
        }}
        
        .feature-header {{
            display: flex;
            align-items: center;
            gap: 0.75rem;
            margin-bottom: 0.5rem;
        }}
        
        .feature-icon {{
            font-size: 1.5rem;
        }}
        
        .feature-name {{
            font-size: 1.25rem;
            font-weight: 600;
            flex-grow: 1;
        }}
        
        .feature-badge {{
            background: var(--accent-purple);
            color: white;
            font-size: 0.7rem;
            padding: 0.2rem 0.5rem;
            border-radius: 4px;
            text-transform: uppercase;
            font-weight: 600;
        }}
        
        .feature-status.passed {{
            background: var(--accent-green);
        }}
        
        .feature-status.failed {{
            background: var(--accent-red);
        }}
        
        .unlinked-warning {{
            background: rgba(251, 191, 36, 0.2);
            color: #fbbf24;
            font-size: 0.7rem;
            padding: 0.2rem 0.5rem;
            border-radius: 4px;
            font-weight: 500;
            margin-left: 0.5rem;
        }}
        
        .feature-card.unlinked {{
            border-left: 3px solid #fbbf24;
        }}
        
        .feature-id {{
            margin-bottom: 0.75rem;
        }}
        
        .feature-id code {{
            background: var(--bg-tertiary);
            padding: 0.2rem 0.5rem;
            border-radius: 4px;
            font-size: 0.8rem;
            color: var(--text-secondary);
        }}
        
        .feature-description {{
            color: var(--text-secondary);
            margin-bottom: 1rem;
            font-size: 0.95rem;
        }}
        
        .feature-tests {{
            background: var(--bg-tertiary);
            border-radius: 8px;
            padding: 1rem;
        }}
        
        .tests-header {{
            font-size: 0.8rem;
            text-transform: uppercase;
            color: var(--text-muted);
            margin-bottom: 0.75rem;
            font-weight: 600;
            letter-spacing: 0.05em;
        }}
        
        .test-case {{
            padding: 0.75rem 0;
            border-bottom: 1px solid var(--border-color);
        }}
        
        .test-case:last-child {{
            border-bottom: none;
            padding-bottom: 0;
        }}
        
        .test-request {{
            font-family: 'SF Mono', 'Consolas', monospace;
            margin-bottom: 0.5rem;
        }}
        
        .method {{
            background: var(--accent-green);
            color: var(--bg-primary);
            padding: 0.15rem 0.4rem;
            border-radius: 4px;
            font-size: 0.75rem;
            font-weight: 600;
        }}
        
        .path {{
            color: var(--accent-blue);
            margin-left: 0.5rem;
        }}
        
        .test-body {{
            font-size: 0.85rem;
            color: var(--text-secondary);
            margin-bottom: 0.5rem;
        }}
        
        .test-body code {{
            background: var(--bg-secondary);
            padding: 0.15rem 0.4rem;
            border-radius: 4px;
            font-size: 0.8rem;
        }}
        
        .assertions {{
            display: flex;
            flex-direction: column;
            gap: 0.25rem;
        }}
        
        .assertion {{
            color: var(--text-secondary);
            font-size: 0.85rem;
            font-family: 'SF Mono', 'Consolas', monospace;
            display: flex;
            align-items: flex-start;
            gap: 0.5rem;
        }}
        
        .assertion-icon {{
            flex-shrink: 0;
        }}
        
        .assertion.passed {{
            color: var(--accent-green);
        }}
        
        .assertion.passed .assertion-icon::before {{
            content: '✓';
        }}
        
        .assertion.failed {{
            color: var(--accent-red);
        }}
        
        .assertion.failed .assertion-icon::before {{
            content: '✗';
        }}
        
        .assertion-message {{
            color: var(--accent-red);
            font-size: 0.75rem;
            margin-left: 1.25rem;
            opacity: 0.8;
        }}
        
        .footer {{
            text-align: center;
            padding: 2rem;
            color: var(--text-muted);
            font-size: 0.85rem;
        }}
        
        .update-toast {{
            position: fixed;
            bottom: 2rem;
            right: 2rem;
            background: var(--accent-purple);
            color: white;
            padding: 0.75rem 1.25rem;
            border-radius: 8px;
            font-size: 0.9rem;
            opacity: 0;
            transform: translateY(20px);
            transition: opacity 0.3s, transform 0.3s;
            z-index: 1000;
        }}
        
        .update-toast.show {{
            opacity: 1;
            transform: translateY(0);
        }}
        
        .results-summary {{
            background: var(--bg-secondary);
            border: 1px solid var(--border-color);
            border-radius: 8px;
            padding: 1rem 1.5rem;
            margin-bottom: 2rem;
            display: none;
        }}
        
        .results-summary.visible {{
            display: block;
        }}
        
        .results-summary.all-passed {{
            border-color: var(--accent-green);
        }}
        
        .results-summary.has-failures {{
            border-color: var(--accent-red);
        }}
        
        .results-summary.has-warnings {{
            border-color: #fbbf24;
        }}
        
        .results-summary h3 {{
            margin-bottom: 0.5rem;
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }}
        
        .results-summary.all-passed h3 {{
            color: var(--accent-green);
        }}
        
        .results-summary.has-failures h3 {{
            color: var(--accent-red);
        }}
        
        .results-summary.has-warnings h3 {{
            color: #fbbf24;
        }}
    </style>
</head>
<body>
    <header class="header">
        <div class="header-content">
            <div class="logo">
                <span class="logo-icon"><img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAYAAACqaXHeAAAAAXNSR0IArs4c6QAAAGxlWElmTU0AKgAAAAgABAEaAAUAAAABAAAAPgEbAAUAAAABAAAARgEoAAMAAAABAAIAAIdpAAQAAAABAAAATgAAAAAAAAEsAAAAAQAAASwAAAABAAKgAgAEAAAAAQAAAECgAwAEAAAAAQAAAEAAAAAA9Ed7egAAAAlwSFlzAAAuIwAALiMBeKU/dgAAEHlJREFUeAHtWAlsHOd1/ubck8eSXN4SSYmkjsi2bDm+a8RxfMROfCB1FAdJYAeo7LooUsNoUBQtwLZO01ZBGyOGgbRu66Ot4wZJmig2FNeHbEuxLdk6LImSqIOHeIlckrvcc3Z2Zvq9WVKSK/mQY7VAu780w93Z+Y/3/d/73ns/UGkVBCoIVBCoIFBBoIJABYEKAhUEKghUEKggUEHg/x0Cyv+sxZ6Cu6A2BZ8OxlPvRLuMqYbGqFo7lXGTo3ZobrhqbWa25/cL6EOJ6/IAhdf5becNgL4+T311z9cvKGZTretixbb6QKnX8OxOzSm014WU+oaQFgurTk1Q9cx8yS0WoaeztjI/ayFVdNTppOVOHEsHjo9ZxlCktm7Ppp/+6O3zAch5AeDyu7/VFBt/6+EvLXO/nsoVA7KZzVEFKxtMtIQVRFQHqqrChQLX86AqClTPheUAcxZweKaIiayDkqdjSczAsZRnPXVQ+0HnZ+98+Md//UepT5ITnzAAnvK79/z27YnBo39zR6fdc2tPiKarSBc9jKRszOaBaRoW0IDWKhXNVTpMQ8VMzsHxpI2pTAl1EQPNEQ3t1SpiQZ1AAXbJxZN7cjicD+9wm5Y/9PgTz2z9pNjwiQHQ+sWHGjbE9vx5xEnftzJqqVe1GSg64sYKNFWByQvc7XmCMTDn4O2xPHYlFMwqVVgZTGFdi4lLmwNojHh8F3AoF7YLMgRkB7tynF8N2sjBKByxon/1t9n1G7HpvtxvyoZPAABPvf0Ln7/51vjcxphmreba8dmOAG114VLDLMrZbN7h7noYTZPWtKjKBFpqA9g2YsFyNayIkQ1hD+PzNgqOguoQGUCG1AeBGtJFABQgMyUFLx7NYmmVht2p0LbnMm1/sOUXP6E2fPz2GwHwmbsfarjc2913S2v+PsUu6GMpC9d3BZGxFYyT6tN5F9PpIkxNRRvp3hzVEY+oCOjALwcK6KrVcUFcx88G8rg4rqGz1sA8+57IOJikO8jfgKGhPqyigf1aIgqKBPTVoTyu7QhjX8pIvZCo/q56z6Pf/8EtPVSPc28fG4Crr7/tukuCo4/ef5G5OpEt4fXjBaysVZCmkom4xbiLXTSoPkjRM8rT2KSyTv9/Z8JGiqy4vjNIqgMp9nnxWAG39oYQ5G4rvtvAZ0uGnaayHoaSRbLJgy5soDBMkvxf/lQYAzMO/vGA+vJo9ar7X/vJE4fPFYKPBcCdX/vmRTdo+1/uiRTqfnnM5U6Y+J1POWgLk9pRDdVBjcpOPybdxYcXmwja2LyDHWMWbl/Bl9nkZ0NTcGS2hAPTBKGbwump/nMZROEnTQDhZROtRK4EYoEXRj1sndBwe3sWaxpD2J6Mbn+qav31+x/7vYw/8Ee8icueU6v7fF/15e6Bx+9eodZxLZhL53FjWxFx+uuFTSYiVDAxnML9HuO5fuS5m9uOW7iOO69QH+SfPLf5fncdgaNvvD1ehEHV8yiY/kUIHI4lxsv3RkaIZXy3NeTi2ngeR+ZctNC97uoqXvaZiWe/I3J5LgaRkOfSPOX+i/7k+3d3WrdtO57H8ZyG6zqCuL3bwBujFmxoDGGM76ftuowuX2X3/5M0vyhuoo0hzqExQnW/cc3SZyld5p2JIkKmuBCf+QCphIAaKGPwJm606VAWFzfquHFZENMFleJZgura6K4qfbrY/It9+wcOHygP/OH3cwJgw1d2fvWmpux3Dp3IKEup0tmigyuXBrg4Bd31Afx6pABdcdEY1nzf9lfNNZic5Q1qhEmqr2sJoMSEx6M1Gl/QiAyx8JsYKLu5ZTCHZUyATJUdF34TrCRpev5IHqsbAuip06Gwb4nPwoqDJDVFIG2LeteUum74j8MHds19uPkE9aO8JO+sX7++t1OZ+btktqheszTix+hG0j6iCZFld1zc1B3EnsmSH+50oTgXR5sxyLifyHm4qp3Gu44vhC5pPZ4u4VCiCKbCvripfL824GFdawivDtuEpzxGeY0eNh8tR45VDQbHoWvQLXprVAwnS7hqSYjao0NzrNZVzsHH7ur7dwbbD28fiQFf7NsUzu/68bNra/Krb1gWQFVAxVtjRVxN+pdpzKUSBUlglpDGrw8XUceUtzqoYt6iWA2LwkfoBgoXaxOkIgaSLp7el8dTzPCqKJpC4wyjgYS9rpiOBMPoKLNHCY1SE700bKGBzFrXbPrgE1c/N5CQykQSiQJweasBzXMwMJXtPnJkf2lwaOg14M8+EIUPBaCPkjz+lxu+e1u79ZU7egyEdAU7p5iyMsx11NKXZR0LXBcm8DHizPtfHyEIBGAbgWolrYdpzL6Ew0rHQ0/M9IF5boROEG1AoJTCzcsM/gYcYFg7MmOjmmgOpplBkmH7E7bvRlf6DJJZaL74BIERnWiMGth9okT9AegdWBU3MDlfuKp+9a/eOnTw4LEPQuBDAbC27rjpzsbZR7+xxlQki8sUXe5+Ab/F3feXsiC6zHXo/1R/PpTYnimp2PhuALMMFT0xoInqfXGzgYuag5jKufjh2ylM2SHU1FTh6NgMmgjSjctD6OXux8iwtC2FkYtH9gdRtG0mWKbvrwH6vSZRgnPJAoQJQU6eoR6NE8H2agNBgvfpFk1/4+jsFdEbv/UvI2+/RH6cvX0gAK94nv7yI9/7h2XRYqcULpKR7aBKx6nQHTUS68sKLaAcm7PRz53aO21jkD4ZC+uYoTB9tVfBZaRmjDSXeH9sjknTKGuBKQ8MeAgGA5iZz/uMckolLK3WEKZqSubYwxQ5XSgRHA0l28GuccvPMBN5xl8yIMJCKiAiI9rBhGsPmdlZRzcgIFsZbmczxfrDx4YTE2Ojvz67+aAQf0Cb6x+43shO/TGzPWXHuI0QHe5oyqGSBzFCX945nsfBWfpq2vULn44aA2sY5tbST0UTGgIujTdBHPwwOJV1sWvSwQH22Z3gzpkCQBDpbI4ZpIvuWiBZ8Oj3uj+ewp3uJgizGRs3dJhU/gCBUpFjLBzmOoZSri+iUmtUiQByvDHOcYyiK9LRRjC3Thnd33jggX/d+tJLrEXPbJSQ92+1IfPLcQb25ioHV3Axf7FTQ50ZRAB5H/0W0k0oJ6KokZKSsAj9xfj+6SIXL2rNTI/ukcp72E49iLK4eZmCpqtcIZucC0jeN5ZVcIiBq5H5/nZWipe2BtlXBTFl2PQwR2Ci1PUY+zc0UTu48VIXSGY4QaP7Z10/Xd6ciOPO5iRuWGVgb8Kjy4XiY0eHvsCpnvQn/G83Lu3s7WsPPNDtZmd7G7UMF6v6MbzaSaHFKODm5QFcu9TE6niAiypnfnTBcuzncAUKgZwBtETLSZHFCu8VGn1hk4Ef9edQYAUoIW6xSbgUKm8edHyfnmERJTm+wagh4NTT5YboVj7b2UlApUf4IDSxwJJS+nNLdfRSlJvdaYRVm70Uuhz7OgnNyiQv27hxIyXyzPa+AATUANlbiNebjNvUekH43jUGLm0C3qWvSSbn7zbHPJnR8bMscoJVXE2APsods1kWb2bycgUV/E3WALumwWrwzGlF1GxqyuN78rhySQSHSWMxWsaWomqSwiku8d5WjgI2hXiGteBImiy92vQj0xzDr86Q2BbxwstjWvvBo8Nr39u3/O3MlfB5X19fOJecW9cVthrrqwy/wpukLHfVmaRUAP0n8khRnCRZFSkWyi82qd1HqAmdpH+RycrzAzlmbjqzOBdP7GNpzDgvHWTXpS3+FULIjvcnNfzsEOsL5hvvMF8YZn4gx2mczmcVmHDJnD6DRP/4Tdxv20jeZ1itX4Xq2E19krOEOA8fGopj9XOp+TWciz3e284KwMRMutMoJlsa9WysnkdUexMullP1JcwZqou19M/t4yW6xqIBpxCwSIu8w9qdE28+YmFFvYkVrPkf2ZHBbJFawflPvX2KPbLTAobBQZ/ut/y84ZaeIHYysxzjYUoHBXCCf4kRO3EM33hmmlzDEUYWi8dHvSzHc/wrwjlFzXCYSmtMOTuN/KpIOND+4IMPNr/X/PdJhTP5/AUrqp2uhoCnij9LlrWqKcAMTI6pFPTSKIV0lRSXeZHvjGKULG6MO1bkIt4cLaCX+foljAibePjx2piHoE7NpaFirFynNzFensmOzJc0PLYz7zPic51k3IxEGSmZy2ceYjyX4TdGYOwmU65lTSIFlfDDJIjLeZwkwHiOg65qJqb5qYbpufTqcq9T9zMY8ErfK3p6Pr2sU5tt0mndIap5K4+rghQU2TuZ1+VMl7eb2MHSVQBS/MWXd2eCx167JwqM1x5WNhoY5ve/35nz+9h2ETaTmsXL4eLEcPleYg5Q/suakr675biNnxO4BmZ5zbykApQwujif0EgOR3Ywsqxk+hcLCRtd//yQ5YZ/ODPO7NNW6AYU4y5jfrVhBpadMr386QwAnp18ro7itVIv5ZdM89RFwszqelZdpLYYLxfXDEoDljNrk53WRZz4X3z+BHP4L62K+Mdiu044+CeK2lBKdqJ0mpHlzz4ApGhpAQABYfEiYvjndwt4a8LBmwT6qiUGlpNRknPIoiU6SC4yQ1DWNGo+M4WV5aaAkZnnBgaOzlOpuGHFTKonm8u20A3qFl7y/yz2OPlsfn78wmw6c8XmVLv+zN4sMzJWXbS6yBpWJjWpAwFe8nkdU1s51JzkIihzfgEjGeCauIqOKg/PvJvGT/uzpD4zxgXaS9xf/Cy7T9wWMspTriG/63TuMbLn4S0zCPNIbS3H7IoFcJwnSirnl3d2Mf+/ZolON3T9MConz5LYuGRQhrGyPujh8Il5fO9dE3uwrCWXnFmXzWZ7ThrLD2dJhEKHTaP45GQ696eT+WjQy7ZgO48XIqqFKpWHFW4eIcVipuWglVlKPcXuhcNp3LU6goM80moi3cQom/R+dTjnx2tdKMMF+23hs8sdlssTBpAd5ZT09Pf4Nt89xPA7QBG6oyfiUzyZtXlwamDfTAmuEcAoy+w3Jy0abCDnmjw2DyLp6Eg7JiwlgDEjjcGpAhqb3IPhcOhZ0zSPlxdSvi+s6vRHEgY9deeLl2zOmzVrYy2seqBW6ZxMYr/QSUIaeUH0uOCShXQyhSolh5xVQH0kgGCAlZ3NEpenc6IPvjTJTMICKpg8ka/CBt3QYVnWyWd8g4bKG2zcaXE8yR6XRDkKXWRyLoswx8/RuGgs7rudo/BwhL6u6VIH+CPAoVs5NisIt5QqjPV7Szs77vnhv/38+fLAp+5nBUB+3rBhQ0u4oQFD/f2BaDTYOj0912kYRgeX3RqKhFs5Y4vreM2upzZpuhqeGx9khhNB89JugiTRgXT0Y5ZsJA3iTGKWj8PCXcyT3xR5T36Um288v/svyk0oTZxJafHxYiGPxNB+1LZ1ETyT4uImKZ6TPGuYLFrWCQ4w5rqlEV3Vj1NPBtvb2xOt3d1eRzyevPfee8+oCssz+NOc++3b3/5m1d69E42hUFVz4cTRPzRq6geCNfEhhh7FkcKAfixx+mSTGHXad4f096lPJpxs8s5iW3i8MBSfytuO5iSGb/VCsWeKnrqrIRQ60Tk9nejbsoWp0v9qOzPLOm/LOUtGd97mqgxcQaCCQAWBCgIVBCoIVBCoIFBBoIJABYEKAhUEKghUEPi/hsB/AU+zq0eenIl3AAAAAElFTkSuQmCC" alt="NTNT Logo" /></span>
                <span class="logo-text">Intent <span>Studio</span></span>
            </div>
            <div class="header-file">
                <code>{file_path}</code>
            </div>
            <div class="header-controls">
                <a href="http://127.0.0.1:{app_port}/" target="_blank" class="open-app-btn" title="Open app in new window">🌐 Open App</a>
                <div class="app-status" id="app-status">
                    <span class="app-status-dot" id="app-status-dot"></span>
                    <span id="app-status-text">Checking app...</span>
                </div>
                {run_button_html}
                <div class="status-badge">
                    <span class="status-dot"></span>
                    <span>Live</span>
                </div>
            </div>
        </div>
    </header>
    
    <main class="main">
        <div class="results-summary" id="results-summary">
            <h3><span id="results-icon"></span> <span id="results-title"></span></h3>
            <p id="results-text"></p>
        </div>
        
        <div class="stats">
            <div class="stat">
                <div class="stat-value">{feature_count}</div>
                <div class="stat-label">Features</div>
            </div>
            <div class="stat">
                <div class="stat-value">{test_count}</div>
                <div class="stat-label">Test Cases</div>
            </div>
            <div class="stat" id="assertions-stat">
                <div class="stat-value">{assertion_count}</div>
                <div class="stat-label">Assertions</div>
            </div>
            <div class="stat passing" id="passed-stat" style="display: none;">
                <div class="stat-value" id="passed-count">0</div>
                <div class="stat-label">Passed</div>
            </div>
            <div class="stat failing" id="failed-stat" style="display: none;">
                <div class="stat-value" id="failed-count">0</div>
                <div class="stat-label">Failed</div>
            </div>
        </div>
        
        <div class="features-grid">
            {features_html}
        </div>
    </main>
    
    <footer class="footer">
        Intent-Driven Development • NTNT Language
    </footer>
    
    <div class="update-toast" id="toast">✨ Intent updated - refreshing...</div>
    
    <script>
        async function runTests() {{
            const btn = document.querySelector('.run-tests-btn');
            btn.textContent = '⏳ Running...';
            btn.classList.add('running');
            
            // Reset all states
            document.querySelectorAll('.assertion').forEach(el => {{
                el.classList.remove('passed', 'failed');
                el.querySelector('.assertion-icon').textContent = '○';
            }});
            document.querySelectorAll('.feature-card').forEach(el => {{
                el.classList.remove('passed', 'failed');
            }});
            document.querySelectorAll('.feature-status').forEach(el => {{
                el.classList.remove('passed', 'failed');
                el.textContent = 'testing...';
            }});
            
            try {{
                const response = await fetch('/run-tests');
                const results = await response.json();
                
                if (results.error) {{
                    // Show error in summary
                    const summary = document.getElementById('results-summary');
                    const icon = document.getElementById('results-icon');
                    const title = document.getElementById('results-title');
                    const text = document.getElementById('results-text');
                    
                    summary.classList.add('visible', 'has-failures');
                    summary.classList.remove('all-passed');
                    icon.textContent = '⚠️';
                    title.textContent = 'Test Error';
                    text.textContent = results.error;
                    
                    // Also check app status
                    checkAppStatus();
                    return;
                }}
                
                // Update UI with results
                let passedCount = results.passed_assertions || 0;
                let failedCount = results.failed_assertions || 0;
                
                // Show stats
                document.getElementById('passed-stat').style.display = 'block';
                document.getElementById('failed-stat').style.display = 'block';
                document.getElementById('passed-count').textContent = passedCount;
                document.getElementById('failed-count').textContent = failedCount;
                
                // Update each feature
                for (const feature of results.features) {{
                    const featureCard = document.querySelector(`.feature-card[data-feature="${{feature.feature_id}}"]`);
                    const featureStatus = document.querySelector(`.feature-status[data-feature="${{feature.feature_id}}"]`);
                    
                    if (featureCard) {{
                        featureCard.classList.add(feature.passed ? 'passed' : 'failed');
                        
                        // Show warning if no implementation linked
                        if (!feature.has_implementation) {{
                            featureCard.classList.add('unlinked');
                            // Add warning badge if not already present
                            const header = featureCard.querySelector('.feature-header');
                            if (header && !header.querySelector('.unlinked-warning')) {{
                                const warningBadge = document.createElement('span');
                                warningBadge.className = 'unlinked-warning';
                                warningBadge.title = 'No @implements annotation found for this feature';
                                warningBadge.textContent = '⚠️ No code linked';
                                header.appendChild(warningBadge);
                            }}
                        }}
                    }}
                    if (featureStatus) {{
                        featureStatus.classList.add(feature.passed ? 'passed' : 'failed');
                        featureStatus.textContent = feature.passed ? 'passed' : 'failed';
                    }}
                    
                    // Update each test's assertions
                    for (let ti = 0; ti < feature.tests.length; ti++) {{
                        const test = feature.tests[ti];
                        for (let ai = 0; ai < test.assertions.length; ai++) {{
                            const assertion = test.assertions[ai];
                            const assertionEl = document.querySelector(
                                `.assertion[data-feature="${{feature.feature_id}}"][data-test="${{ti}}"][data-assert="${{ai}}"]`
                            );
                            if (assertionEl) {{
                                assertionEl.classList.add(assertion.passed ? 'passed' : 'failed');
                                const iconEl = assertionEl.querySelector('.assertion-icon');
                                iconEl.textContent = assertion.passed ? '✓' : '✗';
                                
                                // Remove any existing error message first
                                const existing = assertionEl.querySelector('.assertion-message');
                                if (existing) existing.remove();
                                
                                // Add error message if failed
                                if (!assertion.passed && assertion.message) {{
                                    const msgEl = document.createElement('div');
                                    msgEl.className = 'assertion-message';
                                    msgEl.textContent = assertion.message;
                                    assertionEl.appendChild(msgEl);
                                }}
                            }}
                        }}
                    }}
                }}
                
                // Show summary
                const summary = document.getElementById('results-summary');
                const icon = document.getElementById('results-icon');
                const title = document.getElementById('results-title');
                const text = document.getElementById('results-text');
                
                // Check for unlinked features
                const unlinkedCount = results.total_features - results.linked_features;
                const hasUnlinked = unlinkedCount > 0;
                
                summary.classList.add('visible');
                if (failedCount === 0 && !hasUnlinked) {{
                    summary.classList.remove('has-failures', 'has-warnings');
                    summary.classList.add('all-passed');
                    icon.textContent = '✓';
                    title.textContent = 'All Tests Passed!';
                    text.textContent = `${{passedCount}} assertions passed across ${{results.features.length}} features. Coverage: ${{results.linked_features}}/${{results.total_features}} features linked.`;
                }} else if (failedCount === 0 && hasUnlinked) {{
                    summary.classList.remove('all-passed', 'has-failures');
                    summary.classList.add('has-warnings');
                    icon.textContent = '⚠';
                    title.textContent = 'Tests Passed, But Missing Coverage';
                    text.textContent = `${{passedCount}} assertions passed. Coverage: ${{results.linked_features}}/${{results.total_features}} features linked. Add @implements annotations to link code.`;
                }} else {{
                    summary.classList.remove('all-passed', 'has-warnings');
                    summary.classList.add('has-failures');
                    icon.textContent = '✗';
                    title.textContent = 'Some Tests Failed';
                    text.textContent = `${{passedCount}} passed, ${{failedCount}} failed. Coverage: ${{results.linked_features}}/${{results.total_features}} features linked.`;
                }}
                
            }} catch (e) {{
                console.error('Test run failed:', e);
                alert('Failed to run tests: ' + e.message);
            }} finally {{
                btn.textContent = '▶ Run Tests';
                btn.classList.remove('running');
            }}
        }}
        
        // Poll for changes every 2 seconds
        async function checkForUpdates() {{
            try {{
                const response = await fetch('/check-update');
                const text = await response.text();
                if (text === 'changed') {{
                    const toast = document.getElementById('toast');
                    toast.classList.add('show');
                    setTimeout(() => {{
                        window.location.reload();
                    }}, 500);
                }}
            }} catch (e) {{
                // Server might be restarting, ignore
            }}
        }}
        
        setInterval(checkForUpdates, 2000);
        
        // Check app status
        async function checkAppStatus() {{
            const statusEl = document.getElementById('app-status');
            const textEl = document.getElementById('app-status-text');
            
            try {{
                const response = await fetch('/app-status');
                const data = await response.json();
                
                statusEl.classList.remove('running', 'error', 'starting');
                
                if (data.running && data.healthy) {{
                    statusEl.classList.add('running');
                    textEl.textContent = 'App running';
                    textEl.title = 'Status: ' + data.status;
                }} else if (data.running && !data.healthy) {{
                    statusEl.classList.add('error');
                    textEl.textContent = 'App error';
                    textEl.title = data.error || 'Routes not registered';
                }} else {{
                    statusEl.classList.add('error');
                    textEl.textContent = 'App not responding';
                    textEl.title = data.error || 'Connection failed';
                }}
            }} catch (e) {{
                statusEl.classList.remove('running', 'error', 'starting');
                statusEl.classList.add('error');
                textEl.textContent = 'Status check failed';
            }}
        }}
        
        // Check app status periodically
        setInterval(checkAppStatus, 5000);
        checkAppStatus();
        
        // Auto-run tests on load
        setTimeout(runTests, 500);
    </script>
</body>
</html>"##,
        file_path = html_escape(file_path),
        app_port = app_port,
        feature_count = feature_count,
        test_count = test_count,
        assertion_count = assertion_count,
        features_html = features_html,
        run_button_html = run_button_html
    )
}

/// Render error page for Intent Studio
fn render_intent_studio_error(error: &str, file_path: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Intent Studio - Error</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
            background: #0d1117;
            color: #e6edf3;
            display: flex;
            align-items: center;
            justify-content: center;
            min-height: 100vh;
            margin: 0;
        }}
        .error-box {{
            background: #161b22;
            border: 1px solid #f85149;
            border-radius: 12px;
            padding: 2rem;
            max-width: 600px;
            text-align: center;
        }}
        .error-icon {{
            font-size: 3rem;
            margin-bottom: 1rem;
        }}
        .error-title {{
            font-size: 1.5rem;
            margin-bottom: 0.5rem;
        }}
        .error-file {{
            color: #8b949e;
            font-size: 0.9rem;
            margin-bottom: 1rem;
        }}
        .error-message {{
            background: #21262d;
            padding: 1rem;
            border-radius: 8px;
            font-family: 'SF Mono', monospace;
            font-size: 0.85rem;
            color: #f85149;
            text-align: left;
            white-space: pre-wrap;
        }}
        .retry-note {{
            color: #8b949e;
            font-size: 0.85rem;
            margin-top: 1rem;
        }}
    </style>
</head>
<body>
    <div class="error-box">
        <div class="error-icon">⚠️</div>
        <div class="error-title">Parse Error</div>
        <div class="error-file">{file_path}</div>
        <div class="error-message">{error}</div>
        <div class="retry-note">Fix the error and save - the page will refresh automatically.</div>
    </div>
    
    <script>
        setInterval(async () => {{
            try {{
                const response = await fetch('/check-update');
                const text = await response.text();
                if (text === 'changed') {{
                    window.location.reload();
                }}
            }} catch (e) {{}}
        }}, 2000);
    </script>
</body>
</html>"##,
        file_path = html_escape(file_path),
        error = html_escape(error)
    )
}

/// Simple HTML escaping
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Collect all .tnt files from a path (file or directory)
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
        TypeExpr::WithEffect { value_type, effect } => {
            format!(
                "{} / {}",
                type_to_string(value_type),
                type_to_string(effect)
            )
        }
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
        match stmt {
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
                            TemplatePart::Expr(expr) => {
                                collect_fn(expr, names);
                            }
                            TemplatePart::ForLoop { iterable, body, .. } => {
                                collect_fn(iterable, names);
                                collect_from_template_parts(body, names, collect_fn);
                            }
                            TemplatePart::IfBlock {
                                condition,
                                then_parts,
                                else_parts,
                            } => {
                                collect_fn(condition, names);
                                collect_from_template_parts(then_parts, names, collect_fn);
                                collect_from_template_parts(else_parts, names, collect_fn);
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
                collect_from_expr(body, names);
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
            Pattern::Tuple(patterns) | Pattern::Array(patterns) => {
                for p in patterns {
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
        | Statement::Import { .. }
        | Statement::Protocol { .. } => {}
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
