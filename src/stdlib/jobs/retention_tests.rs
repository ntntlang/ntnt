use super::*;

#[test]
fn retention_invalid_config_does_not_enter_testing_mode() {
    let module = init();
    let Value::NativeFunction { func, .. } = module.get("configure_queue").unwrap() else {
        panic!()
    };
    let opts = kv::json_to_value_public(
        &serde_json::json!({"mode":"testing", "retention":{"max_records":0}}),
    );
    assert!(
        func(&[opts]).is_err(),
        "invalid retention must be rejected before testing-mode mutation"
    );
}
