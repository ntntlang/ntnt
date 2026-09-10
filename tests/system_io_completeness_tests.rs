use ntnt::interpreter::Value;

#[test]
fn temporary_prefix_cannot_select_a_drive_or_stream() {
    let root = tempfile::tempdir().unwrap();
    for function in ["temp_file", "temp_dir"] {
        for prefix in ["C:", "z:relative", "name:stream"] {
            let result = call(
                "fs",
                function,
                &[map(&[
                    ("parent", path(root.path())),
                    ("prefix", string(prefix)),
                ])],
            )
            .unwrap();
            assert!(
                is_err(result.clone()),
                "{function} accepted structural prefix {prefix:?}: {result:?}"
            );
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        }
        for prefix in ["", "ordinary-"] {
            let resource = ok(call(
                "fs",
                function,
                &[map(&[
                    ("parent", path(root.path())),
                    ("prefix", string(prefix)),
                ])],
            )
            .unwrap());
            let Value::String(name) = ok(call("fs", "temp_path", &[resource.clone()]).unwrap())
            else {
                panic!("path")
            };
            assert_eq!(std::path::Path::new(&name).parent(), Some(root.path()));
            ok(call("fs", "temp_close", &[resource]).unwrap());
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        }
    }
}

#[test]
fn atomic_destination_nul_is_rejected_before_publication() {
    let root = tempfile::tempdir().unwrap();
    let victim = root.path().join("victim");
    std::fs::write(&victim, b"original canary").unwrap();
    let malformed = format!("{}\0ignored", victim.to_str().unwrap());
    let result = call(
        "fs",
        "write_file_atomic",
        &[
            string(&malformed),
            string("replacement"),
            map(&[("sync", Value::Bool(false))]),
        ],
    )
    .unwrap();
    assert!(is_err(result.clone()), "{result:?}");
    assert!(
        format!("{result:?}").contains("invalid_argument:"),
        "must reject before staging, not fail during persist: {result:?}"
    );
    assert_eq!(std::fs::read(&victim).unwrap(), b"original canary");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    ok(call(
        "fs",
        "write_file_atomic",
        &[
            path(&victim),
            bytes(&[0, 255]),
            map(&[("sync", Value::Bool(false))]),
        ],
    )
    .unwrap());
    assert_eq!(std::fs::read(&victim).unwrap(), [0, 255]);
}

#[test]
fn binary_base64_decode_round_trips_non_utf8() {
    let module = ntnt::stdlib::crypto::init();
    let function = module
        .get("base64_decode_bytes")
        .expect("std/crypto must expose binary Base64 decoding");
    let Value::NativeFunction { func, .. } = function else {
        panic!("base64_decode_bytes must be callable");
    };
    let result = func(&[Value::String("AP8=".into())]).unwrap();
    let Value::EnumValue {
        enum_name,
        variant,
        values,
    } = result
    else {
        panic!("expected Result::Ok containing bytes");
    };
    assert_eq!(enum_name, "Result");
    assert_eq!(variant, "Ok");
    let Value::Array(bytes) = &values[0] else {
        panic!("expected raw bytes, not decoded text");
    };
    assert_eq!(bytes.len(), 2);
    assert!(matches!(bytes[0], Value::Int(0)));
    assert!(matches!(bytes[1], Value::Int(255)));
}

fn call(module: &str, name: &str, args: &[Value]) -> Result<Value, ntnt::error::IntentError> {
    let m = match module {
        "crypto" => ntnt::stdlib::crypto::init(),
        "time" => ntnt::stdlib::time::init(),
        "fs" => ntnt::stdlib::fs::init(),
        "net" => ntnt::stdlib::net::init(),
        "concurrent" => ntnt::stdlib::concurrent::init(),
        _ => panic!("unknown module"),
    };
    let Value::NativeFunction { func, .. } =
        m.get(name).unwrap_or_else(|| panic!("missing {name}"))
    else {
        panic!("not callable")
    };
    func(args)
}
fn ok(value: Value) -> Value {
    let Value::EnumValue {
        enum_name,
        variant,
        mut values,
    } = value
    else {
        panic!("expected Result")
    };
    assert_eq!(
        (enum_name.as_str(), variant.as_str()),
        ("Result", "Ok"),
        "{values:?}"
    );
    values.remove(0)
}
fn is_err(value: Value) -> bool {
    matches!(value, Value::EnumValue { enum_name, variant, .. } if enum_name == "Result" && variant == "Err")
}
fn bytes(b: &[u8]) -> Value {
    Value::Array(b.iter().map(|b| Value::Int(i64::from(*b))).collect())
}
fn string(s: &str) -> Value {
    Value::String(s.into())
}
fn equal(a: Value, b: Value) {
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
}

#[test]
fn binary_codecs_preserve_exact_bytes_and_reject_invalid_inputs() {
    let raw = bytes(&[0, 255, 254]);
    equal(
        call("crypto", "base64url_encode_bytes", &[raw.clone()]).unwrap(),
        string("AP_-"),
    );
    equal(
        ok(call("crypto", "base64url_decode_bytes", &[string("AP_-")]).unwrap()),
        raw,
    );
    equal(
        call("crypto", "utf8_encode", &[string("é")]).unwrap(),
        bytes(&[195, 169]),
    );
    equal(
        ok(call("crypto", "utf8_decode", &[bytes(&[195, 169])]).unwrap()),
        string("é"),
    );
    assert!(is_err(
        call("crypto", "utf8_decode", &[bytes(&[255])]).unwrap()
    ));
    for name in ["base64_decode_bytes", "base64url_decode_bytes"] {
        assert!(is_err(call("crypto", name, &[string("!!!")]).unwrap()));
        equal(ok(call("crypto", name, &[string("")]).unwrap()), bytes(&[]));
        assert!(is_err(
            call("crypto", name, &[string(&"A".repeat(22_369_624))]).unwrap()
        ));
    }
    for invalid in [
        Value::Int(-1),
        Value::Int(256),
        Value::Float(1.0),
        Value::Bool(true),
    ] {
        assert!(call(
            "crypto",
            "base64url_encode_bytes",
            &[Value::Array(vec![invalid.clone()])]
        )
        .is_err());
        let result = call("crypto", "utf8_decode", &[Value::Array(vec![invalid])]);
        assert!(result.is_err() || is_err(result.unwrap()));
    }
    equal(
        ok(call("crypto", "base64_decode", &[string("aGVsbG8=")]).unwrap()),
        string("hello"),
    );
}

#[test]
fn binary_hashes_and_hmac_verify_rfc_vectors() {
    let data = bytes(b"abc");
    equal(
        call("crypto", "sha256_bytes", &[data.clone()]).unwrap(),
        bytes(
            &hex::decode("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
                .unwrap(),
        ),
    );
    equal(call("crypto", "sha512", &[data.clone()]).unwrap(), string("ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"));
    for name in ["sha256", "sha256_bytes", "sha512", "sha512_bytes"] {
        equal(
            call("crypto", name, &[data.clone()]).unwrap(),
            call("crypto", name, &[string("abc")]).unwrap(),
        );
        assert!(call("crypto", name, &[Value::Array(vec![Value::Int(256)])]).is_err());
    }
    let key = bytes(&[0x0b; 20]);
    let tag = "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7";
    equal(
        call("crypto", "hmac_sha256", &[key.clone(), bytes(b"Hi There")]).unwrap(),
        string(tag),
    );
    equal(
        call(
            "crypto",
            "hmac_sha256_bytes",
            &[key.clone(), string("Hi There")],
        )
        .unwrap(),
        bytes(&hex::decode(tag).unwrap()),
    );
    for expected in [string(tag), bytes(&hex::decode(tag).unwrap())] {
        equal(
            ok(call(
                "crypto",
                "hmac_sha256_verify",
                &[key.clone(), string("Hi There"), expected.clone()],
            )
            .unwrap()),
            Value::Bool(true),
        );
        equal(
            ok(call(
                "crypto",
                "hmac_sha256_verify",
                &[key.clone(), string("changed"), expected],
            )
            .unwrap()),
            Value::Bool(false),
        );
    }
    for tag in [
        string("zz"),
        string(&"g".repeat(64)),
        bytes(&[0; 31]),
        bytes(&[0; 33]),
        Value::Array(vec![Value::Int(256); 32]),
    ] {
        assert!(is_err(
            call(
                "crypto",
                "hmac_sha256_verify",
                &[key.clone(), string(""), tag]
            )
            .unwrap()
        ));
    }
    equal(
        call("crypto", "hmac_sha256", &[string(""), string("")]).unwrap(),
        string("b613679a0814d9ec772f95d778c35fc5ff1697c493715653c6c712144292c5ad"),
    );
}

#[test]
fn monotonic_clock_checked_process_local_arithmetic() {
    let start = call("time", "monotonic_now", &[]).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(3));
    let Value::Int(elapsed) = ok(call("time", "monotonic_elapsed", &[start.clone()]).unwrap())
    else {
        panic!()
    };
    assert!(elapsed >= 3);
    let Value::Int(now) = call("time", "monotonic_now", &[]).unwrap() else {
        panic!()
    };
    let Value::Int(begin) = start else { panic!() };
    assert!(now >= begin);
    let due = ok(call("time", "monotonic_deadline", &[Value::Int(0)]).unwrap());
    equal(
        ok(call("time", "monotonic_remaining", &[due]).unwrap()),
        Value::Int(0),
    );
    let due = ok(call("time", "monotonic_deadline", &[Value::Int(1000)]).unwrap());
    let Value::Int(remaining) = ok(call("time", "monotonic_remaining", &[due]).unwrap()) else {
        panic!()
    };
    assert!((0..=1000).contains(&remaining));
    for name in [
        "monotonic_elapsed",
        "monotonic_deadline",
        "monotonic_remaining",
    ] {
        for invalid in [Value::Int(-1), Value::Float(1.0), string("0")] {
            assert!(is_err(call("time", name, &[invalid]).unwrap()));
        }
    }
    assert!(is_err(
        call("time", "monotonic_elapsed", &[Value::Int(i64::MAX)]).unwrap()
    ));
    assert!(is_err(
        call("time", "monotonic_deadline", &[Value::Int(i64::MAX)]).unwrap()
    ));
}

fn map(entries: &[(&str, Value)]) -> Value {
    Value::Map(
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}
fn path(p: &std::path::Path) -> Value {
    string(p.to_str().unwrap())
}
fn atomic_options() -> Value {
    map(&[("sync", Value::Bool(cfg!(unix)))])
}

#[test]
fn atomic_write_publishes_complete_bytes_and_preserves_old_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("data");
    std::fs::write(&file, b"old").unwrap();
    ok(call(
        "fs",
        "write_file_atomic",
        &[path(&file), bytes(&[0, 255]), atomic_options()],
    )
    .unwrap());
    assert_eq!(std::fs::read(&file).unwrap(), [0, 255]);
    for (content, options) in [
        (Value::Array(vec![Value::Int(256)]), atomic_options()),
        (string("bad"), map(&[("sync", string("yes"))])),
        (string("bad"), map(&[("mode", Value::Int(512))])),
        (string("bad"), map(&[("unknown", Value::Bool(true))])),
    ] {
        assert!(is_err(
            call("fs", "write_file_atomic", &[path(&file), content, options]).unwrap()
        ));
        assert_eq!(std::fs::read(&file).unwrap(), [0, 255]);
    }
    let dest_dir = dir.path().join("directory");
    std::fs::create_dir(&dest_dir).unwrap();
    assert!(is_err(
        call(
            "fs",
            "write_file_atomic",
            &[path(&dest_dir), string("bad"), atomic_options()]
        )
        .unwrap()
    ));
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        2,
        "no staging leak"
    );
    let reading = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let running = reading.clone();
    let read_path = file.clone();
    std::fs::write(&file, vec![1; 16384]).unwrap();
    let observer = std::thread::spawn(move || {
        while running.load(std::sync::atomic::Ordering::Acquire) {
            let data = std::fs::read(&read_path).unwrap();
            assert_eq!(data.len(), 16384);
            assert!(data.iter().all(|b| *b == data[0]));
        }
    });
    for n in 2..8 {
        ok(call(
            "fs",
            "write_file_atomic",
            &[path(&file), bytes(&vec![n; 16384]), atomic_options()],
        )
        .unwrap());
    }
    reading.store(false, std::sync::atomic::Ordering::Release);
    observer.join().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        ok(call(
            "fs",
            "write_file_atomic",
            &[path(&link), string("replacement")],
        )
        .unwrap());
        assert!(!std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(std::fs::read(&file).unwrap(), vec![7; 16384]);
    }
    #[cfg(not(unix))]
    {
        assert!(is_err(
            call("fs", "write_file_atomic", &[path(&file), string("bad")]).unwrap()
        ));
        assert!(is_err(
            call(
                "fs",
                "write_file_atomic",
                &[
                    path(&file),
                    string("bad"),
                    map(&[("sync", Value::Bool(false)), ("mode", Value::Int(384))])
                ]
            )
            .unwrap()
        ));
    }
}

#[test]
fn owned_temporary_resources_share_identity_cleanup_and_reject_transfer() {
    let root = tempfile::tempdir().unwrap();
    let opts = map(&[("parent", path(root.path())), ("prefix", string("owned-"))]);
    for name in ["temp_file", "temp_dir"] {
        let resource = ok(call("fs", name, &[opts.clone()]).unwrap());
        let alias = resource.clone();
        let Value::String(p) = ok(call("fs", "temp_path", &[resource.clone()]).unwrap()) else {
            panic!()
        };
        assert!(std::path::Path::new(&p).exists());
        if name == "temp_file" {
            ok(call("fs", "write_file", &[string(&p), string("use owned path")]).unwrap());
        }
        let nested = Value::ok(map(&[("resource", Value::Array(vec![alias.clone()]))]));
        assert!(ntnt::stdlib::json::intent_value_to_json_reject(&nested).is_err());
        let Value::Array(channel) = call("concurrent", "channel", &[]).unwrap() else {
            panic!()
        };
        assert!(call("concurrent", "send", &[channel[0].clone(), nested]).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                if name == "temp_dir" { 0o700 } else { 0o600 }
            );
            if name == "temp_dir" {
                let outside = root.path().join("outside");
                std::fs::write(&outside, b"safe").unwrap();
                std::os::unix::fs::symlink(&outside, std::path::Path::new(&p).join("link"))
                    .unwrap();
            }
        }
        ok(call("fs", "temp_close", &[resource]).unwrap());
        assert!(!std::path::Path::new(&p).exists());
        ok(call("fs", "temp_close", &[alias.clone()]).unwrap());
        assert!(is_err(call("fs", "temp_path", &[alias]).unwrap()));
    }
    #[cfg(unix)]
    assert_eq!(std::fs::read(root.path().join("outside")).unwrap(), b"safe");
    for prefix in ["../escape", "a/b", "a\\b", "bad\0"] {
        assert!(is_err(
            call(
                "fs",
                "temp_file",
                &[map(&[
                    ("parent", path(root.path())),
                    ("prefix", string(prefix))
                ])]
            )
            .unwrap()
        ));
    }
    assert!(is_err(
        call("fs", "temp_close", &[map(&[("path", string("forged"))])]).unwrap()
    ));
    let resource = ok(call("fs", "temp_dir", &[opts]).unwrap());
    let Value::String(p) = ok(call("fs", "temp_path", &[resource.clone()]).unwrap()) else {
        panic!()
    };
    drop(resource);
    assert!(!std::path::Path::new(&p).exists());
}

#[test]
fn symlink_inspection_preserves_target_and_does_not_follow() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    std::fs::write(&target, b"payload").unwrap();
    let link = root.path().join("link");
    let result = call("fs", "symlink", &[string("target"), path(&link)]).unwrap();
    #[cfg(windows)]
    if is_err(result.clone()) {
        let error = format!("{result:?}");
        assert!(
            error.contains("privilege") || error.contains("1314") || error.contains("denied"),
            "{error}"
        );
        assert!(!link.exists());
        return;
    }
    ok(result);
    equal(
        ok(call("fs", "read_link", &[path(&link)]).unwrap()),
        string("target"),
    );
    let Value::Map(stat) = ok(call("fs", "lstat", &[path(&link)]).unwrap()) else {
        panic!()
    };
    equal(stat["is_symlink"].clone(), Value::Bool(true));
    equal(stat["is_file"].clone(), Value::Bool(false));
    assert!(is_err(
        call("fs", "symlink", &[string("other"), path(&link)]).unwrap()
    ));
    equal(
        ok(call("fs", "read_link", &[path(&link)]).unwrap()),
        string("target"),
    );
    assert!(is_err(call("fs", "read_link", &[path(&target)]).unwrap()));
    assert!(is_err(
        call("fs", "lstat", &[path(&root.path().join("missing"))]).unwrap()
    ));
    assert!(is_err(
        call(
            "fs",
            "symlink",
            &[
                string("target"),
                path(&root.path().join("invalid")),
                string("invalid")
            ]
        )
        .unwrap()
    ));
    let directory = root.path().join("dir");
    std::fs::create_dir(&directory).unwrap();
    ok(call(
        "fs",
        "symlink",
        &[
            path(&directory),
            path(&root.path().join("dirlink")),
            string("dir"),
        ],
    )
    .unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let nonutf8 = std::ffi::OsStr::from_bytes(b"\xff");
        let raw_link = root.path().join("raw");
        std::os::unix::fs::symlink(nonutf8, &raw_link).unwrap();
        assert!(is_err(call("fs", "read_link", &[path(&raw_link)]).unwrap()));
    }
}

fn tcp_pair() -> (Value, std::net::TcpStream) {
    let listener = ok(call("net", "tcp_listen", &[Value::Int(0)]).unwrap());
    let Value::Map(addr) = ok(call("net", "tcp_local_addr", &[listener.clone()]).unwrap()) else {
        panic!()
    };
    let Value::Int(port) = addr["port"] else {
        panic!()
    };
    let client = std::net::TcpStream::connect(("127.0.0.1", port as u16)).unwrap();
    let stream = ok(call("net", "tcp_accept", &[listener.clone()]).unwrap());
    ok(call("net", "tcp_close", &[listener]).unwrap());
    (stream, client)
}
#[test]
fn tcp_reader_coalesced_binary_frames_share_socket_ownership() {
    use std::io::Write;
    let (stream, mut client) = tcp_pair();
    let reader = ok(call(
        "net",
        "tcp_reader",
        &[stream.clone(), map(&[("max_buffer_bytes", Value::Int(32))])],
    )
    .unwrap());
    assert!(is_err(
        call("net", "tcp_reader", &[stream.clone()]).unwrap()
    ));
    assert!(is_err(
        call(
            "net",
            "tcp_read",
            &[stream.clone(), Value::Int(1), Value::Int(10)]
        )
        .unwrap()
    ));
    client.write_all(&[0, 255, 0, 254, 7, 8, 9]).unwrap();
    equal(
        ok(call(
            "net",
            "tcp_read_until",
            &[reader.clone(), bytes(&[0, 254]), Value::Int(10)],
        )
        .unwrap()),
        bytes(&[0, 255, 0, 254]),
    );
    equal(
        ok(call("net", "tcp_read_exact", &[reader.clone(), Value::Int(3)]).unwrap()),
        bytes(&[7, 8, 9]),
    );
    let alias = reader.clone();
    drop(reader);
    client.write_all(b"ok").unwrap();
    equal(
        ok(call("net", "tcp_read_exact", &[alias.clone(), Value::Int(2)]).unwrap()),
        bytes(b"ok"),
    );
    let nested = Value::ok(map(&[("reader", alias.clone())]));
    assert!(ntnt::stdlib::json::intent_value_to_json_reject(&nested).is_err());
    let Value::Array(channel) = call("concurrent", "channel", &[]).unwrap() else {
        panic!()
    };
    assert!(call("concurrent", "send", &[channel[0].clone(), nested]).is_err());
    drop(alias);
    assert!(is_err(
        call("net", "tcp_write", &[stream.clone(), string("closed")]).unwrap()
    ));
    ok(call("net", "tcp_close", &[stream]).unwrap());
}

fn error(value: Value) -> String {
    let Value::EnumValue {
        variant, values, ..
    } = value
    else {
        panic!("expected Err")
    };
    assert_eq!(variant, "Err");
    let Value::String(s) = &values[0] else {
        panic!()
    };
    s.clone()
}
#[test]
fn tcp_reader_errors_preserve_frames_and_shutdown_is_local_eof() {
    use std::io::Write;
    let (stream, mut client) = tcp_pair();
    let reader = ok(call(
        "net",
        "tcp_reader",
        &[stream.clone(), map(&[("max_buffer_bytes", Value::Int(16))])],
    )
    .unwrap());
    client.write_all(b"abcdef!").unwrap();
    let e = error(
        call(
            "net",
            "tcp_read_exact",
            &[reader.clone(), Value::Int(10), Value::Int(30)],
        )
        .unwrap(),
    );
    assert!(
        e.starts_with("timeout:") && e.contains("buffered_bytes=7"),
        "{e}"
    );
    // Delimiter exists in retained storage but beyond this call's boundary.
    let e = error(
        call(
            "net",
            "tcp_read_until",
            &[reader.clone(), string("!"), Value::Int(3), Value::Int(30)],
        )
        .unwrap(),
    );
    assert!(
        e.starts_with("oversize:") && e.contains("buffered_bytes=7"),
        "{e}"
    );
    for args in [
        vec![reader.clone(), Value::Int(0)],
        vec![reader.clone(), Value::Int(17)],
        vec![reader.clone(), Value::Int(1), Value::Int(0)],
        vec![reader.clone(), Value::Int(1), Value::Int(60001)],
    ] {
        assert!(is_err(call("net", "tcp_read_exact", &args).unwrap()));
    }
    assert!(is_err(
        call(
            "net",
            "tcp_read_until",
            &[reader.clone(), bytes(&[]), Value::Int(3)]
        )
        .unwrap()
    ));
    assert!(is_err(
        call(
            "net",
            "tcp_read_until",
            &[
                reader.clone(),
                Value::Array(vec![Value::Int(-1)]),
                Value::Int(3)
            ]
        )
        .unwrap()
    ));
    ok(call("net", "tcp_shutdown", &[stream.clone(), string("read")]).unwrap());
    equal(
        ok(call("net", "tcp_read_exact", &[reader.clone(), Value::Int(3)]).unwrap()),
        bytes(b"abc"),
    );
    let e = error(
        call(
            "net",
            "tcp_read_exact",
            &[reader.clone(), Value::Int(5), Value::Int(30)],
        )
        .unwrap(),
    );
    assert!(
        e.starts_with("eof:") && e.contains("buffered_bytes=4"),
        "{e}"
    );
    equal(
        ok(call(
            "net",
            "tcp_read_until",
            &[reader.clone(), string("!"), Value::Int(4)],
        )
        .unwrap()),
        bytes(b"def!"),
    );
    ok(call("net", "tcp_close", &[reader.clone()]).unwrap());
    assert!(is_err(
        call("net", "tcp_read_exact", &[reader, Value::Int(1)]).unwrap()
    ));
    assert!(is_err(
        call("net", "tcp_write", &[stream, string("x")]).unwrap()
    ));
}
#[test]
fn tcp_reader_fragmented_delimiter_and_whole_call_trickle_timeout() {
    use std::io::Write;
    let (stream, mut client) = tcp_pair();
    let reader = ok(call("net", "tcp_reader", &[stream]).unwrap());
    let peer = std::thread::spawn(move || {
        client.write_all(b"binary\0\xff\r").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        client.write_all(b"\nnext").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        client.write_all(b"\r\n").unwrap();
    });
    equal(
        ok(call(
            "net",
            "tcp_read_until",
            &[
                reader.clone(),
                string("\r\n"),
                Value::Int(32),
                Value::Int(1000),
            ],
        )
        .unwrap()),
        bytes(b"binary\0\xff\r\n"),
    );
    equal(
        ok(call(
            "net",
            "tcp_read_until",
            &[
                reader.clone(),
                string("\r\n"),
                Value::Int(32),
                Value::Int(1000),
            ],
        )
        .unwrap()),
        bytes(b"next\r\n"),
    );
    peer.join().unwrap();
    assert!(
        error(call("net", "tcp_read_exact", &[reader.clone(), Value::Int(1)]).unwrap())
            .starts_with("eof:")
    );
    ok(call("net", "tcp_close", &[reader]).unwrap());
    let (stream, mut client) = tcp_pair();
    let reader = ok(call("net", "tcp_reader", &[stream]).unwrap());
    let peer = std::thread::spawn(move || {
        for _ in 0..50 {
            if client.write_all(b"x").is_err() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });
    let start = std::time::Instant::now();
    let e = error(
        call(
            "net",
            "tcp_read_exact",
            &[reader.clone(), Value::Int(100), Value::Int(100)],
        )
        .unwrap(),
    );
    assert!(e.starts_with("timeout:"), "{e}");
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    ok(call("net", "tcp_close", &[reader]).unwrap());
    peer.join().unwrap();
}

#[test]
fn tcp_local_read_shutdown_does_not_admit_unbuffered_socket_bytes() {
    use std::io::Write;
    for how in ["read", "both"] {
        let (stream, mut client) = tcp_pair();
        let reader = ok(call("net", "tcp_reader", &[stream.clone()]).unwrap());
        client.write_all(b"unbuffered").unwrap();
        // All bytes are queued in the socket, none have entered the reader.
        ok(call("net", "tcp_shutdown", &[stream, string(how)]).unwrap());
        let e = error(
            call(
                "net",
                "tcp_read_exact",
                &[reader.clone(), Value::Int(1), Value::Int(100)],
            )
            .unwrap(),
        );
        assert!(
            e.starts_with("eof:") && e.contains("buffered_bytes=0"),
            "{e}"
        );
        ok(call("net", "tcp_close", &[reader]).unwrap());
    }
}

fn eval(
    interpreter: &mut ntnt::interpreter::Interpreter,
    source: &str,
) -> Result<Value, ntnt::error::IntentError> {
    let program = ntnt::parser::Parser::new(ntnt::lexer::Lexer::new(source).collect())
        .parse()
        .unwrap();
    interpreter.eval(&program)
}
#[test]
fn process_handle_captures_fail_before_spawn_even_when_nested() {
    // No OS process is started: rejection depends on the runtime-local value kind,
    // not on a registry lookup or a live process identifier.
    for resource in [
        Value::ProcessHandle(0),
        Value::Array(vec![Value::ProcessHandle(0)]),
        map(&[("process", Value::ProcessHandle(0))]),
        Value::ok(Value::Array(vec![Value::ProcessHandle(0)])),
    ] {
        let mut interpreter = ntnt::interpreter::Interpreter::new();
        interpreter.define_global("resource".into(), resource);
        let result = eval(
            &mut interpreter,
            "import { spawn } from \"std/concurrent\"\nspawn(fn() { resource })",
        );
        assert!(
            result.is_err(),
            "must reject before returning a task handle: {result:?}"
        );
    }
    let mut interpreter = ntnt::interpreter::Interpreter::new();
    interpreter.define_global("resource".into(), map(&[("ordinary", Value::Int(42))]));
    let result = eval(&mut interpreter, "import { spawn, await_task } from \"std/concurrent\"\nlet task = spawn(fn() { resource })\nunwrap(await_task(task))").unwrap();
    equal(result, map(&[("ordinary", Value::Int(42))]));
}

#[test]
fn task_capture_rejects_nested_owned_authority_before_spawn() {
    let resource = ok(call("fs", "temp_file", &[]).unwrap());
    let mut interpreter = ntnt::interpreter::Interpreter::new();
    interpreter.define_global(
        "resource".into(),
        Value::ok(Value::Array(vec![resource.clone()])),
    );
    let result = eval(
        &mut interpreter,
        "import { spawn } from \"std/concurrent\"\nspawn(fn() { resource })",
    );
    assert!(
        result.is_err(),
        "must reject capture before returning a TaskHandle: {result:?}"
    );
    ok(call("fs", "temp_close", &[resource]).unwrap());
}

#[test]
fn reader_capability_checks_precede_direct_alias_and_nested_callbacks() {
    use ntnt::interpreter::{ExecutionMode, Interpreter};
    let (stream, mut client) = tcp_pair();
    use std::io::Write;
    let reader = ok(call("net", "tcp_reader", &[stream.clone()]).unwrap());
    client.write_all(b"untouched").unwrap();
    for observer in [false] {
        for mode in [
            ExecutionMode::Normal,
            ExecutionMode::Worker,
            ExecutionMode::Job,
            ExecutionMode::UnitTest,
            ExecutionMode::HotReload,
        ] {
            if !observer && mode == ExecutionMode::Normal {
                continue;
            }
            for (name, args) in [
                ("tcp_reader", "stream"),
                ("tcp_read_exact", "reader, 1"),
                ("tcp_read_until", "reader, \"!\", 10"),
            ] {
                for expression in [
                    format!("{name}({args})"),
                    format!("alias({args})"),
                    "sort_by([reader, 1], alias)".into(),
                    "reduce([alias], [reader, 1], sort_by)".into(),
                ] {
                    let mut interp = Interpreter::new();
                    interp.set_execution_mode(mode);
                    interp.define_global("stream".into(), stream.clone());
                    interp.define_global("reader".into(), reader.clone());
                    let result=eval(&mut interp,&format!("import {{ {name} }} from \"std/net\"\nimport {{ sort_by }} from \"std/collections\"\nlet alias = {name}\n{expression}"));
                    let e = result.unwrap_err().to_string();
                    assert!(
                        e.contains(if observer {
                            "Unsupported native test capability"
                        } else {
                            "requires TcpServer"
                        }),
                        "{mode:?} {name} {expression}: {e}"
                    );
                }
            }
        }
    }
    equal(
        ok(call("net", "tcp_read_exact", &[reader.clone(), Value::Int(9)]).unwrap()),
        bytes(b"untouched"),
    );
    ok(call("net", "tcp_close", &[reader]).unwrap());
}

#[test]
fn binary_decoder_rejects_oversized_padding_before_decode_allocation() {
    let encoded = string(&"=".repeat(22_369_625));
    for name in ["base64_decode_bytes", "base64url_decode_bytes"] {
        assert!(error(call("crypto", name, &[encoded.clone()]).unwrap()).starts_with("capacity:"));
    }
}

#[path = "support/system_fixture.rs"]
mod system_fixture;
#[test]
fn normal_mode_tcp_example_exercises_coalesced_and_fragmented_binary_frames() {
    use std::io::{Read, Write};
    let mut fixture = system_fixture::Fixture::start(
        include_str!("../examples/system-io-completeness/tcp.tnt"),
        &["run"],
        &[("NTNT_SYSTEM_IO_TCP", "1")],
    );
    let mut peer = std::net::TcpStream::connect(fixture.ready()).unwrap();
    peer.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    peer.write_all(&[0, 255, 13, 10, 254, 0, 13]).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    peer.write_all(&[10, 7, 8]).unwrap();
    let mut response = [0; 2];
    peer.read_exact(&mut response).unwrap();
    assert_eq!(&response, b"OK");
    assert!(
        fixture.wait().success(),
        "{} {}",
        fixture.stdout(),
        fixture.stderr()
    );
    assert!(fixture.stdout().contains("SYSTEM_IO_TCP_OK"));
}

#[cfg(unix)]
#[test]
fn unix_private_creation_and_atomic_mode_honor_umask_in_isolated_process() {
    use std::os::unix::fs::PermissionsExt;
    const MARKER: &str = "NTNT_SYSTEM_IO_UMASK_CHILD";
    if std::env::var_os(MARKER).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "unix_private_creation_and_atomic_mode_honor_umask_in_isolated_process",
                "--nocapture",
            ])
            .env(MARKER, "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    // Only this isolated subprocess changes umask; production code never does.
    unsafe {
        libc::umask(0);
    }
    let resource = ok(call("fs", "temp_dir", &[]).unwrap());
    let Value::String(p) = ok(call("fs", "temp_path", &[resource.clone()]).unwrap()) else {
        panic!()
    };
    assert_eq!(
        std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let dest = std::path::Path::new(&p).join("data");
    unsafe {
        libc::umask(0o027);
    }
    ok(call(
        "fs",
        "write_file_atomic",
        &[
            path(&dest),
            string("x"),
            map(&[("mode", Value::Int(0o666))]),
        ],
    )
    .unwrap());
    assert_eq!(
        std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
        0o640
    );
    ok(call("fs", "temp_close", &[resource]).unwrap());
}

#[test]
fn opaque_identity_and_nested_task_capture_cover_each_resource_kind() {
    let (stream, _peer) = tcp_pair();
    let reader = ok(call("net", "tcp_reader", &[stream.clone()]).unwrap());
    let file = ok(call("fs", "temp_file", &[]).unwrap());
    let directory = ok(call("fs", "temp_dir", &[]).unwrap());
    for resource in [&reader, &file, &directory] {
        let mut interp = ntnt::interpreter::Interpreter::new();
        interp.define_global("resource".into(), resource.clone());
        interp.define_global("alias".into(), resource.clone());
        eval(&mut interp,"import { includes } from \"std/collections\"\nassert(resource == alias)\nassert(includes([resource], alias))").unwrap();
        interp.define_global(
            "nested".into(),
            Value::Struct {
                name: "Wrapper".into(),
                fields: std::collections::HashMap::from([(
                    "resource".into(),
                    Value::some(resource.clone()),
                )]),
            },
        );
        assert!(eval(
            &mut interp,
            "import { spawn } from \"std/concurrent\"\nspawn(fn() { nested })"
        )
        .is_err());
    }
    ok(call("net", "tcp_close", &[reader]).unwrap());
    ok(call("fs", "temp_close", &[file]).unwrap());
    ok(call("fs", "temp_close", &[directory]).unwrap());
}

#[test]
fn atomic_bare_relative_path_uses_current_directory() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("relative.tnt");
    std::fs::write(&source,"import { write_file_atomic, read_bytes } from \"std/fs\"\nimport { stringify } from \"std/json\"\nunwrap(write_file_atomic(\"bare\", [0, 255], map { \"sync\": false }))\nassert(stringify(unwrap(read_bytes(\"bare\"))) == \"[0,255]\")\n").unwrap();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_ntnt"));
    command.current_dir(root.path()).arg("run").arg(&source);
    for key in [
        "NTNT_ENV",
        "APP_ENV",
        "NTNT_SECRETS_PROVIDER",
        "NTNT_LINT_MODE",
        "NTNT_STRICT",
        "NTNT_TYPE_MODE",
        "NTNT_OOB_MODE",
    ] {
        command.env_remove(key);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(root.path().join("bare")).unwrap(), [0, 255]);
}
