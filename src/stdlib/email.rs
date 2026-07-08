//! Email Sending Module (std/email)
//!
//! SMTP email via the `lettre` crate (DD-058 item 2): signup confirmations,
//! password resets, notifications, contact forms. Configure once at startup
//! with `configure_email()`, then `send_email()` returns
//! `Ok(map { "message_id": ... })` or `Err(message)`.
//!
//! For development and CI there is a log transport
//! (`"transport": "log"` in the config, or `NTNT_EMAIL_MODE=log`): emails
//! are printed to stderr and reported as sent, so app code and tests run
//! without an SMTP server.

use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use lettre::message::{header::ContentType, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use std::collections::HashMap;
use std::sync::Mutex;

/// SMTP configuration set by configure_email()
#[derive(Debug, Clone)]
struct EmailConfig {
    host: String,
    port: u16,
    username: Option<String>,
    password: Option<String>,
    from: String,
    from_name: Option<String>,
    /// "starttls" | "implicit" | "none" (defaulted from the port)
    tls: String,
    /// "smtp" | "log"
    transport: String,
}

/// Global config + cached pooled transport (rebuilt on configure_email).
/// Process-wide like std/log's level; the pool makes send_email_batch and
/// hot request paths reuse connections.
#[allow(clippy::type_complexity)]
static EMAIL_STATE: Mutex<Option<(EmailConfig, Option<SmtpTransport>)>> = Mutex::new(None);

fn get_string(map: &HashMap<String, Value>, key: &str) -> Option<String> {
    match map.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// Effective transport mode: NTNT_EMAIL_MODE env var overrides config,
/// so tests and dev environments can force log mode without code changes
fn transport_mode(config: &EmailConfig) -> String {
    std::env::var("NTNT_EMAIL_MODE")
        .ok()
        .filter(|m| m == "log" || m == "smtp")
        .unwrap_or_else(|| config.transport.clone())
}

fn parse_config(map: &HashMap<String, Value>) -> Result<EmailConfig> {
    let transport = get_string(map, "transport").unwrap_or_else(|| "smtp".to_string());
    if transport != "smtp" && transport != "log" {
        return Err(IntentError::type_error(format!(
            "configure_email(): transport must be \"smtp\" or \"log\", got \"{}\"",
            transport
        )));
    }

    let from = get_string(map, "from").ok_or_else(|| {
        IntentError::type_error("configure_email() requires a \"from\" address".to_string())
    })?;

    let host = get_string(map, "host").unwrap_or_default();
    if transport == "smtp" && host.is_empty() {
        return Err(IntentError::type_error(
            "configure_email() requires a \"host\" (or use \"transport\": \"log\" for development)"
                .to_string(),
        ));
    }

    let port = match map.get("port") {
        Some(Value::Int(p)) if *p > 0 && *p <= 65535 => *p as u16,
        Some(Value::String(s)) => s.parse::<u16>().map_err(|_| {
            IntentError::type_error(format!("configure_email(): invalid port \"{}\"", s))
        })?,
        None => 587,
        Some(other) => {
            return Err(IntentError::type_error(format!(
                "configure_email(): port must be an Int, got {}",
                other.type_name()
            )))
        }
    };

    // TLS mode defaults from the port: 465 implicit, 587/25 STARTTLS
    let tls = match get_string(map, "tls") {
        Some(mode) if ["starttls", "implicit", "none"].contains(&mode.as_str()) => mode,
        Some(mode) => return Err(IntentError::type_error(format!(
            "configure_email(): tls must be \"starttls\", \"implicit\", or \"none\", got \"{}\"",
            mode
        ))),
        None => {
            if port == 465 {
                "implicit".to_string()
            } else {
                "starttls".to_string()
            }
        }
    };

    Ok(EmailConfig {
        host,
        port,
        username: get_string(map, "username"),
        password: get_string(map, "password"),
        from,
        from_name: get_string(map, "from_name"),
        tls,
        transport,
    })
}

fn build_transport(config: &EmailConfig) -> Result<SmtpTransport> {
    let builder = match config.tls.as_str() {
        "implicit" => SmtpTransport::relay(&config.host),
        "starttls" => SmtpTransport::starttls_relay(&config.host),
        _ => Ok(SmtpTransport::builder_dangerous(&config.host)),
    }
    .map_err(|e| IntentError::runtime_error(format!("Email transport setup failed: {}", e)))?;

    let mut builder = builder.port(config.port);
    if let (Some(user), Some(pass)) = (&config.username, &config.password) {
        builder = builder.credentials(Credentials::new(user.clone(), pass.clone()));
    }
    Ok(builder.build())
}

/// Parse a mailbox as "Name <addr>" or bare "addr"
fn parse_mailbox(addr: &str, name: Option<&str>) -> Result<Mailbox> {
    let rendered = match name {
        Some(n) => format!("{} <{}>", n, addr),
        None => addr.to_string(),
    };
    rendered
        .parse::<Mailbox>()
        .map_err(|e| IntentError::type_error(format!("Invalid email address \"{}\": {}", addr, e)))
}

/// Addresses for to/cc/bcc: a single string or an array of strings
fn parse_address_list(value: &Value, field: &str) -> Result<Vec<Mailbox>> {
    let raw: Vec<String> = match value {
        Value::String(s) => vec![s.clone()],
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::String(s) => Ok(s.clone()),
                other => Err(IntentError::type_error(format!(
                    "send_email(): \"{}\" entries must be strings, got {}",
                    field,
                    other.type_name()
                ))),
            })
            .collect::<Result<Vec<String>>>()?,
        other => {
            return Err(IntentError::type_error(format!(
                "send_email(): \"{}\" must be a string or array of strings, got {}",
                field,
                other.type_name()
            )))
        }
    };
    raw.iter().map(|addr| parse_mailbox(addr, None)).collect()
}

/// Build a lettre Message from send_email() options, returning the message
/// and its generated message id
fn build_message(config: &EmailConfig, opts: &HashMap<String, Value>) -> Result<(Message, String)> {
    let to_value = opts.get("to").ok_or_else(|| {
        IntentError::type_error("send_email() requires a \"to\" address".to_string())
    })?;
    let to_boxes = parse_address_list(to_value, "to")?;
    if to_boxes.is_empty() {
        return Err(IntentError::type_error(
            "send_email(): \"to\" must contain at least one address".to_string(),
        ));
    }

    let subject = get_string(opts, "subject").ok_or_else(|| {
        IntentError::type_error("send_email() requires a \"subject\"".to_string())
    })?;

    let text = get_string(opts, "text");
    let html = get_string(opts, "html");
    if text.is_none() && html.is_none() {
        return Err(IntentError::type_error(
            "send_email() requires \"text\" and/or \"html\" content".to_string(),
        ));
    }

    // Per-message from override falls back to the configured sender
    let from_addr = get_string(opts, "from").unwrap_or_else(|| config.from.clone());
    let from_name = get_string(opts, "from_name").or_else(|| config.from_name.clone());
    let from_box = parse_mailbox(&from_addr, from_name.as_deref())?;

    let message_id = format!(
        "<{}@{}>",
        uuid::Uuid::new_v4(),
        config
            .host
            .split(':')
            .next()
            .filter(|h| !h.is_empty())
            .unwrap_or("ntnt.local")
    );

    let mut builder = Message::builder()
        .from(from_box)
        .subject(&subject)
        .message_id(Some(message_id.clone()));

    for mailbox in to_boxes {
        builder = builder.to(mailbox);
    }
    if let Some(cc) = opts.get("cc") {
        for mailbox in parse_address_list(cc, "cc")? {
            builder = builder.cc(mailbox);
        }
    }
    if let Some(bcc) = opts.get("bcc") {
        for mailbox in parse_address_list(bcc, "bcc")? {
            builder = builder.bcc(mailbox);
        }
    }
    if let Some(reply_to) = get_string(opts, "reply_to") {
        builder = builder.reply_to(parse_mailbox(&reply_to, None)?);
    }

    let message = match (text, html) {
        (Some(text), Some(html)) => {
            builder.multipart(MultiPart::alternative_plain_html(text, html))
        }
        (Some(text), None) => builder.singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(text),
        ),
        (None, Some(html)) => builder.singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_HTML)
                .body(html),
        ),
        (None, None) => unreachable!("checked above"),
    }
    .map_err(|e| IntentError::type_error(format!("send_email(): invalid message: {}", e)))?;

    Ok((message, message_id))
}

/// Render a sent-result map: map { "message_id": ..., "transport": ... }
fn sent_result(message_id: String, transport: &str) -> Value {
    let mut result = HashMap::new();
    result.insert("message_id".to_string(), Value::String(message_id));
    result.insert(
        "transport".to_string(),
        Value::String(transport.to_string()),
    );
    Value::ok(Value::Map(result))
}

fn log_email(opts: &HashMap<String, Value>, message_id: &str) {
    let to = match opts.get("to") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    };
    let subject = get_string(opts, "subject").unwrap_or_default();
    eprintln!(
        "[EMAIL:log] to: {} | subject: {} | id: {}",
        to, subject, message_id
    );
}

/// Send one already-parsed message through the given mode, sharing the
/// cached transport for smtp
fn dispatch(
    config: &EmailConfig,
    transport_slot: &mut Option<SmtpTransport>,
    opts: &HashMap<String, Value>,
) -> Result<Value> {
    let mode = transport_mode(config);
    let (message, message_id) = match build_message(config, opts) {
        Ok(built) => built,
        // Bad options are a per-email Err value, not a hard error, so
        // batch sends can report per-item failures
        Err(e) => return Ok(Value::err(Value::String(e.to_string()))),
    };

    if mode == "log" {
        log_email(opts, &message_id);
        return Ok(sent_result(message_id, "log"));
    }

    if transport_slot.is_none() {
        *transport_slot = Some(build_transport(config)?);
    }
    let transport = transport_slot.as_ref().expect("just built");

    match transport.send(&message) {
        Ok(_) => Ok(sent_result(message_id, "smtp")),
        Err(e) => Ok(Value::err(Value::String(format!(
            "Failed to send email: {}",
            e
        )))),
    }
}

fn with_state<T>(f: impl FnOnce(&mut Option<(EmailConfig, Option<SmtpTransport>)>) -> T) -> T {
    let mut guard = EMAIL_STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

fn require_config_map(value: &Value, function: &str) -> Result<HashMap<String, Value>> {
    match value {
        Value::Map(m) => Ok(m.clone()),
        other => Err(IntentError::type_error(format!(
            "{}() requires a map, got {}",
            function,
            other.type_name()
        ))),
    }
}

/// Initialize the std/email module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // @ntnt configure_email
    // @module std/email
    // @module_description SMTP email sending with a log transport for development
    // @signature configure_email(config: Map<String, Any>) -> Unit
    // Configure the SMTP transport. Call once at startup, before send_email().
    //
    // Config keys: "host" (required for smtp), "port" (default 587), "username",
    // "password", "from" (required), "from_name", "tls" ("starttls" | "implicit" |
    // "none"; defaults from the port — 465 is implicit, others STARTTLS), and
    // "transport" ("smtp" | "log"). The log transport prints emails to stderr and
    // reports them as sent — use it in development and tests. The NTNT_EMAIL_MODE
    // env var ("log" or "smtp") overrides the configured transport.
    // @param config SMTP configuration map.
    // @returns Unit
    // @see_also send_email, send_email_batch, get_env
    // @since v0.4.12
    // @tags #email, #smtp
    // @error TypeError ~ "requires a \"from\" address" fix: "Add \"from\": \"hello@myapp.com\" to the config"
    // @example configure_email(map { "host": "smtp.example.com", "from": "hello@myapp.com", "username": "u", "password": "p" }) ~ "Production SMTP"
    // @example configure_email(map { "transport": "log", "from": "dev@localhost" }) ~ "Development: log emails instead of sending"
    module.insert(
        "configure_email".to_string(),
        Value::NativeFunction {
            name: "configure_email".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let map = require_config_map(&args[0], "configure_email")?;
                let config = parse_config(&map)?;
                with_state(|state| *state = Some((config, None)));
                Ok(Value::Unit)
            },
        },
    );

    // @ntnt send_email
    // @module std/email
    // @signature send_email(opts: Map<String, Any>) -> Result<Map<String, String>, String>
    // Send a single email through the configured transport.
    //
    // Options: "to" (string or array, required), "subject" (required), "text"
    // and/or "html" (at least one required — both sends a multipart message with
    // a plain-text fallback), "cc", "bcc", "reply_to", and per-message "from" /
    // "from_name" overrides. Returns Ok(map { "message_id", "transport" }) or
    // Err(message). Invalid options and SMTP failures are Err values (use
    // `otherwise` or match), not runtime errors.
    // @param opts The email to send.
    // @returns Ok with the generated message id, or Err with the failure reason.
    // @see_also configure_email, send_email_batch, template
    // @since v0.4.12
    // @tags #email, #smtp
    // @error RuntimeError ~ "configure_email() has not been called" fix: "Call configure_email() at startup before sending"
    // @example send_email(map { "to": "user@example.com", "subject": "Welcome!", "text": "Hello!" }) ~ "Plain text email"
    // @example send_email(map { "to": "user@example.com", "subject": "Welcome!", "html": template("emails/welcome.html", map { "name": name }), "text": "Welcome!" }) ~ "HTML with text fallback"
    // @gotcha Returns Err for bad addresses or SMTP failures instead of crashing — always handle the Result.
    module.insert(
        "send_email".to_string(),
        Value::NativeFunction {
            name: "send_email".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let opts = require_config_map(&args[0], "send_email")?;
                with_state(|state| {
                    let Some((config, transport_slot)) = state.as_mut() else {
                        return Err(IntentError::runtime_error(
                            "configure_email() has not been called — configure SMTP (or \"transport\": \"log\") at startup"
                                .to_string(),
                        ));
                    };
                    let config = config.clone();
                    dispatch(&config, transport_slot, &opts)
                })
            },
        },
    );

    // @ntnt send_email_batch
    // @module std/email
    // @signature send_email_batch(emails: Array<Map<String, Any>>) -> Result<Array<Any>, String>
    // Send multiple emails, reusing one SMTP connection.
    //
    // Each entry takes the same options as send_email(). Returns Ok with an
    // array of per-email results (each Ok(map) or Err(message), in input
    // order); a hard Err is returned only when the transport itself cannot be
    // set up. One bad email does not stop the rest.
    // @param emails Array of email option maps.
    // @returns Ok(array of per-email results), or Err for transport setup failure.
    // @see_also send_email, configure_email
    // @since v0.4.12
    // @tags #email, #smtp
    // @example send_email_batch([map { "to": "a@x.co", "subject": "Hi", "text": "..." }, map { "to": "b@x.co", "subject": "Hi", "text": "..." }]) ~ "Two emails, one connection"
    module.insert(
        "send_email_batch".to_string(),
        Value::NativeFunction {
            name: "send_email_batch".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let emails = match &args[0] {
                    Value::Array(items) => items.clone(),
                    other => {
                        return Err(IntentError::type_error(format!(
                            "send_email_batch() requires an array of email maps, got {}",
                            other.type_name()
                        )))
                    }
                };
                with_state(|state| {
                    let Some((config, transport_slot)) = state.as_mut() else {
                        return Err(IntentError::runtime_error(
                            "configure_email() has not been called — configure SMTP (or \"transport\": \"log\") at startup"
                                .to_string(),
                        ));
                    };
                    let config = config.clone();
                    let mut results = Vec::with_capacity(emails.len());
                    for email in &emails {
                        let opts = require_config_map(email, "send_email_batch")?;
                        results.push(dispatch(&config, transport_slot, &opts)?);
                    }
                    Ok(Value::ok(Value::Array(results)))
                })
            },
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: Vec<(&str, Value)>) -> HashMap<String, Value> {
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect()
    }

    fn s(text: &str) -> Value {
        Value::String(text.to_string())
    }

    fn log_config() -> EmailConfig {
        parse_config(&map(vec![
            ("transport", s("log")),
            ("from", s("dev@localhost")),
        ]))
        .unwrap()
    }

    #[test]
    fn config_requires_from() {
        let err = parse_config(&map(vec![("host", s("smtp.example.com"))])).unwrap_err();
        assert!(err.to_string().contains("from"));
    }

    #[test]
    fn config_requires_host_for_smtp_but_not_log() {
        let err = parse_config(&map(vec![("from", s("a@b.co"))])).unwrap_err();
        assert!(err.to_string().contains("host"));
        assert!(parse_config(&map(vec![("transport", s("log")), ("from", s("a@b.co"))])).is_ok());
    }

    #[test]
    fn tls_defaults_from_port() {
        let implicit = parse_config(&map(vec![
            ("host", s("smtp.example.com")),
            ("port", Value::Int(465)),
            ("from", s("a@b.co")),
        ]))
        .unwrap();
        assert_eq!(implicit.tls, "implicit");

        let starttls = parse_config(&map(vec![
            ("host", s("smtp.example.com")),
            ("from", s("a@b.co")),
        ]))
        .unwrap();
        assert_eq!(starttls.tls, "starttls");
        assert_eq!(starttls.port, 587);
    }

    #[test]
    fn message_requires_to_subject_and_content() {
        let config = log_config();
        let missing_to = build_message(&config, &map(vec![("subject", s("Hi"))])).unwrap_err();
        assert!(missing_to.to_string().contains("to"));

        let missing_subject = build_message(&config, &map(vec![("to", s("a@b.co"))])).unwrap_err();
        assert!(missing_subject.to_string().contains("subject"));

        let missing_body = build_message(
            &config,
            &map(vec![("to", s("a@b.co")), ("subject", s("Hi"))]),
        )
        .unwrap_err();
        assert!(missing_body.to_string().contains("text"));
    }

    #[test]
    fn message_builds_with_multiple_recipients_and_html() {
        let config = log_config();
        let (message, id) = build_message(
            &config,
            &map(vec![
                ("to", Value::Array(vec![s("a@b.co"), s("c@d.co")])),
                ("cc", s("cc@b.co")),
                ("reply_to", s("reply@b.co")),
                ("subject", s("Hello")),
                ("text", s("plain")),
                ("html", s("<b>rich</b>")),
            ]),
        )
        .unwrap();
        assert!(id.starts_with('<') && id.ends_with('>'));
        let rendered = String::from_utf8(message.formatted()).unwrap();
        assert!(rendered.contains("a@b.co"));
        assert!(rendered.contains("multipart/alternative"));
    }

    #[test]
    fn invalid_address_is_rejected() {
        let config = log_config();
        let err = build_message(
            &config,
            &map(vec![
                ("to", s("not an address")),
                ("subject", s("Hi")),
                ("text", s("x")),
            ]),
        )
        .unwrap_err();
        assert!(err.to_string().contains("Invalid email address"));
    }

    #[test]
    fn per_message_from_overrides_config() {
        let config = log_config();
        let (message, _) = build_message(
            &config,
            &map(vec![
                ("to", s("a@b.co")),
                ("from", s("other@sender.co")),
                ("from_name", s("Other")),
                ("subject", s("Hi")),
                ("text", s("x")),
            ]),
        )
        .unwrap();
        let rendered = String::from_utf8(message.formatted()).unwrap();
        assert!(rendered.contains("other@sender.co"));
        assert!(rendered.contains("Other"));
    }
}
