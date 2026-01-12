//! NTNT Language CLI
//!
//! Command-line interface for the NTNT (Intent) programming language.

use clap::{Parser, Subcommand};
use colored::*;
use ntnt::{interpreter::Interpreter, lexer::Lexer, parser::Parser as IntentParser};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ntnt")]
#[command(author = "NTNT Language Team")]
#[command(version = "0.1.6")]
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
            verbose 
        }) => test_http_server(&file, get_requests, post_requests, put_requests, delete_requests, body, port, verbose),
        Some(Commands::Parse { file, json }) => parse_file(&file, json),
        Some(Commands::Lex { file }) => lex_file(&file),
        Some(Commands::Check { file }) => check_file(&file),
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
    println!("{}", "NTNT (Intent) Programming Language v0.1.6".green().bold());
    println!("Type {} for help, {} to exit\n", ":help".cyan(), ":quit".cyan());

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
    println!("  {}     - Show current environment bindings", ":env".cyan());
    println!();
    println!("{}", "Module System:".yellow().bold());
    println!("  {} - Import specific functions", r#"import { split, join } from "std/string""#.cyan());
    println!("  {}    - Import module with alias", r#"import "std/math" as math"#.cyan());
    println!();
    println!("{}", "Standard Library:".yellow().bold());
    println!("  {} - std/string: split, join, trim, replace, to_upper, to_lower", "String".cyan());
    println!("  {}   - std/math: sin, cos, tan, log, exp, PI, E", "Math".cyan());
    println!("  {} - std/collections: push, pop, first, last, reverse, slice", "Collections".cyan());
    println!("  {}    - std/env: get_env, args, cwd", "Environment".cyan());
    println!("  {}     - std/fs: read_file, write_file, exists, mkdir, remove", "Files".cyan());
    println!("  {}     - std/path: join, dirname, basename, extension, resolve", "Paths".cyan());
    println!("  {}      - std/json: parse, stringify, stringify_pretty", "JSON".cyan());
    println!("  {}      - std/time: now, sleep, elapsed, format_timestamp", "Time".cyan());
    println!("  {}    - std/crypto: sha256, hmac_sha256, uuid, random_bytes", "Crypto".cyan());
    println!("  {}       - std/url: parse, encode, decode, build_query, join", "URL".cyan());
    println!("  {}      - std/http: get, post, put, delete, request, get_json", "HTTP".cyan());
    println!();
    println!("{}", "Basic Examples:".yellow().bold());
    println!("  {}           - Variable binding", "let x = 42;".cyan());
    println!("  {}    - Arithmetic", "let y = x + 10;".cyan());
    println!("  {} - Function definition", "fn add(a, b) { a + b }".cyan());
    println!("  {}       - Function call", "add(1, 2)".cyan());
    println!();
    println!("{}", "Traits:".yellow().bold());
    println!("  {} - Define a trait", "trait Display { fn show(self); }".cyan());
    println!("  {} - Implement trait", "impl Display for Point { ... }".cyan());
    println!();
    println!("{}", "Loops & Iteration:".yellow().bold());
    println!("  {}  - For-in loop", "for x in [1, 2, 3] { print(x); }".cyan());
    println!("  {}    - Range (exclusive)", "for i in 0..5 { print(i); }".cyan());
    println!("  {}   - Range (inclusive)", "for i in 0..=5 { print(i); }".cyan());
    println!("  {} - Iterate strings", r#"for c in "hello" { print(c); }"#.cyan());
    println!();
    println!("{}", "Defer:".yellow().bold());
    println!("  {} - Runs on scope exit", "defer print(\"cleanup\");".cyan());
    println!();
    println!("{}", "Maps:".yellow().bold());
    println!("  {} - Map literal", r#"let m = map { "a": 1, "b": 2 };"#.cyan());
    println!();
    println!("{}", "String Interpolation:".yellow().bold());
    println!("  {} - Embed expressions", r#""Hello, {name}!""#.cyan());
    println!("  {} - With math", r#""Sum: {a + b}""#.cyan());
    println!();
    println!("{}", "Raw Strings:".yellow().bold());
    println!("  {}   - No escape processing", r#"r"C:\path\to\file""#.cyan());
    println!("  {} - With quotes inside", r##"r#"say "hi""#"##.cyan());
    println!();
    println!("{}", "Trait Bounds:".yellow().bold());
    println!("  {} - Bounded generic", "fn sort<T: Comparable>(arr: [T])".cyan());
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
    println!("  {} - Generic function", "fn id<T>(x: T) -> T { x }".cyan());
    println!();
    println!("{}", "Contracts:".yellow().bold());
    println!("  {} - Precondition", "fn div(a, b) requires b != 0 { a / b }".cyan());
    println!("  {} - Postcondition", "fn abs(x) ensures result >= 0 { ... }".cyan());
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
        anyhow::bail!("No requests specified. Use --get, --post, --put, or --delete to specify requests.");
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
                                
                                response_data = Some((method.clone(), req_path.clone(), status_code, body));
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
        println!("{}", format!("[REQUEST {}] {} {}", req_num, method, path).cyan().bold());
        
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
            println!("{}", serde_json::to_string_pretty(&json).unwrap_or_else(|_| body.to_string()));
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
