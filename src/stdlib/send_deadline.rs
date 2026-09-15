//! Optional monitoring start authorization, enforced at the socket boundary.
use crate::interpreter::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub(crate) struct SendDeadline {
    wall: Option<i64>,
    monotonic: Option<i64>,
}

impl SendDeadline {
    pub(crate) fn parse(opts: Option<&HashMap<String, Value>>) -> Result<Self, String> {
        fn field(opts: Option<&HashMap<String, Value>>, key: &str) -> Result<Option<i64>, String> {
            match opts.and_then(|m| m.get(key)) {
                None => Ok(None),
                Some(Value::EnumValue {
                    enum_name,
                    variant,
                    values,
                }) if enum_name == "Option" && variant == "None" && values.is_empty() => Ok(None),
                Some(Value::Int(n)) if *n >= 0 => Ok(Some(*n)),
                _ => Err(format!("{key} must be a nonnegative Int or None")),
            }
        }
        Ok(Self {
            wall: field(opts, "start_deadline_ms")?,
            monotonic: field(opts, "start_monotonic_deadline_ms")?,
        })
    }

    pub(crate) fn is_configured(&self) -> bool {
        self.wall.is_some() || self.monotonic.is_some()
    }

    // Call adjacent to the actual connect/send syscall, after all setup.
    // This authorizes a userspace syscall; it cannot promise NIC wire time.
    pub(crate) fn check(&self) -> Result<(), String> {
        if self
            .wall
            .is_some_and(|n| chrono::Utc::now().timestamp_millis() >= n)
            || self
                .monotonic
                .is_some_and(|n| super::time::monotonic_millis() >= n)
        {
            Err("start_deadline_expired".into())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_deadlines_fail_closed_and_absent_is_compatible() {
        assert!(SendDeadline::parse(None).unwrap().check().is_ok());
        for key in ["start_deadline_ms", "start_monotonic_deadline_ms"] {
            for value in [
                Value::String("1000".into()),
                Value::Float(1000.0),
                Value::Int(-1),
                Value::Bool(true),
                Value::Unit,
            ] {
                assert!(
                    SendDeadline::parse(Some(&HashMap::from([(key.into(), value)]))).is_err(),
                    "accepted malformed {key}"
                );
            }
            let absent = HashMap::from([(key.into(), Value::none())]);
            assert!(SendDeadline::parse(Some(&absent)).unwrap().check().is_ok());
        }
    }

    #[test]
    fn expired_monotonic_deadline_cannot_be_extended_by_wall_clock() {
        let opts = HashMap::from([
            ("start_deadline_ms".into(), Value::Int(i64::MAX)),
            ("start_monotonic_deadline_ms".into(), Value::Int(0)),
        ]);
        assert_eq!(
            SendDeadline::parse(Some(&opts)).unwrap().check(),
            Err("start_deadline_expired".into())
        );
    }

    #[test]
    fn expired_wall_deadline_rejects_before_send() {
        let opts = HashMap::from([("start_deadline_ms".into(), Value::Int(1))]);
        let guard = SendDeadline::parse(Some(&opts)).unwrap();
        assert_eq!(guard.check(), Err("start_deadline_expired".into()));
    }
}
