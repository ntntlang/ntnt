//! Intent Language CLI
//!
//! Command-line interface for the Intent programming language.

use clap::{Parser, Subcommand};
use colored::*;
use intent::{interpreter::Interpreter, lexer::Lexer, parser::Parser as IntentParser};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "intent")]
#[command(author = "Intent Language Team")]
#[command(version = "0.1.5")]
#[command(about = "Intent - A programming language for AI-driven development", long_about = None)]
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
    /// Run an Intent source file
    Run {
        /// The source file to run
        #[arg(value_name = "FILE")]
        file: PathBuf,
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
    println!("{}", "Intent Programming Language v0.1.4".green().bold());
    println!("Type {} for help, {} to exit\n", ":help".cyan(), ":quit".cyan());

    let mut rl = DefaultEditor::new()?;
    let mut interpreter = Interpreter::new();

    loop {
        let readline = rl.readline(&format!("{} ", "intent>".blue().bold()));
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
