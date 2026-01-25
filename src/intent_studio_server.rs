//! Async Intent Studio Server
//!
//! High-performance async HTTP server for Intent Studio using Axum + Tokio.
//!
//! ## Endpoints
//!
//! - `GET /` - Main Intent Studio UI
//! - `GET /api/status` - Check if intent file has changed
//! - `GET /api/app-status` - Check app server health
//! - `POST /api/run-tests` - Execute tests against app server
//! - `GET /api/results` - Get latest test results as JSON
//!
//! ## Architecture
//!
//! The server runs in a Tokio async runtime and handles multiple concurrent
//! connections efficiently. The app server runs as a subprocess on a separate port.

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::RwLock;

use crate::intent::{self, LiveTestResults};

/// Shared state for the Intent Studio server
pub struct StudioState {
    /// Path to the .intent file
    pub intent_path: PathBuf,
    /// Path to the .tnt file (optional)
    pub tnt_path: Option<PathBuf>,
    /// Port where the app server is running
    pub app_port: u16,
    /// Last modification time of the intent file
    pub last_modified: RwLock<SystemTime>,
    /// Cached test results
    pub cached_results: RwLock<Option<LiveTestResults>>,
}

impl StudioState {
    pub fn new(intent_path: PathBuf, tnt_path: Option<PathBuf>, app_port: u16) -> Self {
        let last_modified = fs::metadata(&intent_path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        StudioState {
            intent_path,
            tnt_path,
            app_port,
            last_modified: RwLock::new(last_modified),
            cached_results: RwLock::new(None),
        }
    }
}

/// App status response
#[derive(Serialize)]
pub struct AppStatus {
    pub running: bool,
    pub healthy: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// File change status response
#[derive(Serialize)]
pub struct ChangeStatus {
    pub changed: bool,
}

/// Create the Intent Studio router
pub fn create_router(state: Arc<StudioState>) -> Router {
    Router::new()
        // Main UI
        .route("/", get(serve_ui))
        // IAL Explorer
        .route("/ial-explorer", get(serve_ial_explorer))
        // Legacy endpoints (backwards compatible)
        .route("/check-update", get(check_update))
        .route("/app-status", get(app_status))
        .route("/run-tests", get(run_tests))
        // New API endpoints
        .route("/api/status", get(check_update))
        .route("/api/app-status", get(app_status))
        .route("/api/run-tests", post(run_tests))
        .route("/api/results", get(get_results))
        .route("/api/glossary", get(get_glossary))
        .with_state(state)
}

/// Serve the Intent Studio UI
async fn serve_ui(State(state): State<Arc<StudioState>>) -> impl IntoResponse {
    let intent_path_str = state.intent_path.to_string_lossy().to_string();

    match intent::IntentFile::parse(&state.intent_path) {
        Ok(_intent_file) => {
            let html = render_intent_studio_html(&intent_path_str, state.app_port);
            Html(html)
        }
        Err(e) => {
            let html = render_intent_studio_error(&e.to_string(), &intent_path_str);
            Html(html)
        }
    }
}

/// Check if the intent file has changed
async fn check_update(State(state): State<Arc<StudioState>>) -> Json<ChangeStatus> {
    let current_modified = fs::metadata(&state.intent_path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let mut last = state.last_modified.write().await;
    let changed = current_modified != *last;
    if changed {
        *last = current_modified;
    }

    Json(ChangeStatus { changed })
}

/// Check app server status
async fn app_status(State(state): State<Arc<StudioState>>) -> Json<AppStatus> {
    let app_url = format!("http://127.0.0.1:{}/", state.app_port);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build();

    match client {
        Ok(client) => match client.get(&app_url).send().await {
            Ok(resp) => {
                let status_code = resp.status().as_u16();
                if status_code == 404 {
                    Json(AppStatus {
                        running: true,
                        healthy: false,
                        status: Some(status_code),
                        error: Some("No routes registered (404)".to_string()),
                    })
                } else if status_code >= 500 {
                    Json(AppStatus {
                        running: true,
                        healthy: false,
                        status: Some(status_code),
                        error: Some("Server error".to_string()),
                    })
                } else {
                    Json(AppStatus {
                        running: true,
                        healthy: true,
                        status: Some(status_code),
                        error: None,
                    })
                }
            }
            Err(e) => Json(AppStatus {
                running: false,
                healthy: false,
                status: None,
                error: Some(e.to_string()),
            }),
        },
        Err(e) => Json(AppStatus {
            running: false,
            healthy: false,
            status: None,
            error: Some(e.to_string()),
        }),
    }
}

/// Run tests against the app server
async fn run_tests(State(state): State<Arc<StudioState>>) -> Response {
    match intent::IntentFile::parse(&state.intent_path) {
        Ok(intent_file) => {
            // Collect all source files for annotation checking
            let project_dir = state
                .tnt_path
                .as_ref()
                .and_then(|p| p.parent())
                .unwrap_or(std::path::Path::new("."));

            let source_files: Vec<(String, String)> = collect_tnt_files(&project_dir.to_path_buf())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|p| {
                    let content = fs::read_to_string(&p).ok()?;
                    Some((p.to_string_lossy().to_string(), content))
                })
                .collect();

            let results =
                intent::run_tests_against_server(&intent_file, state.app_port, &source_files);

            // Cache the results
            {
                let mut cached = state.cached_results.write().await;
                *cached = Some(results.clone());
            }

            Json(results).into_response()
        }
        Err(e) => {
            let error = serde_json::json!({
                "error": e.to_string()
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

/// Get cached test results
async fn get_results(State(state): State<Arc<StudioState>>) -> Response {
    let cached = state.cached_results.read().await;
    match cached.as_ref() {
        Some(results) => Json(results.clone()).into_response(),
        None => {
            let error = serde_json::json!({
                "error": "No test results available. Run tests first."
            });
            (StatusCode::NOT_FOUND, Json(error)).into_response()
        }
    }
}

/// Get glossary terms from the intent file
async fn get_glossary(State(state): State<Arc<StudioState>>) -> Response {
    match intent::IntentFile::parse(&state.intent_path) {
        Ok(intent_file) => {
            let glossary: Vec<serde_json::Value> = intent_file
                .glossary
                .as_ref()
                .map(|g| {
                    g.terms
                        .values()
                        .map(|gt| {
                            serde_json::json!({
                                "term": gt.term,
                                "meaning": gt.meaning
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Json(serde_json::json!({ "glossary": glossary })).into_response()
        }
        Err(e) => {
            let error = serde_json::json!({
                "error": e.to_string()
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

/// Serve the IAL Explorer UI
async fn serve_ial_explorer(State(state): State<Arc<StudioState>>) -> impl IntoResponse {
    let intent_path_str = state.intent_path.to_string_lossy().to_string();

    match intent::IntentFile::parse(&state.intent_path) {
        Ok(intent_file) => {
            let html = render_ial_explorer_html(&intent_file, &intent_path_str);
            Html(html)
        }
        Err(e) => {
            let html = render_intent_studio_error(&e.to_string(), &intent_path_str);
            Html(html)
        }
    }
}

/// Collect all .tnt files in a directory recursively
fn collect_tnt_files(dir: &PathBuf) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // Skip common non-source directories
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !matches!(name, "node_modules" | ".git" | "target" | "build" | "dist") {
                    files.extend(collect_tnt_files(&path)?);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("tnt") {
                files.push(path);
            }
        }
    }

    Ok(files)
}

/// Intent Studio Lite HTML (embedded at compile time)
const INTENT_STUDIO_LITE_HTML: &str = include_str!("intent_studio_lite.html");

/// Render the Intent Studio HTML page
fn render_intent_studio_html(file_path: &str, app_port: u16) -> String {
    INTENT_STUDIO_LITE_HTML
        .replace("server.intent", &html_escape(file_path))
        .replace(
            "http://localhost:8081",
            &format!("http://127.0.0.1:{}", app_port),
        )
}

/// Escape HTML special characters
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
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
            padding: 20px;
            box-sizing: border-box;
        }}
        .error-container {{
            max-width: 600px;
            padding: 40px;
            background: #161b22;
            border-radius: 12px;
            border: 1px solid #30363d;
        }}
        h1 {{
            color: #f85149;
            margin-bottom: 20px;
        }}
        .file {{
            color: #8b949e;
            font-size: 14px;
            margin-bottom: 20px;
        }}
        pre {{
            background: #0d1117;
            padding: 16px;
            border-radius: 8px;
            overflow-x: auto;
            font-family: 'SF Mono', Consolas, monospace;
            font-size: 14px;
            color: #f85149;
        }}
        .refresh {{
            margin-top: 20px;
            color: #8b949e;
        }}
    </style>
</head>
<body>
    <div class="error-container">
        <h1>Parse Error</h1>
        <div class="file">File: {}</div>
        <pre>{}</pre>
        <p class="refresh">Fix the error and refresh to continue.</p>
    </div>
    <script>
        // Auto-refresh every 2 seconds to check for fixes
        setInterval(() => location.reload(), 2000);
    </script>
</body>
</html>"##,
        html_escape(file_path),
        html_escape(error)
    )
}

/// Render the IAL Explorer HTML page
fn render_ial_explorer_html(intent_file: &intent::IntentFile, file_path: &str) -> String {
    // Build glossary terms JSON
    let glossary_json: Vec<String> = intent_file
        .glossary
        .as_ref()
        .map(|g| {
            g.terms
                .values()
                .map(|gt| {
                    format!(
                        r#"{{ "term": "{}", "meaning": "{}" }}"#,
                        escape_json_string(&gt.term),
                        escape_json_string(&gt.meaning)
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    // Build features for context
    let features_json: Vec<String> = intent_file
        .features
        .iter()
        .map(|f| {
            let scenarios: Vec<String> = f
                .scenarios
                .iter()
                .map(|s| {
                    let outcomes: Vec<String> = s
                        .outcomes
                        .iter()
                        .map(|o| format!(r#""{}""#, escape_json_string(o)))
                        .collect();
                    format!(
                        r#"{{ "name": "{}", "when": "{}", "outcomes": [{}] }}"#,
                        escape_json_string(&s.name),
                        escape_json_string(&s.when_clause),
                        outcomes.join(", ")
                    )
                })
                .collect();
            let feature_id = f.id.as_deref().unwrap_or("unknown");
            format!(
                r#"{{ "id": "{}", "name": "{}", "scenarios": [{}] }}"#,
                escape_json_string(feature_id),
                escape_json_string(&f.name),
                scenarios.join(", ")
            )
        })
        .collect();

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>IAL Explorer - {file_name}</title>
    <style>
        :root {{
            --bg-primary: #0d1117;
            --bg-secondary: #161b22;
            --bg-tertiary: #21262d;
            --bg-elevated: #1c2128;
            --border: #30363d;
            --border-hover: #484f58;
            --text-primary: #e6edf3;
            --text-secondary: #8b949e;
            --text-muted: #6e7681;
            --green: #3fb950;
            --green-bg: rgba(63, 185, 80, 0.15);
            --red: #f85149;
            --blue: #58a6ff;
            --blue-bg: rgba(88, 166, 255, 0.15);
            --purple: #a371f7;
            --purple-bg: rgba(163, 113, 247, 0.15);
            --orange: #f0883e;
            --teal: #39c5cf;
            --teal-bg: rgba(57, 197, 207, 0.15);
        }}

        * {{ box-sizing: border-box; margin: 0; padding: 0; }}

        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
            background: var(--bg-primary);
            color: var(--text-primary);
            line-height: 1.6;
            font-size: 14px;
        }}

        .header {{
            background: var(--bg-secondary);
            border-bottom: 1px solid var(--border);
            padding: 16px 24px;
            display: flex;
            align-items: center;
            justify-content: space-between;
            position: sticky;
            top: 0;
            z-index: 50;
        }}

        .logo {{
            display: flex;
            align-items: center;
            gap: 8px;
            font-weight: 600;
            white-space: nowrap;
        }}

        .logo img {{
            height: 24px;
            width: 24px;
            vertical-align: middle;
        }}

        .logo .ial {{ color: var(--purple); }}

        .btn {{
            display: inline-flex;
            align-items: center;
            gap: 6px;
            padding: 8px 12px;
            border-radius: 6px;
            font-size: 13px;
            font-weight: 500;
            cursor: pointer;
            border: 1px solid var(--border);
            background: var(--bg-tertiary);
            color: var(--text-primary);
            text-decoration: none;
            transition: all 0.1s ease;
        }}

        .btn:hover {{
            background: var(--bg-elevated);
            border-color: var(--border-hover);
        }}

        .container {{
            display: grid;
            grid-template-columns: 1fr 350px;
            min-height: calc(100vh - 60px);
        }}

        @media (max-width: 900px) {{
            .container {{ grid-template-columns: 1fr; }}
            .sidebar {{ display: none; }}
        }}

        .content {{
            padding: 24px;
            overflow-y: auto;
        }}

        .intro {{
            background: var(--blue-bg);
            border: 1px solid rgba(88, 166, 255, 0.3);
            border-radius: 8px;
            padding: 16px;
            margin-bottom: 20px;
            font-size: 13px;
            color: var(--blue);
        }}

        .feature-section {{
            background: var(--bg-secondary);
            border: 1px solid var(--border);
            border-radius: 8px;
            margin-bottom: 16px;
        }}

        .feature-header {{
            padding: 16px;
            border-bottom: 1px solid var(--border);
            cursor: pointer;
        }}

        .feature-header:hover {{
            background: var(--bg-tertiary);
        }}

        .feature-name {{
            font-weight: 600;
            color: var(--blue);
            margin-bottom: 4px;
        }}

        .feature-id {{
            font-size: 12px;
            color: var(--text-muted);
        }}

        .scenarios {{
            padding: 16px;
        }}

        .scenario {{
            margin-bottom: 16px;
            padding: 12px;
            background: var(--bg-tertiary);
            border-radius: 6px;
        }}

        .scenario-name {{
            font-weight: 500;
            margin-bottom: 8px;
        }}

        .when-clause {{
            color: var(--orange);
            font-family: ui-monospace, monospace;
            font-size: 13px;
            margin-bottom: 8px;
        }}

        .outcomes {{
            list-style: none;
        }}

        .outcome {{
            padding: 4px 0;
            display: flex;
            align-items: flex-start;
            gap: 8px;
        }}

        .outcome-arrow {{
            color: var(--teal);
            font-weight: 600;
        }}

        .glossary-term {{
            background: var(--purple-bg);
            color: var(--purple);
            padding: 1px 4px;
            border-radius: 3px;
            cursor: help;
        }}

        .glossary-term:hover {{
            background: rgba(163, 113, 247, 0.35);
        }}

        .sidebar {{
            background: var(--bg-secondary);
            border-left: 1px solid var(--border);
            padding: 20px;
            overflow-y: auto;
        }}

        .sidebar-title {{
            font-size: 12px;
            text-transform: uppercase;
            letter-spacing: 0.5px;
            color: var(--text-muted);
            margin-bottom: 16px;
            padding-bottom: 8px;
            border-bottom: 1px solid var(--border);
        }}

        .glossary-list {{
            list-style: none;
        }}

        .glossary-item {{
            padding: 10px 12px;
            margin-bottom: 6px;
            background: var(--bg-tertiary);
            border-radius: 6px;
            cursor: pointer;
            transition: all 0.1s ease;
        }}

        .glossary-item:hover {{
            background: var(--bg-elevated);
            border-left: 3px solid var(--purple);
            padding-left: 9px;
        }}

        .glossary-item .term {{
            font-weight: 500;
            color: var(--purple);
            font-size: 13px;
            margin-bottom: 4px;
        }}

        .glossary-item .means {{
            font-size: 12px;
            color: var(--text-secondary);
            font-family: ui-monospace, monospace;
        }}

        .legend {{
            background: var(--bg-tertiary);
            border-radius: 6px;
            padding: 12px;
            margin-bottom: 20px;
        }}

        .legend-title {{
            font-size: 11px;
            text-transform: uppercase;
            letter-spacing: 0.5px;
            color: var(--text-muted);
            margin-bottom: 10px;
        }}

        .legend-item {{
            display: flex;
            align-items: center;
            gap: 8px;
            padding: 4px 0;
            font-size: 12px;
        }}

        .legend-color {{
            width: 12px;
            height: 12px;
            border-radius: 3px;
        }}

        .legend-color.glossary {{ background: var(--purple-bg); border: 1px solid var(--purple); }}
        .legend-color.primitive {{ background: var(--green-bg); border: 1px solid var(--green); }}

        /* Popover styles */
        .popover {{
            position: fixed;
            background: var(--bg-elevated);
            border: 1px solid var(--border);
            border-radius: 8px;
            padding: 16px;
            min-width: 300px;
            max-width: 420px;
            box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
            z-index: 1000;
            pointer-events: none;
            opacity: 0;
            transform: translateY(8px);
            transition: opacity 0.15s ease, transform 0.15s ease;
        }}

        .popover.visible {{
            opacity: 1;
            transform: translateY(0);
        }}

        .popover-title {{
            font-size: 14px;
            font-weight: 600;
            color: var(--text-primary);
            margin-bottom: 12px;
            display: flex;
            align-items: center;
            gap: 8px;
        }}

        .popover-badge {{
            font-size: 10px;
            text-transform: uppercase;
            letter-spacing: 0.5px;
            padding: 2px 6px;
            border-radius: 4px;
            font-weight: 500;
            background: var(--purple-bg);
            color: var(--purple);
        }}

        .resolution-chain {{
            font-family: ui-monospace, monospace;
            font-size: 12px;
        }}

        .resolution-step {{
            display: flex;
            align-items: flex-start;
            gap: 8px;
            padding: 6px 0;
            border-left: 2px solid var(--purple);
            padding-left: 12px;
            margin-left: 4px;
        }}

        .step-arrow {{
            color: var(--text-muted);
            flex-shrink: 0;
        }}

        .step-term {{
            color: var(--purple);
        }}

        .step-meaning {{
            color: var(--teal);
        }}
    </style>
</head>
<body>
    <header class="header">
        <div class="logo">
            <img src="data:image/png;base64,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" alt="NTNT">NTNT <span class="ial">IAL</span> Explorer
        </div>
        <a href="/" class="btn">← Back to Studio</a>
    </header>

    <div class="container">
        <div class="content">
            <div class="intro">
                <strong>IAL Explorer</strong> - Visualize how natural language assertions resolve to executable checks.
                Hover over highlighted terms to see their definitions.
            </div>

            <div id="features"></div>
        </div>

        <div class="sidebar">
            <div class="legend">
                <div class="legend-title">Term Types</div>
                <div class="legend-item">
                    <div class="legend-color glossary"></div>
                    <span>Glossary term (resolvable)</span>
                </div>
                <div class="legend-item">
                    <div class="legend-color primitive"></div>
                    <span>Primitive check (executable)</span>
                </div>
            </div>

            <div class="sidebar-title">Glossary Terms (<span id="glossary-count">0</span>)</div>
            <ul class="glossary-list" id="glossary-list"></ul>
        </div>
    </div>

    <!-- Popover element -->
    <div class="popover" id="popover">
        <div class="popover-title">
            <span id="popover-term">Term</span>
            <span class="popover-badge">GLOSSARY</span>
        </div>
        <div class="resolution-chain" id="popover-chain"></div>
    </div>

    <script>
        const glossaryTerms = [{glossary}];
        const features = [{features}];

        // Build glossary lookup (case-insensitive)
        const glossaryLookup = {{}};
        glossaryTerms.forEach((g, idx) => {{
            glossaryLookup[g.term.toLowerCase()] = {{ meaning: g.meaning, idx: idx }};
        }});

        function escapeHtml(str) {{
            if (!str) return '';
            return String(str).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
        }}

        // Highlight glossary terms in text - safer approach
        // Only highlights exact term matches, uses index lookup for meaning
        function highlightTerms(text) {{
            if (!text) return '';

            // Sort terms by length (longest first) to avoid partial matches
            const sortedTerms = [...glossaryTerms].sort((a, b) => b.term.length - a.term.length);

            // Find all matches first
            const matches = [];
            let processedText = text;

            for (const termObj of sortedTerms) {{
                const termLower = termObj.term.toLowerCase();
                const textLower = processedText.toLowerCase();
                let pos = 0;

                while ((pos = textLower.indexOf(termLower, pos)) !== -1) {{
                    // Check word boundaries manually
                    const before = pos > 0 ? textLower[pos - 1] : ' ';
                    const after = pos + termLower.length < textLower.length ? textLower[pos + termLower.length] : ' ';
                    const isWordBoundaryBefore = !/[a-z0-9]/.test(before);
                    const isWordBoundaryAfter = !/[a-z0-9]/.test(after);

                    if (isWordBoundaryBefore && isWordBoundaryAfter) {{
                        matches.push({{
                            start: pos,
                            end: pos + termLower.length,
                            term: termObj.term,
                            original: processedText.substring(pos, pos + termLower.length)
                        }});
                    }}
                    pos += 1;
                }}
            }}

            // Remove overlapping matches (keep first/longest)
            matches.sort((a, b) => a.start - b.start);
            const filteredMatches = [];
            let lastEnd = -1;
            for (const m of matches) {{
                if (m.start >= lastEnd) {{
                    filteredMatches.push(m);
                    lastEnd = m.end;
                }}
            }}

            // Build result
            let result = '';
            let lastPos = 0;
            for (const m of filteredMatches) {{
                result += escapeHtml(text.substring(lastPos, m.start));
                const idx = glossaryLookup[m.term.toLowerCase()]?.idx ?? -1;
                result += `<span class="glossary-term" data-idx="${{idx}}">${{escapeHtml(m.original)}}</span>`;
                lastPos = m.end;
            }}
            result += escapeHtml(text.substring(lastPos));

            return result;
        }}

        // Render features
        const featuresEl = document.getElementById('features');
        features.forEach(feature => {{
            const html = `
                <div class="feature-section">
                    <div class="feature-header">
                        <div class="feature-name">${{escapeHtml(feature.name)}}</div>
                        <div class="feature-id">${{escapeHtml(feature.id)}}</div>
                    </div>
                    <div class="scenarios">
                        ${{feature.scenarios.map(s => `
                            <div class="scenario">
                                <div class="scenario-name">${{escapeHtml(s.name)}}</div>
                                <div class="when-clause">When ${{highlightTerms(s.when)}}</div>
                                <ul class="outcomes">
                                    ${{s.outcomes.map(o => `
                                        <li class="outcome">
                                            <span class="outcome-arrow">→</span>
                                            <span>${{highlightTerms(o)}}</span>
                                        </li>
                                    `).join('')}}
                                </ul>
                            </div>
                        `).join('')}}
                    </div>
                </div>
            `;
            featuresEl.innerHTML += html;
        }});

        // Render glossary sidebar
        const glossaryList = document.getElementById('glossary-list');
        document.getElementById('glossary-count').textContent = glossaryTerms.length;
        glossaryTerms.forEach(g => {{
            const item = document.createElement('li');
            item.className = 'glossary-item';
            item.innerHTML = `
                <div class="term">${{escapeHtml(g.term)}}</div>
                <div class="means">→ ${{escapeHtml(g.meaning)}}</div>
            `;
            glossaryList.appendChild(item);
        }});

        // Popover functionality
        const popover = document.getElementById('popover');
        const popoverTerm = document.getElementById('popover-term');
        const popoverChain = document.getElementById('popover-chain');

        // Set up event handlers for glossary terms (after DOM is built)
        setTimeout(() => {{
            document.querySelectorAll('.glossary-term').forEach(termEl => {{
                termEl.addEventListener('mouseenter', (e) => {{
                    const idx = parseInt(e.target.dataset.idx, 10);
                    const glossaryItem = glossaryTerms[idx];

                    if (glossaryItem) {{
                        popoverTerm.textContent = glossaryItem.term;
                        popoverChain.innerHTML = `
                            <div class="resolution-step">
                                <span class="step-arrow">→</span>
                                <span class="step-term">${{escapeHtml(glossaryItem.term)}}</span>
                            </div>
                            <div class="resolution-step">
                                <span class="step-arrow">→</span>
                                <span class="step-meaning">${{escapeHtml(glossaryItem.meaning)}}</span>
                            </div>
                        `;
                        popover.classList.add('visible');
                    }}
                }});

                termEl.addEventListener('mouseleave', () => {{
                    popover.classList.remove('visible');
                }});

                termEl.addEventListener('mousemove', (e) => {{
                    const x = e.clientX + 15;
                    const y = e.clientY + 10;

                    // Keep popover in viewport
                    const popoverRect = popover.getBoundingClientRect();
                    const maxX = window.innerWidth - popoverRect.width - 20;
                    const maxY = window.innerHeight - popoverRect.height - 20;

                    popover.style.left = Math.min(x, maxX) + 'px';
                    popover.style.top = Math.min(y, maxY) + 'px';
                }});
            }});
        }}, 0);
    </script>
</body>
</html>"##,
        file_name = html_escape(file_path),
        glossary = glossary_json.join(", "),
        features = features_json.join(", ")
    )
}

/// Escape a string for JSON embedding
fn escape_json_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Start the async Intent Studio server
pub async fn start_server(
    state: Arc<StudioState>,
    port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = create_router(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    let listener = tokio::net::TcpListener::bind(addr).await?;

    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("\"quoted\""), "&quot;quoted&quot;");
    }
}
