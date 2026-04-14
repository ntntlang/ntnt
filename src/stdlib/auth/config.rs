use super::*;

pub(super) fn initialize_session_store(store: &SessionStore) -> std::result::Result<(), String> {
    match store {
        SessionStore::Sqlite(path) => init_sqlite_sessions(path),
        SessionStore::Postgres(url) => init_postgres_sessions(url),
        SessionStore::Redis(url) => init_redis_sessions(url),
        SessionStore::Memory => Ok(()),
    }
}

pub fn parse_auth_session_store(value: &str) -> std::result::Result<SessionStore, String> {
    if value == "memory" || value.is_empty() {
        return Ok(SessionStore::Memory);
    }
    if value.starts_with("sqlite:") {
        let path = value.strip_prefix("sqlite:").unwrap_or("./sessions.db");
        let path = if path.is_empty() {
            "./sessions.db"
        } else {
            path
        };
        return Ok(SessionStore::Sqlite(path.to_string()));
    }
    if value.starts_with("postgres://") || value.starts_with("postgresql://") {
        return Ok(SessionStore::Postgres(value.to_string()));
    }
    if value.starts_with("redis://") || value.starts_with("valkey://") {
        return Ok(SessionStore::Redis(value.to_string()));
    }

    Err("session_store must be one of: memory, sqlite:PATH, postgres://..., postgresql://..., redis://..., valkey://...".to_string())
}

pub fn ensure_auth_session_store(config: &AuthConfig) -> std::result::Result<(), String> {
    initialize_session_store(&config.session_store)
}

pub(super) fn auth_option_suggestion(key: &str) -> Option<String> {
    let valid = [
        "session_secret",
        "session_ttl",
        "refresh_ttl",
        "success_url",
        "after_login",
        "failure_url",
        "after_failure",
        "logout_url",
        "after_logout",
        "cookie_name",
        "cookie_secure",
        "session_store",
        "store_tokens",
        "protected_paths",
    ];

    valid
        .iter()
        .map(|candidate| {
            (
                *candidate,
                crate::error::levenshtein_distance(key, candidate),
            )
        })
        .filter(|(_, distance)| *distance <= 4)
        .min_by_key(|(_, distance)| *distance)
        .map(|(candidate, _)| candidate.to_string())
}

/// Initialize auth with config
pub fn init_auth(config: AuthConfig) {
    let is_prod = std::env::var("NTNT_ENV")
        .map(|v| v == "production" || v == "prod")
        .unwrap_or(false);

    // SECURITY: Require secure session_secret in production
    if is_prod && config.session_secret == DEFAULT_SESSION_SECRET_SENTINEL {
        eprintln!("┌─────────────────────────────────────────────────────────────────┐");
        eprintln!("│ FATAL: Cannot use default session_secret in production!        │");
        eprintln!("│                                                                 │");
        eprintln!("│ Set a secure random secret in enable_auth():                   │");
        eprintln!("│   enable_auth([...], map {{ \"session_secret\": get_env(\"SECRET\") }}) │");
        eprintln!("│                                                                 │");
        eprintln!("│ Generate a secret: openssl rand -base64 32                      │");
        eprintln!("└─────────────────────────────────────────────────────────────────┘");
        std::process::exit(1);
    }

    // In dev mode with no explicit secret, use auto-generated random secret
    let config = if !is_prod && config.session_secret == DEFAULT_SESSION_SECRET_SENTINEL {
        eprintln!(
            "[auth] Using auto-generated session secret (sessions won't persist across restarts)"
        );
        eprintln!("       Set session_secret in enable_auth() for production.");
        let mut config = config;
        config.session_secret = dev_session_secret().to_string();
        config
    } else {
        config
    };

    // Log session storage type
    match &config.session_store {
        SessionStore::Memory => {
            eprintln!("[auth] Using in-memory session storage");
            eprintln!("       Sessions will be lost on server restart.");
            if is_prod {
                eprintln!("       WARNING: Running in production without persistent storage!");
            }
            // Start background cleanup task for expired sessions (every 5 minutes)
            let store = SESSION_STORE.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(300));
                if let Ok(mut s) = store.lock() {
                    let now = chrono::Utc::now().timestamp();
                    let removed_sessions = s.cleanup_expired(now);
                    let removed_challenges = s.cleanup_expired_auth_challenges(now);
                    s.cleanup_expired_oauth_states(now - 600);
                    s.cleanup_expired_exchange_tokens(now);
                    if removed_sessions > 0 {
                        eprintln!("[auth] Cleaned up {} expired session(s)", removed_sessions);
                    }
                    if removed_challenges > 0 {
                        eprintln!(
                            "[auth] Cleaned up {} expired auth challenge(s)",
                            removed_challenges
                        );
                    }
                }
            });
        }
        SessionStore::Sqlite(path) => {
            eprintln!("[auth] Using SQLite session storage: {}", path);
        }
        SessionStore::Postgres(_url) => {
            eprintln!("[auth] Using PostgreSQL session storage");
            // Don't log connection URL (may contain password)
        }
        SessionStore::Redis(url) => {
            let backend = if url.starts_with("valkey://") {
                "Valkey"
            } else {
                "Redis"
            };
            eprintln!("[auth] Using {} session storage", backend);
            // Don't log connection URL (may contain password)
        }
    }

    let mut auth_config = AUTH_CONFIG.lock().unwrap();
    *auth_config = Some(config.clone());

    reset_protected_paths();
    if !config.protected_paths.is_empty() {
        register_protected_paths(&config.protected_paths);
    }
}

/// Initialize SQLite session storage
fn init_sqlite_sessions(path: &str) -> std::result::Result<(), String> {
    let conn =
        rusqlite::Connection::open(path).map_err(|e| format!("Failed to open SQLite: {}", e))?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS auth_sessions (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            provider TEXT NOT NULL,
            email TEXT,
            name TEXT,
            picture TEXT,
            raw_json TEXT NOT NULL,
            data_json TEXT NOT NULL,
            csrf_token TEXT NOT NULL DEFAULT '',
            access_token TEXT,
            refresh_token TEXT,
            token_expires_at INTEGER,
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| format!("Failed to create sessions table: {}", e))?;

    // Index for cleanup queries
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_expires ON auth_sessions(expires_at)",
        [],
    )
    .map_err(|e| format!("Failed to create index: {}", e))?;

    // OAuth state table for CSRF protection
    conn.execute(
        "CREATE TABLE IF NOT EXISTS auth_oauth_states (
            state TEXT PRIMARY KEY,
            nonce TEXT,
            pkce_verifier TEXT,
            provider TEXT NOT NULL,
            redirect_url TEXT NOT NULL,
            created_at INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| format!("Failed to create oauth_states table: {}", e))?;

    // Exchange token table for Safari ITP cookie workaround
    conn.execute(
        "CREATE TABLE IF NOT EXISTS auth_exchange_tokens (
            token TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            created_at INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| format!("Failed to create exchange_tokens table: {}", e))?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS auth_challenges (
            id TEXT PRIMARY KEY,
            subject_id TEXT NOT NULL,
            provider TEXT NOT NULL,
            kind TEXT NOT NULL,
            data_json TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| format!("Failed to create auth_challenges table: {}", e))?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_auth_challenges_expires ON auth_challenges(expires_at)",
        [],
    )
    .map_err(|e| format!("Failed to create auth_challenges index: {}", e))?;

    let mut sqlite_conn = SQLITE_CONN.lock().unwrap();
    *sqlite_conn = Some(conn);
    Ok(())
}

/// Initialize PostgreSQL session storage
fn init_postgres_sessions(url: &str) -> std::result::Result<(), String> {
    // Test connection and create table
    let mut client = postgres::Client::connect(url, postgres::NoTls)
        .map_err(|e| format!("Failed to connect to PostgreSQL: {}", e))?;

    client
        .execute(
            "CREATE TABLE IF NOT EXISTS auth_sessions (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            provider TEXT NOT NULL,
            email TEXT,
            name TEXT,
            picture TEXT,
            raw_json TEXT NOT NULL,
            data_json TEXT NOT NULL,
            csrf_token TEXT NOT NULL DEFAULT '',
            access_token TEXT,
            refresh_token TEXT,
            token_expires_at BIGINT,
            created_at BIGINT NOT NULL,
            expires_at BIGINT NOT NULL
        )",
            &[],
        )
        .map_err(|e| format!("Failed to create sessions table: {}", e))?;

    // Index for cleanup queries
    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_sessions_expires ON auth_sessions(expires_at)",
            &[],
        )
        .ok(); // Ignore if already exists

    // OAuth state table for CSRF protection
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS auth_oauth_states (
            state TEXT PRIMARY KEY,
            nonce TEXT,
            pkce_verifier TEXT,
            provider TEXT NOT NULL,
            redirect_url TEXT NOT NULL,
            created_at BIGINT NOT NULL
        )",
            &[],
        )
        .map_err(|e| format!("Failed to create oauth_states table: {}", e))?;

    // Exchange token table for Safari ITP cookie workaround
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS auth_exchange_tokens (
            token TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            created_at BIGINT NOT NULL
        )",
            &[],
        )
        .map_err(|e| format!("Failed to create exchange_tokens table: {}", e))?;

    client
        .execute(
            "CREATE TABLE IF NOT EXISTS auth_challenges (
            id TEXT PRIMARY KEY,
            subject_id TEXT NOT NULL,
            provider TEXT NOT NULL,
            kind TEXT NOT NULL,
            data_json TEXT NOT NULL,
            created_at BIGINT NOT NULL,
            expires_at BIGINT NOT NULL
        )",
            &[],
        )
        .map_err(|e| format!("Failed to create auth_challenges table: {}", e))?;

    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_auth_challenges_expires ON auth_challenges(expires_at)",
            &[],
        )
        .ok();

    // Store URL for later connections
    let mut pg_url = POSTGRES_URL.lock().unwrap();
    *pg_url = Some(url.to_string());
    Ok(())
}

/// Initialize Redis session storage
fn init_redis_sessions(url: &str) -> std::result::Result<(), String> {
    // Convert valkey:// to redis:// for the redis crate
    let redis_url = if url.starts_with("valkey://") {
        url.replacen("valkey://", "redis://", 1)
    } else {
        url.to_string()
    };

    // Test connection
    let client = redis::Client::open(redis_url.as_str())
        .map_err(|e| format!("Failed to create Redis client: {}", e))?;

    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Failed to connect to Redis: {}", e))?;

    // Test with a PING
    let _: String = redis::cmd("PING")
        .query(&mut conn)
        .map_err(|e| format!("Redis PING failed: {}", e))?;

    // Store URL for later connections
    let mut redis_url_store = REDIS_URL.lock().unwrap();
    *redis_url_store = Some(redis_url);
    Ok(())
}
