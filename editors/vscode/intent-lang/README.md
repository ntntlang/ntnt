# Intent Language for VS Code

Syntax highlighting and language support for the [Intent programming language](https://github.com/joshcramer/intent).

## Features

- **Syntax Highlighting** for `.intent` and `.itn` files
- **Code Snippets** for common patterns
- **Bracket Matching** and auto-closing
- **Comment Toggling** (line and block)
- **Folding** support

## Supported File Extensions

- `.intent` - Standard Intent source files
- `.itn` - Short form Intent source files

## Highlighting

The extension provides semantic highlighting for:

- **Keywords**: `fn`, `let`, `const`, `struct`, `impl`, `trait`, `enum`, etc.
- **Control Flow**: `if`, `else`, `while`, `for`, `match`, `return`, etc.
- **Contracts**: `requires`, `ensures`, `invariant`, `old`, `result`
- **Effects**: `pure`, `io`, `network`, `database`, `throws`
- **Types**: `Int`, `Float`, `String`, `Bool`, `Array`, `Option`, `Result`, etc.
- **Built-in Functions**: `print`, `len`, `map`, `filter`, `reduce`, etc.
- **Comments**: Line (`//`) and block (`/* */`)
- **Strings**: Double-quoted, single-quoted, and template strings

## Snippets

| Prefix    | Description                                |
| --------- | ------------------------------------------ |
| `fn`      | Function definition                        |
| `fnc`     | Function with contracts (requires/ensures) |
| `fnr`     | Function with return type                  |
| `struct`  | Struct definition                          |
| `structi` | Struct with invariant                      |
| `impl`    | Implementation block                       |
| `if`      | If-else statement                          |
| `while`   | While loop                                 |
| `for`     | For-in loop                                |
| `let`     | Let binding                                |
| `letm`    | Mutable let binding                        |
| `match`   | Match expression                           |
| `req`     | Requires clause                            |
| `ens`     | Ensures clause                             |
| `inv`     | Invariant clause                           |
| `old`     | Old value reference                        |
| `enum`    | Enum definition                            |
| `trait`   | Trait definition                           |
| `test`    | Test function                              |
| `pr`      | Print statement                            |
| `ass`     | Assert statement                           |

## Example

```intent
struct BankAccount {
    balance: Int,
    owner: String
}

impl BankAccount {
    invariant self.balance >= 0

    fn deposit(self, amount: Int)
        requires amount > 0
        ensures self.balance == old(self.balance) + amount
    {
        self.balance = self.balance + amount
    }

    fn withdraw(self, amount: Int) -> Bool
        requires amount > 0
        ensures result == true implies self.balance == old(self.balance) - amount
    {
        if self.balance >= amount {
            self.balance = self.balance - amount
            return true
        }
        return false
    }
}
```

## Installation

### From VS Code Marketplace

Search for "Intent Language" in the VS Code Extensions view.

### Manual Installation

1. Clone or download this extension
2. Copy the `intent-lang` folder to your VS Code extensions directory:
   - **Windows**: `%USERPROFILE%\.vscode\extensions`
   - **macOS**: `~/.vscode/extensions`
   - **Linux**: `~/.vscode/extensions`
3. Restart VS Code

### Development

```bash
# Package the extension
cd editors/vscode/intent-lang
npx vsce package

# Install locally
code --install-extension intent-lang-0.1.0.vsix
```

## Contributing

Contributions are welcome! Please see the [main Intent repository](https://github.com/joshcramer/intent) for guidelines.

## License

MIT License - see the Intent project for details.
