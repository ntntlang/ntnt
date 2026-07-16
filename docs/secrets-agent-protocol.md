# Local secrets-agent protocol v1

ntnt's `unix-socket` secrets provider can use any local agent that implements this protocol. The protocol is backend-neutral: an agent may obtain values from a managed secrets service, an operating-system key store, an HSM-backed service, or another deployment-owned source.

This is an integration contract for `std/secrets`; it is not a general socket API for ntnt programs.

## Transport

- Unix domain stream socket.
- One request and one response per connection.
- Each message is one UTF-8 JSON object followed by `\n`.
- The client half-closes its write side after the request.
- The agent should half-close its write side after the response. The newline completes the frame; the client does not wait for EOF, but rejects any trailing byte that is already available.
- Requests and responses must not use carriage returns or additional frames.
- Filesystem ownership and permissions authenticate the local endpoint. The protocol `scope` field detects routing mistakes but is not authentication.

## Request

```json
{"protocol":1,"request_id":1,"op":"get","name":"API_KEY","scope":"deployment-a"}
```

Fields are exact; unknown, duplicate, or missing fields are rejected.

| Field | Meaning |
|---|---|
| `protocol` | Integer protocol version; currently `1`. |
| `request_id` | Positive client-generated integer echoed by the response. It is correlation data, not a credential. |
| `op` | Operation; v1 supports only `get`. |
| `name` | Declared logical secret name. |
| `scope` | Deployment-owned authorization/routing scope configured outside application source. |

## Responses

Found:

```json
{"protocol":1,"request_id":1,"status":"found","scope":"deployment-a","value":"<opaque>"}
```

Non-value result:

```json
{"protocol":1,"request_id":1,"status":"missing","scope":"deployment-a"}
```

Supported statuses:

| Status | Meaning | Client retry/failover |
|---|---|---|
| `found` | A non-empty opaque value is present. | No |
| `missing` | The declared name has no value. | No |
| `access_denied` | The caller or deployment scope is not authorized. | No |
| `unavailable` | A bounded transient agent/backend outage. | Yes |
| `invalid_request` | The request is semantically invalid. | No |
| `invalid_configuration` | The agent cannot safely serve the configured deployment. | No |

Agents must not return backend error text, stack traces, paths, credentials, or secret fragments. ntnt validates the protocol version, request ID, exact scope, status shape, frame size, value size, and connection completion before exposing a value to `std/secrets`.

## Deployment configuration

```text
NTNT_SECRETS_PROVIDER=unix-socket
NTNT_SECRETS_SOCKET_ENDPOINTS=/run/ntnt-secrets/primary.sock,/run/ntnt-secrets/secondary.sock
NTNT_SECRETS_AUTHORIZATION_SCOPE=deployment-a
NTNT_SECRETS_TIMEOUT_MS=1000
```

Production endpoints must be below `/run/ntnt-secrets`. The deployment owner creates and protects that directory and its sockets; application code and `ntnt.toml` do not select endpoint paths or authorization scope.

The provider permits one through eight unique ordered endpoints, performs two bounded attempts per endpoint, and fails over only after `unavailable`. All endpoints in one configured group must represent the same authorization scope.

## Limits

| Limit | v1 value |
|---|---:|
| Response frame | 65,536 bytes |
| Decoded secret value | 32,768 bytes |
| Endpoints | 1–8 |
| Attempts per endpoint | 2 |
| Attempt timeout | 10–10,000 ms |

One monotonic deadline covers connect, request write, response read, and frame completion for each attempt. Partial responses cannot extend the deadline by making slow progress.

## Out of scope

- A plaintext secret cache inside ntnt.
- Remote TCP, HTTP, or WebSocket agents.
- Backend-specific behavior or dependencies.
- A general-purpose Unix-socket or WebSocket API for ntnt application code.

Those are separate security and language-design decisions rather than accidental side effects of the secrets-provider contract.
