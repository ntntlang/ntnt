//! Auth route protection over the real async `ntnt run` server.
//!
//! - The public exemption under the auth `route_prefix` covers std/auth's own
//!   handlers only. A custom handler sharing that prefix gets the protection
//!   the app asked for: `use_middleware(require_auth())` or a protected path.
//! - Auth decides on the canonical path the router matches, so repeated or
//!   trailing slashes cannot select a protected handler while missing its rule.
#[path = "support/system_fixture.rs"]
mod fixture;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};

/// Where the auth guard sends a browser that has no session.
const LOGIN_PAGE: &str = "/auth";
const GITHUB_AUTHORIZE: &str = "https://github.com/login/oauth/authorize?";

/// Every request goes through `use_middleware(require_auth())`. The app wires
/// std/auth's convenience handlers itself (as `examples/auth_demo` does) and
/// registers `/auth/profile` before `enable_auth()`, so the router selects
/// these handlers rather than the built-in `{provider}` routes.
const REQUIRE_AUTH_APP: &str = r#"
import { auth_callback, auth_me, auth_start, enable_auth, oauth, require_auth } from "std/auth"
import { text } from "std/http/server"

fn profile(req: Request) -> Response { return text("custom-profile") }
fn settings(req: Request) -> Response { return text("custom-settings") }
fn revoke(req: Request) -> Response { return text("custom-revoke") }

get("/auth/profile", profile)
get("/auth/me", auth_me)
get("/auth/{provider}", auth_start)
get("/auth/{provider}/callback", auth_callback)
enable_auth([oauth("github", "fixture-client", "fixture-secret")], map { "session_secret": "fixture-session-secret-0123456789abcdef" })
use_middleware(require_auth())
get("/auth/account/settings", settings)
post("/auth/sessions/revoke", revoke)
listen(0, map { "fixture": true, "readiness": "json" })
"#;

/// Protection through path rules (`protected_paths` and `require_auth(path)`),
/// with the built-in auth routes and a public `/login` that signs in.
const PROTECTED_PATHS_APP: &str = r#"
import { enable_auth, oauth, require_auth, sign_in_session } from "std/auth"
import { redirect, text } from "std/http/server"

fn login(req: Request) -> Response { return sign_in_session(redirect("/public"), req, map { "subject_id": "fixture-user" }) }
fn public_page(req: Request) -> Response { return text("public-page") }
fn admin_home(req: Request) -> Response { return text("admin-home") }
fn admin_panel(req: Request) -> Response { return text("admin-panel") }
fn daily_report(req: Request) -> Response { return text("daily-report") }
fn auth_admin_panel(req: Request) -> Response { return text("auth-admin-panel") }

enable_auth([oauth("github", "fixture-client", "fixture-secret")], map { "session_secret": "fixture-session-secret-0123456789abcdef", "protected_paths": ["/auth/*"] })
require_auth("/admin/*")
require_auth("/reports/daily")
get("/login", login)
get("/public", public_page)
get("/admin", admin_home)
get("/admin/panel", admin_panel)
get("/reports/daily", daily_report)
get("/auth/admin/panel", auth_admin_panel)
listen(0, map { "fixture": true, "readiness": "json" })
"#;

struct Reply {
    status: u16,
    head: String,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }

    fn location(&self) -> Option<&str> {
        self.header("location")
    }
}

impl std::fmt::Display for Reply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\n\n{}", self.head, self.body)
    }
}

/// One raw HTTP/1.1 exchange, so the request target reaches the server exactly
/// as written (`//admin` is not rewritten by a client URL parser).
fn send(addr: SocketAddr, method: &str, target: &str, headers: &[(&str, &str)]) -> Reply {
    let mut stream = TcpStream::connect_timeout(&addr, fixture::HANG_GUARD).unwrap();
    stream.set_read_timeout(Some(fixture::HANG_GUARD)).unwrap();
    stream.set_write_timeout(Some(fixture::HANG_GUARD)).unwrap();
    let mut request = format!(
        "{method} {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: 0\r\n"
    );
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).unwrap();
    let mut raw = Vec::new();
    stream.take(1 << 20).read_to_end(&mut raw).unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|status| status.parse().ok())
        .unwrap_or(0);
    Reply {
        status,
        head: head.to_string(),
        body: body.to_string(),
    }
}

/// The guard's denial for a browser: a redirect to the login page, and the
/// protected handler's output never appears.
fn assert_sent_to_login(reply: &Reply, request: &str, handler_output: &str) {
    assert_eq!(
        (reply.status, reply.location()),
        (302, Some(LOGIN_PAGE)),
        "{request} must require a session: {reply}"
    );
    assert!(
        !reply.body.contains(handler_output),
        "{request} ran the protected handler: {reply}"
    );
}

/// Sign in through the app's public `/login` route and return the cookie pair.
fn session_cookie(addr: SocketAddr) -> String {
    let reply = send(addr, "GET", "/login", &[]);
    assert_eq!(reply.status, 302, "login: {reply}");
    let cookie = reply.header("set-cookie").expect("login sets a cookie");
    let pair = cookie.split(';').next().unwrap().trim().to_string();
    assert!(pair.starts_with("ntnt_session="), "{cookie}");
    pair
}

#[test]
fn require_auth_middleware_protects_custom_handlers_under_the_auth_prefix() {
    let mut server = fixture::Fixture::start(REQUIRE_AUTH_APP, &["run"], &[]);
    let addr = server.ready();
    for (method, target, output) in [
        ("GET", "/auth/profile", "custom-profile"),
        ("GET", "/auth/account/settings", "custom-settings"),
        ("GET", "//auth/account/settings", "custom-settings"),
        ("POST", "/auth/sessions/revoke", "custom-revoke"),
    ] {
        let reply = send(addr, method, target, &[]);
        assert_sent_to_login(&reply, &format!("{method} {target}"), output);
    }
    // An API client gets the guard's JSON 401 instead of a redirect.
    let reply = send(
        addr,
        "POST",
        "/auth/sessions/revoke",
        &[("Accept", "application/json")],
    );
    assert_eq!(reply.status, 401, "{reply}");
    assert!(reply.body.contains("Authentication required"), "{reply}");
    assert!(!reply.body.contains("custom-revoke"), "{reply}");
}

#[test]
fn require_auth_middleware_keeps_std_auth_handlers_public() {
    let mut server = fixture::Fixture::start(REQUIRE_AUTH_APP, &["run"], &[]);
    let addr = server.ready();
    // Built-in login page: one provider, so it forwards to that provider.
    let reply = send(addr, "GET", "/auth", &[]);
    assert_eq!(
        (reply.status, reply.location()),
        (302, Some("/auth/github")),
        "{reply}"
    );
    // The app-wired convenience handlers start and finish the OAuth flow.
    let reply = send(addr, "GET", "/auth/github", &[]);
    assert_eq!(reply.status, 302, "{reply}");
    assert!(
        reply
            .location()
            .is_some_and(|to| to.starts_with(GITHUB_AUTHORIZE)),
        "{reply}"
    );
    let reply = send(addr, "GET", "/auth/github/callback", &[]);
    assert_eq!(
        (reply.status, reply.location()),
        (302, Some("/")),
        "{reply}"
    );
    // auth_me answers an anonymous caller itself.
    let reply = send(addr, "GET", "/auth/me", &[]);
    assert_eq!(reply.status, 401, "{reply}");
    assert!(reply.body.contains("Not authenticated"), "{reply}");
    // Built-in logout.
    let reply = send(addr, "POST", "/auth/logout", &[]);
    assert_eq!(
        (reply.status, reply.location()),
        (302, Some("/")),
        "{reply}"
    );
    assert!(reply.header("set-cookie").is_some(), "{reply}");
}

#[test]
fn protected_path_rule_protects_custom_handler_under_the_auth_prefix() {
    let mut server = fixture::Fixture::start(PROTECTED_PATHS_APP, &["run"], &[]);
    let addr = server.ready();
    let spellings = [
        "/auth/admin/panel",
        "//auth/admin/panel",
        "/auth//admin/panel",
    ];
    for target in spellings {
        let reply = send(addr, "GET", target, &[]);
        assert_sent_to_login(&reply, &format!("GET {target}"), "auth-admin-panel");
    }

    let cookie = session_cookie(addr);
    for target in spellings {
        let reply = send(addr, "GET", target, &[("Cookie", &cookie)]);
        assert_eq!(reply.status, 200, "GET {target} with a session: {reply}");
        assert!(reply.body.contains("auth-admin-panel"), "{reply}");
    }

    // `/auth/*` also covers the built-in auth routes; they stay public.
    let reply = send(addr, "GET", "/auth", &[]);
    assert_eq!(
        (reply.status, reply.location()),
        (302, Some("/auth/github")),
        "{reply}"
    );
    let reply = send(addr, "GET", "/auth/github", &[]);
    assert!(
        reply.status == 302
            && reply
                .location()
                .is_some_and(|to| to.starts_with(GITHUB_AUTHORIZE)),
        "{reply}"
    );
    let reply = send(addr, "GET", "/auth/github/callback", &[]);
    assert_eq!(
        (reply.status, reply.location()),
        (302, Some("/")),
        "{reply}"
    );
    let reply = send(addr, "GET", "/auth/health", &[]);
    assert_eq!(reply.status, 200, "{reply}");
    assert!(reply.body.contains("\"ok\":true"), "{reply}");
    let reply = send(addr, "POST", "/auth/logout", &[]);
    assert_eq!(
        (reply.status, reply.location()),
        (302, Some("/")),
        "{reply}"
    );
}

#[test]
fn protected_path_rules_match_the_path_the_router_serves() {
    let mut server = fixture::Fixture::start(PROTECTED_PATHS_APP, &["run"], &[]);
    let addr = server.ready();
    let spellings = [
        // `/admin/*`: leading repeated slashes, interior and trailing ones.
        ("//admin", "admin-home"),
        ("//admin/", "admin-home"),
        ("//admin/panel", "admin-panel"),
        ("/admin//panel", "admin-panel"),
        ("/admin/panel/", "admin-panel"),
        ("/admin/panel", "admin-panel"),
        // `/reports/daily` (exact rule).
        ("/reports/daily", "daily-report"),
        ("/reports/daily/", "daily-report"),
        ("/reports//daily", "daily-report"),
        ("//reports/daily", "daily-report"),
        ("/reports//daily//", "daily-report"),
    ];
    for (target, output) in spellings {
        let reply = send(addr, "GET", target, &[]);
        assert_sent_to_login(&reply, &format!("GET {target}"), output);
    }

    let cookie = session_cookie(addr);
    for (target, output) in spellings {
        let reply = send(addr, "GET", target, &[("Cookie", &cookie)]);
        assert_eq!(reply.status, 200, "GET {target} with a session: {reply}");
        assert!(reply.body.contains(output), "GET {target}: {reply}");
    }

    // Unprotected paths stay public in every spelling.
    for target in ["/public", "//public", "/public/"] {
        let reply = send(addr, "GET", target, &[]);
        assert_eq!(reply.status, 200, "GET {target}: {reply}");
        assert!(reply.body.contains("public-page"), "GET {target}: {reply}");
    }
}

#[test]
fn auth_exemption_uses_the_path_the_router_serves() {
    // `//auth/github` selects the app-wired auth_start, like `/auth/github`.
    let mut server = fixture::Fixture::start(REQUIRE_AUTH_APP, &["run"], &[]);
    let addr = server.ready();
    let reply = send(addr, "GET", "//auth/github", &[]);
    assert_eq!(reply.status, 302, "{reply}");
    assert!(
        reply
            .location()
            .is_some_and(|to| to.starts_with(GITHUB_AUTHORIZE)),
        "{reply}"
    );
}
