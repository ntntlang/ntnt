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
                                    // Check if source already uses raw string
                                    let line = find_line_number(source_lines, s);
                                    let is_already_raw = line.map_or(false, |ln| {
                                        if let Some(source_line) = source_lines.get(ln - 1) {
                                            // Check for r"..." or r#"..."# pattern before the string content
                                            source_line.contains(&format!("r\"{}\"", s))
                                                || source_line.contains(&format!("r#\"{}\"#", s))
                                        } else {
                                            false
                                        }
                                    });

                                    if !is_already_raw {
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
                                }
                                Expression::InterpolatedString(parts) => {
                                    // Interpolated string used as route - check if it has expression parts
                                    // which likely means they used {param} expecting route params but got interpolation
                                    let has_expression_parts =
                                        parts.iter().any(|p| matches!(p, StringPart::Expr(_)));
                                    if has_expression_parts {
                                        issues.push(json!({
                                            "severity": "warning",
                                            "rule": "route_pattern_interpolation",
                                            "message": "Route pattern uses string interpolation. If you intended route parameters like {id}, use a raw string: r\"/path/{param}\"",
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
    println!("Starting server on port {}...", port);

    // Get the current executable path to run ntnt
    let current_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ntnt"));

    // Start the NTNT app server as a subprocess
    use std::process::Command;
    let mut app_process = Command::new(&current_exe)
        .arg("run")
        .arg(&ntnt_path)
        .env("NTNT_LISTEN_PORT", port.to_string())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to start app server: {}", e))?;

    // Give the server time to start
    std::thread::sleep(std::time::Duration::from_millis(1000));

    // Run tests using the new IAL engine
    let results = intent::run_tests_against_server(&intent_file, port, &source_files);

    // Print results
    println!();
    println!("{}", "=== Test Results ===".cyan().bold());
    println!();

    // Calculate stats
    let mut scenarios_passed = 0;
    let mut scenarios_failed = 0;
    let mut scenarios_skipped = 0;
    let mut features_failed = 0;

    for feature in &results.features {
        let status_icon = if feature.passed {
            "✓".green()
        } else {
            "✗".red()
        };
        println!("{} Feature: {}", status_icon, feature.feature_name.bold());

        if !feature.passed {
            features_failed += 1;
        }

        for scenario in &feature.scenarios {
            let icon = match scenario.status.as_str() {
                "pass" => {
                    scenarios_passed += 1;
                    "  ✓".green()
                }
                "fail" => {
                    scenarios_failed += 1;
                    "  ✗".red()
                }
                "skip" => {
                    scenarios_skipped += 1;
                    "  ⏭️ ".yellow()
                }
                _ => "  ⧗".yellow(),
            };
            println!("{} {}", icon, scenario.name);

            if verbose {
                if let Some(ref given) = scenario.given_clause {
                    println!("      Given {}", given.dimmed());
                }
                println!("      When {}", scenario.when_clause.dimmed());
                for outcome in &scenario.outcomes {
                    println!("      → {}", outcome.dimmed());
                }
                if let Some(ref test_result) = scenario.test_result {
                    for assertion in &test_result.assertions {
                        let a_icon = if assertion.passed { "✓" } else { "✗" };
                        println!("        {} {}", a_icon, assertion.assertion_text.dimmed());
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

            if verbose {
                for assertion in &test.assertions {
                    let a_icon = if assertion.passed { "✓" } else { "✗" };
                    println!("      {} {}", a_icon, assertion.assertion_text.dimmed());
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
                "pass" => {
                    scenarios_passed += 1;
                    "  ✓".green()
                }
                "fail" => {
                    scenarios_failed += 1;
                    "  ✗".red()
                }
                "skip" => {
                    scenarios_skipped += 1;
                    "  ⏭️ ".yellow()
                }
                _ => "  ⧗".yellow(),
            };
            println!("{} {}", icon, scenario.name);
        }
        println!();
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

    // Start simple HTTP server with non-blocking accepts BEFORE opening browser
    let listener = TcpListener::bind(&addr)
        .map_err(|e| anyhow::anyhow!("Failed to bind to {}: {}", addr, e))?;
    listener.set_nonblocking(true)?;

    // Now open browser (server is ready to accept connections)
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
                                // Collect all source files for annotation checking
                                let project_dir = tnt_path
                                    .as_ref()
                                    .and_then(|p| p.parent())
                                    .unwrap_or(std::path::Path::new("."));
                                let source_files: Vec<(String, String)> =
                                    collect_tnt_files(&project_dir.to_path_buf())
                                        .unwrap_or_default()
                                        .into_iter()
                                        .filter_map(|p| {
                                            let content = fs::read_to_string(&p).ok()?;
                                            Some((p.to_string_lossy().to_string(), content))
                                        })
                                        .collect();
                                let results = intent::run_tests_against_server(
                                    &intent_file,
                                    app_port,
                                    &source_files,
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

/// Intent Studio Lite HTML (embedded at compile time)
const INTENT_STUDIO_LITE_HTML: &str = include_str!("intent_studio_lite.html");

/// Render the Intent Studio HTML page (Lite version with IAL support)
fn render_intent_studio_html(
    _intent_file: &intent::IntentFile,
    file_path: &str,
    app_port: u16,
) -> String {
    // Return the Lite HTML with placeholders filled in
    INTENT_STUDIO_LITE_HTML
        .replace("server.intent", &html_escape(file_path))
        .replace(
            "http://localhost:8081",
            &format!("http://127.0.0.1:{}", app_port),
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
                            TemplatePart::FilteredExpr { expr, filters } => {
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
