use super::*;

fn options() -> HashMap<String, Value> {
    let callback = Value::NativeFunction {
        name: "padding_test_callback".into(),
        arity: 1,
        max_arity: 1,
        requires: None,
        func: |_| panic!("option parsing must not invoke callbacks"),
    };
    HashMap::from([
        (
            "base_url".into(),
            Value::String("https://auth.example.com".into()),
        ),
        ("eligible".into(), callback.clone()),
        ("deliver".into(), callback.clone()),
        ("authorize".into(), callback),
    ])
}

#[test]
fn padding_defaults_to_zero() {
    let parsed = parse_magic_link_flow_options(&Value::Map(options())).unwrap();
    assert_eq!(parsed.generic_response_floor_ms, 0);
    assert_eq!(parsed.client_limit, 10);
    assert_eq!(parsed.identity_limit, 3);
}

#[test]
fn padding_accepts_nonnegative_integers() {
    for floor in [0, 1, 1200, i64::MAX] {
        let mut options = options();
        options.insert("generic_response_floor_ms".into(), Value::Int(floor));
        let parsed = parse_magic_link_flow_options(&Value::Map(options)).unwrap();
        assert_eq!(parsed.generic_response_floor_ms, floor as u64);
    }
}

#[test]
fn padding_rejects_negative_and_non_integer_values() {
    for value in [
        Value::Int(-1),
        Value::Int(i64::MIN),
        Value::Float(0.0),
        Value::String("0".into()),
        Value::Bool(false),
        Value::Unit,
    ] {
        let mut options = options();
        options.insert("generic_response_floor_ms".into(), value);
        let error = parse_magic_link_flow_options(&Value::Map(options))
            .unwrap_err()
            .to_string();
        assert!(error.contains("generic_response_floor_ms"), "{error}");
        assert!(
            error.contains("must be >= 0") || error.contains("must be an int"),
            "{error}"
        );
    }
}
