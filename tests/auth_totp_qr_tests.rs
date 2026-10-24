use ntnt::interpreter::Value;
use ntnt::stdlib::auth;

fn qr(args: &[Value]) -> ntnt::error::Result<Value> {
    let module = auth::init();
    match module.get("totp_qr_svg") {
        Some(Value::NativeFunction { func, .. }) => func(args),
        _ => panic!("missing std/auth.totp_qr_svg"),
    }
}

fn result_string(value: Value, expected_variant: &str) -> String {
    let Value::EnumValue {
        enum_name,
        variant,
        values,
        ..
    } = value
    else {
        panic!("expected Result");
    };
    assert_eq!(enum_name, "Result");
    assert_eq!(variant, expected_variant);
    match values.as_slice() {
        [Value::String(s)] => s.clone(),
        _ => panic!("expected string payload"),
    }
}

#[test]
fn totp_qr_svg_rejects_invalid_uris_without_disclosure_or_panics() {
    let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
    for uri in [
        String::new(),
        "not a URI".to_string(),
        "otpauth:/missing-host".to_string(),
        "otpauth:///missing-host".to_string(),
        format!("https://totp/user?secret={secret}"),
        format!("otpauth://hotp/user?secret={secret}&counter=0"),
        format!("otpauth://totp/?secret={secret}"),
        format!("otpauth://totp/Issuer:?secret={secret}"),
        "otpauth://totp/private-label?secret=NOT-BASE32".to_string(),
        "otpauth://totp/private-label".to_string(),
        "otpauth://totp/private-label?secret=ABC".to_string(),
        format!("otpauth://totp/private-label?secret={secret}&period=0"),
        format!("otpauth://totp/private-label?secret={secret}&digits=9"),
        format!("otpauth://totp/private-label?secret={secret}&algorithm=unknown"),
        format!("otpauth://totp/Issuer:user?secret={secret}&issuer=Other"),
        format!("otpauth://user@totp/private-label?secret={secret}"),
        format!("otpauth://totp:123/private-label?secret={secret}"),
        format!("otpauth://totp/private-label?secret={secret}#fragment"),
        format!(" otpauth://totp/private-label?secret={secret}"),
        format!("otpauth://totp/private\nlabel?secret={secret}"),
        format!("otpauth://totp/private%ZZlabel?secret={secret}"),
    ] {
        let error = result_string(qr(&[Value::String(uri)]).unwrap(), "Err");
        assert_eq!(error, "Invalid TOTP enrollment URI");
    }
}

#[test]
fn totp_qr_svg_bounds_utf8_bytes_and_handles_qr_capacity() {
    let suffix = "?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
    let prefix = "otpauth://totp/";
    let sized = |label: &str, bytes: usize| {
        format!(
            "{prefix}{}{suffix}",
            label.repeat(bytes - prefix.len() - suffix.len())
        )
    };
    let at_limit = sized("1", 2048);
    assert_eq!(at_limit.len(), 2048);
    result_string(qr(&[Value::String(at_limit)]).unwrap(), "Ok");

    let over_limit = sized("1", 2049);
    assert_eq!(
        result_string(qr(&[Value::String(over_limit)]).unwrap(), "Err"),
        "TOTP URI must be at most 2048 UTF-8 bytes"
    );
    let unicode = format!("{prefix}{}{suffix}", "界".repeat(700));
    assert!(unicode.chars().count() < 2048);
    assert!(unicode.len() > 2048);
    assert_eq!(
        result_string(qr(&[Value::String(unicode)]).unwrap(), "Err"),
        "TOTP URI must be at most 2048 UTF-8 bytes"
    );

    // Byte-mode payload exceeds version-40/Q capacity within the input limit.
    let capacity = sized("a", 2048);
    assert_eq!(
        result_string(qr(&[Value::String(capacity)]).unwrap(), "Err"),
        "TOTP URI exceeds QR code capacity"
    );
}

#[test]
fn totp_qr_svg_checks_runtime_types_and_arity() {
    for args in [
        vec![],
        vec![Value::Int(42)],
        vec![Value::String("".into()), Value::Int(1)],
    ] {
        assert!(qr(&args).is_err());
    }
    let module = auth::init();
    assert!(matches!(
        module.get("totp_qr_svg"),
        Some(Value::NativeFunction {
            arity: 1,
            max_arity: 1,
            requires: None,
            ..
        })
    ));
}

#[test]
fn totp_qr_svg_preserves_matrix_with_four_module_quiet_zone() {
    use qrcode::{Color, EcLevel, QrCode};
    use std::collections::HashSet;

    let uri = auth::get_totp_uri(
        "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
        "Zoë <&\"'>@example.com",
        "研究 & <Admin>",
    )
    .unwrap();
    let svg = result_string(qr(&[Value::String(uri.clone())]).unwrap(), "Ok");
    let code = QrCode::with_error_correction_level(uri.as_bytes(), EcLevel::Q).unwrap();
    let side = code.width() + 8;
    let header = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {side} {side}\" width=\"{}\" height=\"{}\" shape-rendering=\"crispEdges\"><rect width=\"100%\" height=\"100%\" fill=\"white\"/><path fill=\"black\" d=\"",
        side * 4, side * 4,
    );
    // Fixed XML envelope contains only geometry: no text, metadata, or input.
    let path = svg
        .strip_prefix(&header)
        .unwrap()
        .strip_suffix("\"/></svg>")
        .unwrap();
    let pattern = regex::Regex::new(r"M(\d+) (\d+)h1v1h-1z").unwrap();
    let mut modules = HashSet::new();
    let mut parsed_bytes = 0;
    for capture in pattern.captures_iter(path) {
        parsed_bytes += capture[0].len();
        let x = capture[1].parse::<usize>().unwrap();
        let y = capture[2].parse::<usize>().unwrap();
        assert!(x >= 4 && y >= 4 && x < side - 4 && y < side - 4);
        assert!(modules.insert((x, y)), "duplicate module");
    }
    assert_eq!(parsed_bytes, path.len(), "non-geometry content");
    for y in 0..side {
        for x in 0..side {
            let expected = x >= 4
                && y >= 4
                && x < side - 4
                && y < side - 4
                && code[(x - 4, y - 4)] == Color::Dark;
            assert_eq!(modules.contains(&(x, y)), expected, "module ({x}, {y})");
        }
    }
}

#[test]
fn totp_qr_svg_example_lints_and_runs_natively() {
    use std::process::Command;
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("main.tnt");
    std::fs::write(&source, include_str!("../examples/totp_qr.tnt")).unwrap();
    for action in ["lint", "run"] {
        let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
            .arg(action)
            .arg(&source)
            .current_dir(directory.path())
            .env("NTNT_TYPE_MODE", "strict")
            .env("NTNT_LINT_MODE", "strict")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if action == "run" {
            let uri = auth::get_totp_uri(
                "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
                "Zoë <&\"'>@example.com",
                "研究 & <Admin>",
            )
            .unwrap();
            let expected = result_string(qr(&[Value::String(uri)]).unwrap(), "Ok");
            assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
        }
    }
}

#[test]
fn totp_qr_svg_works_without_auth_initialization() {
    let uri = auth::get_totp_uri(
        "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
        "Zoë <&\"'>@example.com",
        "研究 & <Admin>",
    )
    .unwrap();
    let svg = result_string(qr(&[Value::String(uri.clone())]).unwrap(), "Ok");
    assert!(svg.starts_with("<svg "));
    assert!(!svg.contains(&uri));
    assert!(!svg.contains("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"));
    assert!(!svg.contains("Zoë"));
    assert!(!svg.contains("研究"));
    assert_eq!(svg, result_string(qr(&[Value::String(uri)]).unwrap(), "Ok"));
}
