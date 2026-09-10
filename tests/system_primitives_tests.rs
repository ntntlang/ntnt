use ntnt::interpreter::Value;
use std::collections::HashMap;
fn call(module: &HashMap<String, Value>, name: &str, args: Vec<Value>) -> Value {
    let Value::NativeFunction { func, .. } = &module[name] else {
        panic!("native")
    };
    func(&args).unwrap()
}
fn ok(value: Value) -> Value {
    match value {
        Value::EnumValue {
            variant,
            mut values,
            ..
        } if variant == "Ok" => values.remove(0),
        other => panic!("expected Ok, got {other:?}"),
    }
}
fn err(value: Value) -> String {
    match value {
        Value::EnumValue {
            variant,
            mut values,
            ..
        } if variant == "Err" => values.remove(0).to_string(),
        other => panic!("expected Err, got {other:?}"),
    }
}
fn text(s: impl Into<String>) -> Value {
    Value::String(s.into())
}
fn bytes(b: &[u8]) -> Value {
    Value::Array(b.iter().map(|b| Value::Int(i64::from(*b))).collect())
}
#[test]
fn sha384_vectors_bytes_and_pinned_htmx() {
    let m = ntnt::stdlib::crypto::init();
    for (input, expected) in [
        (b"".as_slice(), "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b"),
        (b"abc".as_slice(), "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"),
        (&[0,159,146,150,255], "a7f04a3e3c7a6d8ae443807146c5aece796c330e72d93c8ae09889c6b90a07037f2df6addf0f45207ad8943a464e8380"),
        (b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnohijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu".as_slice(), "aacb8012d4d9427e87dd40183dec88ab0e1cf7fe25a95fcba678ed2404aa0ad241112bfeea0351cd8d64f48190231ce5"),
    ] {
        assert_eq!(call(&m,"sha384",vec![bytes(input)]).to_string(), expected);
        let Value::Array(raw) = call(&m,"sha384_bytes",vec![bytes(input)]) else { panic!("bytes") };
        assert_eq!(raw.len(),48);
        assert_eq!(raw.iter().map(|v| if let Value::Int(n)=v { format!("{n:02x}") } else { panic!("byte") }).collect::<String>(),expected);
    }
    let input = include_bytes!("fixtures/system-primitives/htmx-2.0.10.min.js");
    assert_eq!(input.len(), 51238);
    let digest = call(&m, "sha384_bytes", vec![bytes(input)]);
    let sri = call(&m, "base64_encode_bytes", vec![digest]).to_string();
    assert_eq!(
        sri,
        "H5SrcfygHmAuTDZphMHqBJLc3FhssKjG7w/CeCpFReSfwBWDTKpkzPP8c+cLsK+V"
    );
    let mut changed = input.to_vec();
    changed[0] ^= 1;
    assert_ne!(
        call(
            &m,
            "base64_encode_bytes",
            vec![call(&m, "sha384_bytes", vec![bytes(&changed)])]
        )
        .to_string(),
        sri
    );
    assert_eq!(
        call(&m, "sha384", vec![text("abc")]).to_string(),
        call(&m, "sha384", vec![bytes(b"abc")]).to_string()
    );
    for name in ["sha384", "sha384_bytes", "base64_encode_bytes"] {
        let Value::NativeFunction { func, .. } = &m[name] else {
            panic!()
        };
        for bad in [
            Value::Int(-1),
            Value::Int(256),
            Value::Float(1.0),
            Value::Bool(true),
            text("0"),
        ] {
            assert!(func(&[Value::Array(vec![bad])]).is_err(), "{name}");
        }
    }
    assert_eq!(
        call(&m, "base64_encode_bytes", vec![bytes(&[0, 255])]).to_string(),
        "AP8="
    );
    assert_eq!(
        call(&m, "base64_encode", vec![text("abc")]).to_string(),
        "YWJj"
    );
    assert_eq!(
        call(&m, "sha256_bytes", vec![text("abc")]).type_name(),
        "Array"
    );
}
#[test]
fn byte_write_validation_preserves_existing_contents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data");
    let p = text(path.to_str().unwrap());
    let m = ntnt::stdlib::fs::init();
    ok(call(
        &m,
        "write_bytes",
        vec![p.clone(), bytes(&[0, 255, 159])],
    ));
    assert_eq!(std::fs::read(&path).unwrap(), [0, 255, 159]);
    for bad in [
        Value::Int(-1),
        Value::Int(256),
        Value::Float(1.0),
        text("1"),
    ] {
        assert!(err(call(
            &m,
            "write_bytes",
            vec![p.clone(), Value::Array(vec![bad])]
        ))
        .starts_with("invalid_argument:"));
        assert_eq!(std::fs::read(&path).unwrap(), [0, 255, 159]);
    }
    assert!(
        err(call(&m, "write_bytes", vec![text("bad\0path"), bytes(&[])]))
            .starts_with("invalid_argument:")
    );
}
#[cfg(unix)]
#[test]
fn secure_files_metadata_symlinks_access_and_publication() {
    use std::os::unix::fs::{symlink, MetadataExt};
    let dir = tempfile::tempdir().unwrap();
    let m = ntnt::stdlib::fs::init();
    let path = dir.path().join("temp");
    let p = text(path.to_str().unwrap());
    let bad = Value::Map(HashMap::from([("mode".into(), Value::Int(512))]));
    assert!(err(call(
        &m,
        "write_file_exclusive",
        vec![p.clone(), text("secret"), bad]
    ))
    .starts_with("invalid_argument:"));
    assert!(!path.exists());
    ok(call(
        &m,
        "write_file_exclusive",
        vec![p.clone(), bytes(&[0, 255])],
    ));
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o077, 0);
    assert!(err(call(
        &m,
        "write_file_exclusive",
        vec![p.clone(), text("replace")]
    ))
    .starts_with("already_exists:"));
    assert_eq!(std::fs::read(&path).unwrap(), [0, 255]);
    for target in [&path, &dir.path().join("missing")] {
        let link = dir.path().join("link");
        symlink(target, &link).unwrap();
        let l = text(link.to_str().unwrap());
        assert!(err(call(
            &m,
            "write_file_exclusive",
            vec![l.clone(), text("replace")]
        ))
        .starts_with("already_exists:"));
        err(call(&m, "sync_file", vec![l]));
        std::fs::remove_file(link).unwrap();
    }
    ok(call(&m, "chmod", vec![p.clone(), Value::Int(0o640)]));
    let meta = std::fs::metadata(&path).unwrap();
    assert_eq!(meta.mode() & 0o777, 0o640);
    ok(call(
        &m,
        "chown",
        vec![
            p.clone(),
            Value::Int(i64::from(meta.uid())),
            Value::Int(i64::from(meta.gid())),
        ],
    ));
    let Value::Map(info) = ok(call(&m, "file_permissions", vec![p.clone()])) else {
        panic!()
    };
    assert_eq!(info["uid"].to_string(), meta.uid().to_string());
    assert_eq!(
        ok(call(&m, "access", vec![p.clone(), text("r")])).to_string(),
        "true"
    );
    err(call(&m, "access", vec![p.clone(), text("rr")]));
    err(call(
        &m,
        "chown",
        vec![p.clone(), Value::Int(-1), Value::Int(0)],
    ));
    ok(call(&m, "sync_file", vec![p.clone()]));
    let published = dir.path().join("published");
    std::fs::rename(&path, &published).unwrap();
    ok(call(
        &m,
        "sync_dir",
        vec![text(dir.path().to_str().unwrap())],
    ));
    assert_eq!(std::fs::read(published).unwrap(), [0, 255]);
    let private = dir.path().join("private");
    let q = text(private.to_str().unwrap());
    ok(call(&m, "mkdir_private", vec![q.clone()]));
    assert_eq!(std::fs::metadata(&private).unwrap().mode() & 0o077, 0);
    err(call(&m, "mkdir_private", vec![q.clone()]));
    err(call(&m, "sync_file", vec![q]));
    err(call(
        &m,
        "mkdir_private",
        vec![text(dir.path().join("absent/child").to_str().unwrap())],
    ));
    let fifo = dir.path().join("fifo");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    err(call(&m, "sync_file", vec![text(fifo.to_str().unwrap())]));
}
#[cfg(unix)]
#[test]
fn missing_resolution_preserves_symlink_order_and_rejects_loops() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("real")).unwrap();
    std::fs::create_dir(root.join("real/inner")).unwrap();
    symlink("real/inner", root.join("link")).unwrap();
    symlink("loop", root.join("loop")).unwrap();
    let m = ntnt::stdlib::path::init();
    for (input, expected) in [
        ("link/../missing", "real/missing"),
        ("missing/../link/tail", "real/inner/tail"),
        ("link/tail", "real/inner/tail"),
    ] {
        assert_eq!(
            ok(call(
                &m,
                "resolve_missing",
                vec![text(root.join(input).to_str().unwrap())]
            ))
            .to_string(),
            root.join(expected).to_str().unwrap()
        );
    }
    err(call(
        &m,
        "resolve_missing",
        vec![text(root.join("loop/a").to_str().unwrap())],
    ));
    std::fs::write(root.join("file"), b"x").unwrap();
    for suffix in ["file/child", "file/../tail", "file/."] {
        err(call(
            &m,
            "resolve_missing",
            vec![text(root.join(suffix).to_str().unwrap())],
        ));
    }
    err(call(
        &m,
        "resolve",
        vec![text(root.join("absent").to_str().unwrap())],
    ));
}

#[cfg(unix)]
#[test]
fn exclusive_competing_creators_have_exactly_one_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("winner");
    let workers=(0..2).map(|_| {let path=path.clone();std::thread::spawn(move || {
        let m=ntnt::stdlib::fs::init();matches!(call(&m,"write_file_exclusive",vec![text(path.to_str().unwrap()),text("winner")]),Value::EnumValue { variant, .. } if variant=="Ok")
    })}).collect::<Vec<_>>();
    let successes = workers
        .into_iter()
        .map(|w| usize::from(w.join().unwrap()))
        .sum::<usize>();
    assert_eq!(successes, 1);
    assert_eq!(std::fs::read(path).unwrap(), b"winner");
}
#[cfg(not(unix))]
#[test]
fn unsupported_private_operations_leave_no_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing");
    let p = text(path.to_str().unwrap());
    let m = ntnt::stdlib::fs::init();
    for (name, args) in [
        ("write_file_exclusive", vec![p.clone(), text("secret")]),
        ("mkdir_private", vec![p.clone()]),
        ("chmod", vec![p.clone(), Value::Int(384)]),
        ("chown", vec![p.clone(), Value::Int(1), Value::Int(1)]),
        ("access", vec![p.clone(), text("r")]),
        ("file_permissions", vec![p.clone()]),
        ("sync_dir", vec![p]),
    ] {
        assert!(err(call(&m, name, args)).starts_with("unsupported:"));
        assert!(!path.exists());
    }
}
#[path = "support/system_fixture.rs"]
mod fixture;
#[test]
fn strict_native_crypto_and_file_surface() {
    let source = include_str!("../examples/system-primitives/files.tnt");
    let out = fixture::strict_run(source);
    assert!(out.contains("SYSTEM_FILES_OK"));
}

#[cfg(unix)]
#[test]
fn initial_modes_obey_inherited_umask_in_an_exec_child() {
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("private");
    let directory = dir.path().join("directory");
    let source=format!("import {{ write_file_exclusive, mkdir_private }} from \"std/fs\"\nunwrap(write_file_exclusive({}, \"bytes\", map {{ \"mode\": 511 }}))\nunwrap(mkdir_private({}, 511))",serde_json::to_string(file.to_str().unwrap()).unwrap(),serde_json::to_string(directory.to_str().unwrap()).unwrap());
    let script = dir.path().join("case.tnt");
    std::fs::write(&script, source).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_ntnt"));
    command
        .args(["run", script.to_str().unwrap()])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Child-only, before exec; implementation never reads or changes global umask.
    unsafe {
        command.pre_exec(|| {
            libc::umask(0o077);
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() >= end {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("umask child deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert!(status.success());
    assert_eq!(std::fs::metadata(file).unwrap().mode() & 0o777, 0o700);
    assert_eq!(std::fs::metadata(directory).unwrap().mode() & 0o777, 0o700);
}

#[cfg(unix)]
#[test]
fn invalid_options_and_oversized_text_have_no_creation_effect() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new");
    let p = text(path.to_str().unwrap());
    let m = ntnt::stdlib::fs::init();
    for options in [
        Value::Bool(true),
        Value::Map(HashMap::from([("unknown".into(), Value::Bool(true))])),
        Value::Map(HashMap::from([("sync".into(), Value::Int(1))])),
        Value::Map(HashMap::from([("mode".into(), Value::Int(-1))])),
    ] {
        assert!(err(call(
            &m,
            "write_file_exclusive",
            vec![p.clone(), text("content"), options]
        ))
        .starts_with("invalid_argument:"));
        assert!(!path.exists());
    }
    err(call(
        &m,
        "write_file_exclusive",
        vec![p.clone(), text("x".repeat(16 * 1024 * 1024 + 1))],
    ));
    assert!(!path.exists());
    assert_eq!(
        ok(call(&m, "access", vec![p.clone(), text("")])).to_string(),
        "false"
    );
    let published = dir.path().join("published");
    ok(call(&m, "write_file_exclusive", vec![p, text("published")]));
    std::fs::rename(&path, &published).unwrap();
    err(call(
        &m,
        "sync_dir",
        vec![text(published.to_str().unwrap())],
    ));
    assert_eq!(std::fs::read(published).unwrap(), b"published");
}

#[cfg(unix)]
#[test]
fn missing_suffix_dangling_links_and_expansion_limit() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let m = ntnt::stdlib::path::init();
    symlink("absent/tail", root.join("relative")).unwrap();
    symlink(root.join("absolute-target"), root.join("absolute")).unwrap();
    for (link, target) in [
        ("relative", "absent/tail/child"),
        ("absolute", "absolute-target/child"),
    ] {
        assert_eq!(
            ok(call(
                &m,
                "resolve_missing",
                vec![text(root.join(link).join("child").to_str().unwrap())]
            ))
            .to_string(),
            root.join(target).to_str().unwrap()
        );
    }
    for i in 0..41 {
        symlink(format!("link{}", i + 1), root.join(format!("link{i}"))).unwrap();
    }
    assert!(err(call(
        &m,
        "resolve_missing",
        vec![text(root.join("link0").to_str().unwrap())]
    ))
    .contains("limit"));
    err(call(&m, "resolve_missing", vec![text("x".repeat(65537))]));
    err(call(&m, "resolve_missing", vec![text("nul\0")]));
}
