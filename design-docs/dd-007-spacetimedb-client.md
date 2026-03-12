# Design Doc 007: SpacetimeDB Client for ntnt

**Status:** Draft  
**Author:** Larri  
**Created:** 2026-03-05  
**App:** ntnt  

## Summary

Add native SpacetimeDB 2.0 support to ntnt via a new `std/spacetime` stdlib module, enabling real-time subscriptions, typed queries, and reducer invocations from ntnt applications.

## Background

### What is SpacetimeDB?

SpacetimeDB is a relational database that runs application logic ("modules") inside the database itself. Clients connect via WebSocket and receive real-time updates when subscribed data changes. Version 2.0 (released Feb 2026) pivoted from games-only to general web applications.

**Key capabilities:**
- Relational tables with ACID guarantees
- Real-time subscription queries (SQL-like, push updates on change)
- Reducers (transactional server-side functions)
- Procedures (server functions with HTTP/external API access)
- View functions (per-user row/column filtering)
- Event tables (ephemeral pub/sub)
- 100-170k transactions/second

**Licensing:** BSL 1.1 (source-available, self-hostable, converts to AGPL v3 + linking exception after 4 years)

### Why for ntnt?

ntnt apps currently use PostgreSQL for persistent data. This works well for traditional CRUD, but some patterns require additional infrastructure:
- Real-time collaborative features (chat, live cursors, shared editing)
- Live dashboards and activity feeds
- Multiplayer game state
- Notifications and presence

Today these require separate WebSocket servers, Redis pub/sub, or polling. SpacetimeDB provides this out of the box with a single connection.

## Goals

1. **Native WebSocket client** — Rust implementation in ntnt runtime connecting to SpacetimeDB
2. **Real-time subscriptions** — Subscribe to queries, receive push updates
3. **Reducer invocation** — Call server-side functions from ntnt handlers
4. **Bridge to client** — Forward subscription updates to browser WebSockets
5. **Type safety** — Generated bindings from SpacetimeDB module schema

## Non-Goals

- Replacing PostgreSQL for traditional CRUD (use the right tool for the job)
- Running SpacetimeDB modules written in ntnt (modules are Rust/C#/TypeScript)
- Full ORM abstraction over SpacetimeDB

## Technical Design

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      ntnt Application                        │
├─────────────────────────────────────────────────────────────┤
│  HTTP Handlers          │  WebSocket Handlers               │
│  ──────────────         │  ────────────────────             │
│  - Use spacetime_*()    │  - Bridge subscriptions           │
│    for queries/reducers │    to connected clients           │
└────────────┬────────────┴───────────────┬───────────────────┘
             │                            │
             ▼                            ▼
┌─────────────────────────────────────────────────────────────┐
│                   std/spacetime module                       │
├─────────────────────────────────────────────────────────────┤
│  spacetime_connect(uri, db, token?)                         │
│  spacetime_subscribe(conn, query) → subscription_id         │
│  spacetime_unsubscribe(conn, subscription_id)               │
│  spacetime_call(conn, reducer, args) → result               │
│  spacetime_query(conn, sql) → rows (one-shot)               │
│  spacetime_on_update(conn, callback)                        │
└────────────────────────────┬────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────┐
│              SpacetimeDB Rust Client (vendored)              │
│              ───────────────────────────────────             │
│              WebSocket connection + BSATN codec              │
└────────────────────────────┬────────────────────────────────┘
                             │
                             ▼
                    ┌─────────────────┐
                    │  SpacetimeDB    │
                    │  (self-hosted   │
                    │   or Maincloud) │
                    └─────────────────┘
```

### Proposed API

#### Connection Management

```ntnt
// Connect to SpacetimeDB
let conn = spacetime_connect("ws://localhost:3000", "my_database", {
  token: get_env("SPACETIME_TOKEN") ?? none
})

// Connection is maintained for app lifetime
// Auto-reconnect on disconnect
```

#### Subscriptions (Real-time)

```ntnt
// Subscribe to a query — returns subscription handle
let sub = spacetime_subscribe(conn, "SELECT * FROM messages WHERE room_id = $1", [room_id])

// Register callback for updates
spacetime_on_insert(sub, fn(row) {
  // New row inserted matching subscription
  broadcast_to_room(room_id, { type: "new_message", data: row })
})

spacetime_on_update(sub, fn(old_row, new_row) {
  // Row updated
  broadcast_to_room(room_id, { type: "message_updated", data: new_row })
})

spacetime_on_delete(sub, fn(row) {
  // Row deleted
  broadcast_to_room(room_id, { type: "message_deleted", id: row.id })
})

// Unsubscribe when done
spacetime_unsubscribe(sub)
```

#### Reducer Calls (Mutations)

```ntnt
// Call a reducer (transactional server function)
let result = spacetime_call(conn, "send_message", {
  room_id: room_id,
  sender_id: user.id,
  content: message_text
})

// Reducers can return values
match result {
  Ok(data) => { /* success */ },
  Err(e) => { /* SpacetimeDB error */ }
}
```

#### One-shot Queries

```ntnt
// Query without subscription (for initial load, etc.)
let messages = spacetime_query(conn, "SELECT * FROM messages WHERE room_id = $1 ORDER BY created_at DESC LIMIT 50", [room_id])
```

### Implementation Phases

#### Phase 1: Core Client (2-3 weeks)

1. **Vendor SpacetimeDB Rust client** into ntnt runtime
   - Their client is MIT-licensed, separate from BSL database
   - Handles WebSocket connection, BSATN serialization, auth
   
2. **Basic stdlib functions:**
   - `spacetime_connect(uri, database, options)`
   - `spacetime_query(conn, sql, params)` — one-shot query
   - `spacetime_call(conn, reducer, args)` — reducer invocation

3. **Connection pooling** — maintain connections across requests (like pg pool)

#### Phase 2: Subscriptions (2-3 weeks)

1. **Subscription management:**
   - `spacetime_subscribe(conn, query, params)` 
   - `spacetime_unsubscribe(handle)`
   
2. **Callback system:**
   - `spacetime_on_insert(sub, callback)`
   - `spacetime_on_update(sub, callback)`
   - `spacetime_on_delete(sub, callback)`

3. **Background task** — process incoming subscription updates, dispatch to callbacks

#### Phase 3: WebSocket Bridge (1-2 weeks)

1. **Helper for forwarding to clients:**
   ```ntnt
   // In WebSocket handler
   let sub = spacetime_subscribe(conn, query, params)
   spacetime_bridge_to_websocket(sub, ws_client)
   // Updates automatically forwarded to browser
   ```

2. **Cleanup** — auto-unsubscribe when WebSocket disconnects

#### Phase 4: Codegen & Types (Optional, 2 weeks)

1. **CLI command:** `ntnt spacetime generate <module>`
2. **Output:** `.tnt` file with typed table definitions and reducer signatures
3. **Runtime validation** against schema

### Open Questions

1. **Connection lifecycle** — per-request vs app-lifetime connection pool?
   - Recommendation: Pool like PostgreSQL, reuse across requests

2. **Subscription scope** — per-user subscriptions vs shared?
   - SpacetimeDB parameterizes by client identity
   - May need ntnt-side multiplexing for shared subscriptions

3. **Error handling** — how to surface SpacetimeDB errors?
   - Reducers can throw `SenderError` (client's fault) vs internal errors
   - Map to ntnt Result types

4. **PgWire alternative** — could we just use `pg_*` functions?
   - SpacetimeDB's PgWire is limited (no subscriptions, no parameterized queries yet)
   - Native client required for real-time features

### Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| SpacetimeDB Rust client changes | Breaking updates | Vendor specific version, test before upgrading |
| BSL license concerns | Can't compete with Maincloud | We're using it, not selling it; no issue |
| Complexity for simple apps | Overengineering | Clear docs: "use PostgreSQL for CRUD, SpacetimeDB for real-time" |
| WebSocket connection limits | Scale concerns | Connection pooling, subscription multiplexing |

## Alternatives Considered

### 1. Just use PgWire compatibility
**Pros:** No new code, reuse `pg_*` functions  
**Cons:** No subscriptions, limited SQL support, misses the whole point of SpacetimeDB

### 2. External WebSocket service (Supabase Realtime style)
**Pros:** Simpler ntnt integration  
**Cons:** Another service to run, not as integrated, different protocol

### 3. Build our own real-time layer over PostgreSQL
**Pros:** Full control, standard database  
**Cons:** Significant work, reinventing SpacetimeDB poorly

## Success Metrics

- [ ] Can connect to self-hosted SpacetimeDB from ntnt
- [ ] Can subscribe to queries and receive real-time updates
- [ ] Can call reducers and handle responses
- [ ] Bridge subscriptions to browser WebSockets
- [ ] Documentation and example app (chat room)

## Timeline Estimate

| Phase | Effort | Dependencies |
|-------|--------|--------------|
| Phase 1: Core Client | 2-3 weeks | None |
| Phase 2: Subscriptions | 2-3 weeks | Phase 1 |
| Phase 3: WebSocket Bridge | 1-2 weeks | Phase 2 |
| Phase 4: Codegen | 2 weeks | Phase 1 |
| **Total** | **7-10 weeks** | |

## References

- [SpacetimeDB 2.0 Release Notes](https://github.com/clockworklabs/SpacetimeDB/releases/tag/v2.0.1)
- [SpacetimeDB SDK Overview](https://spacetimedb.com/docs/sdks/)
- [SpacetimeDB Rust Client](https://github.com/clockworklabs/SpacetimeDB/tree/master/crates/sdk)
- [SpacetimeDB Self-Hosting](https://spacetimedb.com/docs/deploying/spacetimedb-standalone/)
- [BSATN Serialization Format](https://spacetimedb.com/docs/bsatn/)
