//! `serve_static(prefix, dir, map { "extensions": [...] })` over the real servers:
//! the async production server and the sync test-mode server must both answer
//! 404 for files outside the allowlist, while unlisted mounts keep serving everything.
#[path = "support/system_fixture.rs"]
mod fixture;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

fn request(addr: SocketAddr, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = Vec::new();
    stream.take(1 << 20).read_to_end(&mut raw).unwrap();
    let text = String::from_utf8_lossy(&raw).to_string();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, text)
}

fn populate(public: &std::path::Path) {
    fs::create_dir_all(public.join("img")).unwrap();
    for (name, body) in [
        ("site.css", "body{}"),
        ("img/photo.WEBP", "RIFF"),
        ("doc.pdf", "%PDF"),
        ("run.sh", "#!/bin/sh"),
        ("evil.tnt", "print(1)"),
        ("setup.exe", "MZ"),
        ("index.html", "<h1>no</h1>"),
        ("README", "text"),
        (".env", "SECRET=1"),
    ] {
        fs::write(public.join(name), body).unwrap();
    }
}

const SOURCE: &str = r#"
import { text } from "std/http/server"
get("/health", fn(req) { text("ok") })
serve_static("/assets", "./public", map { "extensions": ["css", "webp", "pdf"] })
serve_static("/open", "./public")
listen(0, map { "fixture": true, "readiness": "json" })
"#;

#[test]
fn allowlist_enforced_by_async_server_in_development_and_production() {
    for env in [
        vec![],
        vec![("NTNT_ENV", "production"), ("NTNT_WORKERS", "2")],
    ] {
        let mut server = fixture::Fixture::start(SOURCE, &["run"], &env);
        populate(&server.dir.path().join("public"));
        let addr = server.ready();
        for (path, expected) in [
            ("/assets/site.css", 200),
            ("/assets/img/photo.WEBP", 200),
            ("/assets/doc.pdf", 200),
            ("/assets/run.sh", 404),
            ("/assets/evil.tnt", 404),
            ("/assets/setup.exe", 404),
            ("/assets/index.html", 404),
            ("/assets/README", 404),
            ("/assets/.env", 404),
            ("/assets/", 404),
            ("/open/run.sh", 200),
            ("/open/.env", 200),
            ("/health", 200),
        ] {
            let (status, text) = request(addr, path);
            assert_eq!(status, expected, "{env:?} {path}: {text}");
            if path == "/assets/.env" || path == "/assets/run.sh" {
                assert!(
                    !text.contains("SECRET=1") && !text.contains("#!/bin/sh"),
                    "{path} leaked body"
                );
            }
        }
        drop(server);
    }
}

#[test]
fn invalid_options_fail_at_registration() {
    for (options, needle) in [
        (r#"map { "extensions": [] }"#, "at least one"),
        (r#"map { "extensions": "css" }"#, "array of strings"),
        (r#"map { "extensions": ["c/ss"] }"#, "alphanumeric"),
        (r#"map { "typo": ["css"] }"#, "invalid serve_static option"),
        // The strict typechecker rejects a non-Map third argument before runtime.
        (r#""css""#, "expected Map"),
    ] {
        let source = format!(
            "import {{ text }} from \"std/http/server\"\nget(\"/\", fn(req) {{ text(\"x\") }})\nserve_static(\"/a\", \"./public\", {options})\nlisten(0, map {{ \"fixture\": true, \"readiness\": \"json\" }})\n"
        );
        let mut server = fixture::Fixture::start(&source, &["run"], &[]);
        let status = server.wait();
        assert!(!status.success(), "{options} should fail");
        let err = format!("{}{}", server.stdout(), server.stderr());
        assert!(err.contains(needle), "{options}: {err}");
    }
}
