// NTNT Syntax Highlighting - Prism.js language definitions
// Matches VS Code tmLanguage for consistent highlighting

// Define NTNT language for Prism.js
Prism.languages.ntnt = {
    'comment': [
        { pattern: /\/\/.*/, greedy: true },
        { pattern: /\/\*[\s\S]*?\*\//, greedy: true }
    ],
    'annotation': { pattern: /@\w+:?[^\n]*/, greedy: true },
    'string': [
        { pattern: /r#"[\s\S]*?"#/, greedy: true },
        { pattern: /r"[^"]*"/, greedy: true },
        { pattern: /"(?:[^"\\]|\\.)*"/, greedy: true }
    ],
    'contract': /\b(?:requires|ensures|invariant|old|result)\b/,
    'keyword': /\b(?:if|else|while|for|in|loop|break|continue|return|match|when|async|await)\b/,
    'declaration': /\b(?:fn|let|mut|const|struct|enum|impl|trait|type|import|export|from|pub)\b/,
    'builtin': /\b(?:print|println|len|json|html|text|template|listen|get|post|put|delete|serve_static|connect|query|parse|map)\b/,
    'boolean': /\b(?:true|false)\b/,
    'type': /\b(?:Int|Float|String|Bool|Map|Array|Result|Option|Ok|Err|Some|None)\b/,
    'number': /\b\d+\.?\d*\b/,
    'operator': /->|=>|&&|\|\||[+\-*/%=<>!]+/,
    'punctuation': /[{}()\[\];,.:]/
};

// Define Intent file language
Prism.languages.intent = {
    'comment': { pattern: /^#.*/m, greedy: true },
    'keyword': /^(?:Feature|Scenario|When|Then|Given|Constraint|Component):/m,
    'label': /^\s+(?:id|description|test|assert|request|body|status):/m,
    'method': /\b(?:GET|POST|PUT|DELETE|PATCH)\b/,
    'string': { pattern: /"(?:[^"\\]|\\.)*"/, greedy: true },
    'assertion': /\b(?:status|body contains|body not contains|body matches)\b/,
    'number': /\b\d+\b/
};

// Auto-detect language and highlight on page load
document.addEventListener("DOMContentLoaded", () => {
    // Add language class to code blocks for Prism
    document.querySelectorAll("pre code").forEach((codeEl) => {
        const text = codeEl.textContent;
        if (text.includes('Feature:') || text.includes('Scenario:')) {
            codeEl.className = 'language-intent';
        } else if (text.includes('fn ') || text.includes('import ') || text.includes('let ') ||
                   text.includes('get(') || text.includes('listen(') ||
                   text.includes('requires ') || text.includes('ensures ')) {
            codeEl.className = 'language-ntnt';
        }
    });

    // Run Prism highlighting
    Prism.highlightAll();

    // Add copy buttons to code blocks
    // Wrap pre elements in a container so the button stays fixed when code scrolls
    document.querySelectorAll("pre").forEach((pre) => {
        // Skip if already wrapped
        if (pre.parentElement.classList.contains("code-container")) {
            return;
        }

        // Create container wrapper
        const container = document.createElement("div");
        container.className = "code-container";
        pre.parentNode.insertBefore(container, pre);
        container.appendChild(pre);

        // Create copy button (positioned in container, not pre)
        const button = document.createElement("button");
        button.type = "button";
        button.className = "copy-button";
        button.textContent = "Copy";

        button.addEventListener("click", async () => {
            const codeEl = pre.querySelector("code");
            const codeText = codeEl ? codeEl.innerText : pre.innerText;

            try {
                await navigator.clipboard.writeText(codeText.trim());
                button.classList.add("copied");
                button.textContent = "Copied";
                setTimeout(() => {
                    button.classList.remove("copied");
                    button.textContent = "Copy";
                }, 1200);
            } catch (err) {
                const textarea = document.createElement("textarea");
                textarea.value = codeText.trim();
                textarea.style.position = "fixed";
                textarea.style.opacity = "0";
                document.body.appendChild(textarea);
                textarea.select();
                document.execCommand("copy");
                document.body.removeChild(textarea);
                button.classList.add("copied");
                button.textContent = "Copied";
                setTimeout(() => {
                    button.classList.remove("copied");
                    button.textContent = "Copy";
                }, 1200);
            }
        });

        // Add button to container (not pre), so it stays fixed when pre scrolls
        container.appendChild(button);
    });
});
