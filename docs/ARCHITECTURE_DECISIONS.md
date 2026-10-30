# Merton Labs — Architecture Decisions

This is the register of architecture decisions for the Merton Labs backend
(`merton-labs/merton-api`). It records the **effective** decisions. Where an
earlier requirement was later replaced, the old entry is kept and marked
**Superseded**, so readers can see what changed.

Related documents:

- [`../CLAUDE.md`](../CLAUDE.md): rules for agents working in this repository
- [`PROJECT_PLAN.md`](PROJECT_PLAN.md): scope, roadmap, phase status and acceptance criteria
- [`../README.md`](../README.md): how to run, configure and test the service

## How to read this document

Every decision uses the same fields:

| Field | Meaning |
| --- | --- |
| **ID** | Stable identifier, `ADR-<AREA>-<NN>`. Never reuse or renumber an ID. |
| **Status** | `Accepted`: in force. `Superseded`: replaced (see the referenced ID). `Proposed`: recommended, not yet approved by the project owner. `Open`: unresolved; needs a project-owner decision. |
| **Source** | `Owner`: decided by the project owner. `Implementation (Pn)`: chosen while implementing phase *n*, and in force because it shipped in reviewed code, but not separately approved as a product decision. |
| **Decision** | What was decided. |
| **Rationale** | Why. Text marked *(technical interpretation)* is a reconstruction; the original discussion is not on record. |
| **Consequences** | What this requires of, or rules out for, implementers. |
| **Phases** | Roadmap phases affected (see [PROJECT_PLAN.md §15](PROJECT_PLAN.md#15-implementation-roadmap)). |

**Rules for changing this document**

1. Never present a `Proposed` item as `Accepted`. Only the project owner can accept it.
2. To change an accepted decision, mark the old entry `Superseded by ADR-…`
   and add a new entry. Do not edit history away.
3. After any change here, check whether
   [PROJECT_PLAN.md](PROJECT_PLAN.md) (scope, phase deliverables, open
   questions) must change too, and update it in the same change set.

---

## 1. Product scope

### ADR-SCOPE-01 — Self-hosted personal portfolio tracker
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** Merton Labs is an open-source, self-hosted, multi-user
  investment portfolio tracker, inspired by Snowball Analytics. It records
  transactions and reports holdings, cash, cost basis, income, valuation,
  allocation and CSV import/export. It is **not** a brokerage, an
  order-execution service, or a quantitative analytics engine.
- **Rationale:** Financial correctness, reasonable security, simple
  self-hosting and maintainability are the product goals.
- **Consequences:** Any feature that executes trades, connects to brokers,
  or performs advanced analytics is out of scope until the owner approves it.

### ADR-SCOPE-02 — MVP asset classes
- **Status:** Accepted · **Source:** Owner · **Phases:** P2B–P6
- **Decision:** The MVP supports Thai stocks, international stocks, ETFs,
  mutual funds, cash, and bank deposits.
- **Consequences:** Data models, validation and tests cover only these asset
  classes. See ADR-INST-04 for the fixed allocation classes.

### ADR-SCOPE-03 — Bonds and fixed income in the MVP
- **Status:** Superseded by ADR-SCOPE-05 · **Source:** Owner
- **Decision (historical):** The initial architecture review included bond
  accounting in the MVP.
- **Consequences:** Removed after the architecture review. Do **not** add
  bond accrual, coupon, amortization or maturity logic.

### ADR-SCOPE-04 — Cryptocurrency and stablecoins in the MVP
- **Status:** Superseded by ADR-SCOPE-05 · **Source:** Owner
- **Decision (historical):** The initial architecture review included
  crypto assets.
- **Consequences:** Removed from the MVP. Do **not** add crypto instruments,
  wallets or on-chain data.

### ADR-SCOPE-05 — Explicit MVP exclusions
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** These are excluded from the MVP:
  - bonds and fixed income
  - cryptocurrency and stablecoins
  - options, futures, margin trading engines and derivatives pricing
  - automated brokerage integration and automated market-data feeds
  - advanced performance and risk analytics
  - benchmark comparison and portfolio optimization
  - sharing and collaboration
- **Consequences:** Older notes that mention these are obsolete. Do not
  reintroduce them without a new owner decision.

### ADR-SCOPE-06 — SabiQuant is not an MVP dependency
- **Status:** Accepted · **Source:** Owner · **Phases:** none (post-MVP)
- **Decision:** The SabiQuant library may be integrated later. The MVP must
  not depend on it.

### ADR-SCOPE-07 — License
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** Apache License 2.0 ([`../LICENSE`](../LICENSE)).

### ADR-SCOPE-08 — Snowball Analytics as the behavioural reference
- **Status:** Accepted · **Source:** Owner (2026-10-30) · **Phases:** all
- **Decision:** when a financial behaviour is not yet decided and Snowball
  Analytics documents how it works, follow Snowball. Where it documents
  nothing, the implementer proposes a rule and the owner confirms it.
- **Consequences:**
  - Each ADR that applies this rule cites the Snowball page it follows,
    or says that Snowball documents nothing on the point.
  - The rule does not override Accepted decisions, the MVP exclusions
    (ADR-SCOPE-05), or the precision rules (ADR-NUM-01 … NUM-05).

---

## 2. Technology stack

### ADR-STACK-01 — Backend stack
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** The backend uses:
  - Rust stable, edition 2024
  - Axum, Tokio and Tower/tower-http
  - REST, with OpenAPI via utoipa
  - SeaORM, with SeaORM Migration and SeaQuery
  - SQLite initially; PostgreSQL is the preferred future database
  - Serde
  - tracing and tracing-subscriber
- **Consequences:** No other web framework, ORM or runtime. New
  dependencies need a clear reason and must be compatible and maintained.
  The implemented versions are pinned in `Cargo.lock`, and the MSRV (1.94)
  is set by SeaORM 2.

### ADR-STACK-02 — Frontend is a separate SvelteKit application
- **Status:** Accepted · **Source:** Owner · **Phases:** all API phases
- **Decision:** The frontend lives in a separate repository and uses
  SvelteKit with Svelte 5 runes and TypeScript. It calls the Axum REST API
  directly. There is no SvelteKit BFF (backend-for-frontend) layer unless an
  explicit requirement appears.
- **Consequences:** The backend is the only API and the only authority on
  business rules and authorization. API contracts (OpenAPI) are the
  integration boundary.

### ADR-STACK-03 — Same-origin reverse-proxy deployment
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** The preferred deployment puts both behind one origin: `/`
  goes to SvelteKit and `/api/*` goes to Axum.
- **Consequences:** No CORS headers are needed, and none are sent (see
  ADR-API-06). Cookie `SameSite=Lax` and the CSRF design (ADR-AUTH-04)
  assume same-origin. Behind a proxy, set `APP_TRUSTED_ORIGINS` and
  `TRUSTED_PROXIES`.

### ADR-STACK-04 — Minimal operational footprint
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** The MVP runs on Docker and Docker Compose with persistent
  SQLite storage. Redis, Kubernetes, message brokers and mandatory cloud
  services are excluded.
- **Consequences:** State that would normally live in Redis (rate limits)
  is kept in process memory, with documented limits (ADR-AUTH-05).

---

## 3. Application architecture

### ADR-ARCH-01 — Single-crate, API-first modular monolith
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** One Cargo crate (`merton-api`) contains the binary and the
  library. There is no Cargo workspace, no extra crates, and no separate
  migration crate.
- **Consequences:** Migrations, entities and features all live under `src/`.

### ADR-ARCH-02 — Separation of responsibilities
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:**
  - Axum handlers stay thin.
  - HTTP, application (use cases), domain, repository and infrastructure
    responsibilities are kept explicit.
  - Financial calculations (FIFO, valuation, allocation, rounding) must not
    depend on Axum, SeaORM or SvelteKit types.
- **Consequences:** Financial logic is written as plain Rust functions over
  domain types, so it can be unit-tested without a database or HTTP server.
  Phase 1 already follows this: `src/auth/` contains no Axum types.

### ADR-ARCH-03 — Module organization (feature-first vs layer-first)
- **Status:** **Open** · **Source:** n/a · **Phases:** P2A onward
- **Current state (implemented):** a hybrid layout:
  - `src/auth/` holds framework-independent auth logic: domain, service and
    repository.
  - `src/http/` holds handlers, extractors, errors and OpenAPI for all
    features.
  - `src/infrastructure/` holds the database connection, SeaORM entities
    and migrations.
- **Recommendation (Proposed, not approved):** feature-oriented modules
  with internal separation of concerns.
- **Consequences:** Until the owner decides, preserve the existing layout.
  New features follow the same pattern: a `src/<feature>/` logic module,
  `src/http/<feature>.rs` handlers, and entities and migrations under
  `src/infrastructure/`. Do not reorganize existing modules without
  approval.
- **P2A/P2B note:** portfolios, instruments and cash accounts followed this
  default (`src/{portfolios,instruments,cash_accounts}/` plus
  `src/http/<feature>.rs`). Value types used by several features live in
  `src/validation.rs` (`DisplayName`, `CurrencyCode`, `check`) and
  `src/pagination.rs` (`Page`, `Paged`). The organization question is still
  Open.

### ADR-ARCH-04 — No speculative abstractions
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** Do not introduce abstractions until they are needed. That
  rules out generic repository traits, generic authorization frameworks,
  RBAC, plugin systems and empty modules.
- **Consequences:** Repositories are plain functions taking
  `&impl ConnectionTrait`. Add a trait only when a second implementation
  actually exists.

---

## 4. Database

### ADR-DB-01 — SQLite first, PostgreSQL-portable patterns
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** SQLite is the initial database, accessed through SeaORM.
  Schema and query patterns should be portable to PostgreSQL.
  Unavoidable SQLite-specific code is kept in one place: today that is
  `src/infrastructure/database.rs`, through `map_sqlx_sqlite_opts`.
- **Consequences:** Avoid raw SQLite-only SQL in migrations and
  repositories. Prefer the SeaORM/SeaQuery builders.

### ADR-DB-02 — Connection policy
- **Status:** Accepted · **Source:** Owner (policy), Implementation (P0) (values) · **Phases:** all
- **Decision:** Every SQLite connection has foreign keys enabled, WAL
  journaling, and a configurable busy timeout. Write transactions are kept
  short.
- **Implemented defaults:**
  - pool: max 5, min 1
  - acquire timeout: 5 s
  - `DATABASE_BUSY_TIMEOUT_MS`: 5000
  - the database file is created if missing
- **Consequences:** Never hold a write transaction across slow work such
  as Argon2 or a large CSV parse. Parse and validate first, then write.

### ADR-DB-03 — In-crate migrations applied at startup
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** SeaORM migrations live in `src/infrastructure/migrations/`
  and run (`Migrator::up`) before the HTTP listener binds.
- **Consequences:** Migrations must be forward-only, deterministic and
  safe on existing data. Never edit a migration that has already been
  committed; add a new one instead.

### ADR-DB-04 — UUID primary keys, generated as UUIDv7 in the application
- **Status:** Accepted · **Source:** Owner (UUID PKs), Implementation (P1) (v7) · **Phases:** all
- **Decision:** Primary keys are UUIDs generated by the application with
  `Uuid::now_v7()`, which gives time-ordered IDs and better index locality.
- **Consequences:** New tables follow the same pattern. IDs carry no
  authorization meaning (ADR-OWN-02).

### ADR-DB-05 — Timestamps and dates
- **Status:** Accepted (timestamps) / **Proposed** (dates) · **Source:** Implementation (P1) · **Phases:** all
- **Decision:**
  - **Timestamps (Accepted):** UTC timestamps use
    `TimestampWithTimeZone` columns mapped to `time::OffsetDateTime`. On
    SQLite these are stored as text. Session expiry is therefore compared
    in Rust, not in SQL.
- **Proposal:**
  - **Dates (Proposed):** use date-only columns (ISO-8601 `YYYY-MM-DD`) for
    transaction dates, price dates and FX-rate dates. These compare
    correctly as text in SQLite and natively in PostgreSQL.
- **Consequences:** Before adding SQL range filters on timestamp columns,
  verify their behaviour on SQLite. Date-range lookups such as "latest
  price on or before `as_of`" should use date columns.

### ADR-DB-06 — PostgreSQL migration is a project, not a configuration change
- **Status:** Accepted · **Source:** Owner · **Phases:** post-MVP
- **Decision:** Changing `DATABASE_URL` is not a SQLite→PostgreSQL migration.
  A real migration needs schema conversion, data transfer, financial
  reconciliation and verification.
- **Consequences:** Only the `sqlx-sqlite` driver feature is compiled in
  today.

### ADR-DB-07 — Classifying SQLite `ON DELETE RESTRICT` refusals
- **Status:** Accepted · **Source:** Implementation (P2A) · **Phases:** P2A onward
- **Decision:** SQLite reports a refusal from `ON DELETE RESTRICT` as
  `SQLITE_CONSTRAINT_TRIGGER` (code 1811), not as the usual foreign-key
  code (787). SeaORM's `sql_err()` only recognises 787. To cover both,
  `infrastructure::database::is_foreign_key_violation` matches either 787,
  or 1811 combined with the message "FOREIGN KEY".
- **Consequences:**
  - Any code that maps foreign-key failures to an API error must use this
    helper.
  - It is the one place to adapt when PostgreSQL support is added.

### ADR-DB-09 — Read-then-write transactions start with a write
- **Status:** Accepted · **Source:** Implementation (P2B) · **Phases:** P2B onward
- **Decision:** A SQLite transaction that reads and then writes must make
  its **first statement a write**, for example a guarded `UPDATE … WHERE
  owner` that doubles as the ownership check.
- **Rationale:** A deferred transaction that starts by reading cannot
  upgrade to a write lock once another connection has written. SQLite then
  fails immediately with `SQLITE_BUSY` ("database is locked") rather than
  waiting for the busy timeout.
- **Evidence (P2B):** a stress test ran portfolio delete against concurrent
  cash-account creates, 20 rounds on a file database with 5 connections.
  - Read-first version: 14 of 20 deletes failed with `500`.
  - Write-first version (`portfolios::repository::lock_owned`): 0 failures.
- **Consequences:**
  - Ledger transactions (P3 onward) must follow this rule.
  - Read-only transactions, such as list plus count, are unaffected.
  - PostgreSQL support must add row locks (`FOR UPDATE`/`FOR SHARE`) to
    the guarded writes as well; see ADR-PORT-04.

### ADR-DB-08 — `CHECK` constraints for enumerated columns
- **Status:** Accepted · **Source:** Implementation (P2A) · **Phases:** P2A onward
- **Decision:** Enumerated text columns get a portable `CHECK (col IN (…))`
  constraint (for example `portfolios.status`), as defense in depth behind
  the domain types.
- **Consequences:** Only portable expressions are allowed, so no `GLOB`,
  `REGEXP` or other SQLite-only functions. Format rules such as currency
  codes stay in Rust.

---

## 5. API contract

### ADR-API-01 — Versioned REST with OpenAPI
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** Endpoints live under `/api/v1`. Every endpoint is documented
  with utoipa, including its request bodies, success and error responses,
  security requirements, and required headers. The document is served at
  `GET /api/v1/openapi.json`.
- **Consequences:** An endpoint without OpenAPI annotations is incomplete.
  Tests assert that key parts of the OpenAPI document exist.

### ADR-API-02 — RFC 9457 Problem Details with stable codes
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** Errors are `application/problem+json` with the fields
  `type`, `title`, `status`, `detail` and a stable machine-readable `code`.
  Validation errors add `errors: [{field, message}]`. Success responses are
  ordinary JSON.
- **Implemented codes:**
  - `malformed_request`
  - `unsupported_media_type`
  - `unauthenticated`
  - `invalid_credentials`
  - `untrusted_origin`
  - `csrf_invalid`
  - `not_found`
  - `email_taken`
  - `validation_failed`
  - `rate_limited`
  - `service_overloaded`
  - `portfolio_archived`, `portfolio_not_archived`, `portfolio_has_history` (P2A)
  - `instrument_in_use`, `cash_account_in_use` (P2B)
  - `insufficient_holdings` (P3, `409`, with the date in `detail`)
  - `no_holdings` (P4, `409`): a split or stock dividend has no holding to
    adjust on its date
  - `price_exists` and `fx_rate_exists` (P5, `409`): a price for that
    instrument and date, or a rate for that pair and date, already exists
  - `internal_error`
- **Consequences:** Never change an existing code's meaning. Never leak SQL
  errors, stack traces, secrets or other users' data; internal errors are
  logged server-side and returned as `internal_error`.

### ADR-API-03 — Status code conventions
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:**
  - `200`: success
  - `201`: created
  - `204`: no content / deleted
  - `400`: malformed request
  - `401`: unauthenticated
  - `403`: CSRF failure or forbidden operation
  - `404`: missing **or inaccessible** resource
  - `409`: conflicting state
  - `415`: wrong content type
  - `422`: validation failure
  - `429`: rate limited
  - `500`: unexpected error
  - `503`: temporary overload

### ADR-API-04 — Offset pagination
- **Status:** Accepted (mechanism) · **Proposed, implemented provisionally in P2A** (limits) · **Source:** Owner (mechanism), Implementation (P2A) (limits) · **Phases:** P2A onward
- **Decision:** List endpoints accept `limit` and `offset`, return results in
  a stable deterministic order (with a unique tie-breaker, e.g. `id`), and
  enforce a maximum page size.
- **Provisional values (awaiting owner confirmation):**
  - `limit` defaults to 50 and may be 1–200.
  - `offset` defaults to 0 and is capped at the database's signed 64-bit
    maximum.
  - Out-of-range values return `422`; non-numeric values return `400`.
  - The response is `{items, total, limit, offset}`, where `total` is
    counted in the same read transaction as the page.

### ADR-API-05 — Decimal values serialized as JSON strings
- **Status:** Accepted · **Source:** Owner · **Phases:** P2B onward
- **Decision:** Exact financial values (money, quantity, price, rate) are
  serialized as JSON strings in canonical decimal form, never as JSON
  numbers.
- **Consequences:** Request bodies accept strings for these fields too,
  parsed into `rust_decimal::Decimal` with explicit validation. OpenAPI
  documents them as `string` with a decimal format and pattern.

### ADR-API-06 — No CORS
- **Status:** Accepted · **Source:** Implementation (P1), consistent with ADR-STACK-03 · **Phases:** all
- **Decision:** The API sends no CORS headers. Wildcard CORS is prohibited.
- **Consequences:** If a cross-origin frontend is ever required, it needs an
  owner decision and an explicit origin allow-list.

### ADR-API-07 — Strict request shapes
- **Status:** Accepted · **Source:** Implementation (P2A) · **Phases:** P2A onward
- **Decision:**
  - JSON request bodies use `#[serde(deny_unknown_fields)]`, so an attempt
    to send `owner_user_id`, `status` or `id` is rejected with `400`, not
    silently ignored.
  - Unknown query parameters are ignored.
  - A path ID that is not a valid UUID cannot name a resource and returns
    `404 not_found`, the same as a missing or inaccessible one (extractor
    `ResourceId`).

### ADR-API-08 — `Cache-Control: no-store` on private routes
- **Status:** Accepted · **Source:** Implementation (P1, P2A) · **Phases:** all
- **Decision:** Authentication routes and every route that returns a user's
  private data (portfolios onward) send `Cache-Control: no-store`.

---

## 6. Authentication and sessions (implemented in P1/P1.1)

### ADR-AUTH-01 — Local email/password with Argon2id
- **Status:** Accepted · **Source:** Owner · **Phases:** P1
- **Decision:** Accounts use local email and password. Passwords are
  hashed with Argon2id using the `argon2` crate defaults (m=19456 KiB, t=2,
  p=1) and a random salt per hash.
- **Consequences:** There is no OAuth, social login, MFA or JWT in the MVP.
  For unknown accounts the hash is verified against a dummy hash with the
  same cost, so response timing does not reveal whether an account exists.

### ADR-AUTH-02 — Server-managed sessions in HttpOnly cookies
- **Status:** Accepted · **Source:** Owner · **Phases:** P1
- **Decision:**
  - The session token is 256-bit random, encoded as base64url. Only its
    SHA-256 digest is stored (`user_sessions.token_hash`, unique).
  - The cookie is `__Host-merton_session` with `HttpOnly`, `Secure`,
    `SameSite=Lax` and `Path=/`.
  - Login revokes any previously presented session. Logout revokes the
    current session.
  - No authentication token is ever placed in browser storage.
- **Consequences:**
  - The cookie is named `merton_session` without `Secure` only when
    `SESSION_COOKIE_SECURE=false`. Startup rejects that setting unless
    every trusted origin is loopback HTTP.
  - Expiry and revocation are always checked on the server; browser cookie
    expiry is not authoritative.

### ADR-AUTH-03 — Session lifetime
- **Status:** Accepted · **Source:** Owner (7d/30d), Implementation (P1) (15 min interval) · **Phases:** P1
- **Decision:**
  - Sessions expire after 7 days idle (sliding) or 30 days in total
    (absolute).
  - Activity extends the idle expiry at most once every 15 minutes, which
    limits writes to SQLite.
  - The idle expiry never extends past the absolute expiry.

### ADR-AUTH-04 — CSRF protection
- **Status:** Accepted · **Source:** Owner · **Phases:** P1 onward
- **Decision:** Two layers protect against CSRF.
  - **Origin check.** State-changing endpoints require a trusted `Origin`.
    Without an `Origin`, `Sec-Fetch-Site` must be `same-origin` or `none`.
    Requests with neither header are treated as non-browser clients.
  - **CSRF token.** Authenticated mutations also require `X-CSRF-Token`.
    The token is `HMAC-SHA256(APP_SECRET_KEY, label ‖ session_id)`, issued
    by `GET /api/v1/auth/csrf` and verified in constant time.
- **Consequences:** **Every future authenticated mutation (POST, PATCH,
  PUT, DELETE) must use the `CsrfProtectedSession` extractor.**
  `APP_SECRET_KEY` is required, at least 32 bytes, and must stay stable
  across restarts.

### ADR-AUTH-05 — Process-local login rate limiting
- **Status:** Accepted · **Source:** Owner (requirement), Implementation (P1/P1.1) (values, semantics) · **Phases:** P1, P1.1
- **Decision:**
  - **Limits:** fixed 15-minute windows; 20 attempts per client IP
    (IPv6 grouped by /64), counting both login and registration; 5
    unsuccessful logins per normalized email.
  - **Atomicity:** quota is checked and consumed under one mutex
    (`FixedWindowLimiter::acquire`).
  - **Account slots:** a slot is reserved *before* the password is
    verified, and released only when the login succeeds.
  - **Bounded memory:** at most 10,000 keys are tracked. When the table is
    full, expired keys are evicted first, then the lowest-count keys.
  - **Proxies:** `X-Forwarded-For` is honoured only from exact
    `TRUSTED_PROXIES` IPs, reading the rightmost untrusted hop.
- **Consequences:** Limits reset on restart and are not shared between
  instances. Known residual risks are listed in
  [PROJECT_PLAN.md §21](PROJECT_PLAN.md#21-security-risks).

### ADR-AUTH-06 — Bounded Argon2 concurrency
- **Status:** Accepted · **Source:** Owner · **Phases:** P1.1
- **Decision:**
  - At most `AUTH_MAX_CONCURRENT_HASHES` Argon2 operations run at once
    (default 4, allowed 1–64). One shared semaphore covers registration,
    login and dummy-hash verification.
  - Permits are taken with `try_acquire`. When saturated, the server
    replies `503 service_overloaded` with `Retry-After: 1`; nothing queues.
  - The `OwnedSemaphorePermit` moves into the `spawn_blocking` closure.
    A cancelled request therefore cannot free capacity while its hash is
    still running.
- **Consequences:** Argon2 is only reachable through `HashPermit`, because
  `password::hash` and `password::verify` are private. Keep it that way.

### ADR-AUTH-07 — Email validation and normalization
- **Status:** Accepted · **Source:** Implementation (P1) · **Phases:** P1
- **Decision:**
  - Accepted addresses are unquoted ASCII with a dotted domain, at most
    254 characters, with a local part of at most 64 characters.
  - The normalized form is trimmed and ASCII-lowercased. It is unique in
    `users.normalized_email`, and the original form is kept for display.
- **Consequences:** Internationalized addresses are rejected. No
  provider-specific normalization is applied (Gmail dots or `+tags` are
  kept as-is).

### ADR-AUTH-08 — Password policy
- **Status:** Accepted · **Source:** Implementation (P1) · **Phases:** P1
- **Decision:** Passwords must be 12–128 characters, counted as Unicode
  scalar values. There are no composition rules.

### ADR-AUTH-09 — Registration reveals duplicate emails
- **Status:** Accepted · **Source:** Owner · **Phases:** P1
- **Decision:** Registering an existing normalized email returns `409
  email_taken`. Registration does not log the user in. Login errors stay
  generic.

### ADR-AUTH-10 — Account recovery
- **Status:** **Open** · **Phases:** follow-up
- **Decision:** Not implemented. Neither the mechanism (for example an
  admin CLI, or email-based reset, which would need email infrastructure
  that is currently out of scope) nor the session-revocation behaviour
  after a reset has been decided.
- **Consequences:** Until this is decided, an operator has to manage
  accounts directly in the database. Do not invent an insecure recovery
  flow.

### ADR-AUTH-11 — Expired session cleanup
- **Status:** **Open** · **Phases:** follow-up
- **Decision:** Not implemented. Expired and revoked rows in
  `user_sessions` accumulate. When and how they are purged is undecided.

---

## 7. Users and ownership

### ADR-OWN-01 — Personal multi-user model
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** Many users share one installation. Each user manages only
  their own portfolios. There are no teams, organizations, workspaces,
  invitations, sharing or public portfolios.

### ADR-OWN-02 — Identity comes only from the verified session
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** The authenticated identity is read only from the
  server-side session, through the `CurrentSession` or
  `CsrfProtectedSession` extractors. A `user_id` or `owner_user_id`
  supplied in a request body, query string or header is never trusted.
- **Implementation hook:** `auth::UserId` can only be constructed via
  `UserId::from_verified`, which is `pub(crate)` and called only from the
  session or login code.
- **Consequences:** Owner-scoped functions take a `UserId`, never a raw
  `Uuid` that came from a request.

### ADR-OWN-03 — Owner-scoped queries and relationship validation
- **Status:** Accepted · **Source:** Owner · **Phases:** P2A onward
- **Decision:** Ownership is enforced on every read, write, import and
  export:
  - Every repository query on a user-owned resource filters by the owner,
    directly or through the parent portfolio.
  - The application layer checks that related resources belong to the
    same owner. For example, a transaction's portfolio, instrument and
    cash account must all belong to the caller.
  - Foreign keys and constraints support this consistency.
- **Consequences:** Never fetch a resource by ID alone and check the owner
  afterwards in the handler. Put the owner in the query.

### ADR-OWN-04 — Inaccessible resources return 404
- **Status:** Accepted · **Source:** Owner · **Phases:** P2A onward
- **Decision:** A resource that exists but belongs to another user returns
  the same `404 not_found` as one that does not exist.

### ADR-OWN-05 — Cross-user authorization tests are mandatory
- **Status:** Accepted · **Source:** Owner · **Phases:** P2A onward
- **Decision:** Every phase that adds user-owned resources includes
  integration tests in which user B tries to read, update, delete, list,
  import into or export user A's resources, and gets 404 (or an
  empty list).

---

## 8. Portfolios

### ADR-PORT-01 — Portfolio attributes
- **Status:** Accepted · **Source:** Owner · **Phases:** P2A
- **Decision:** A portfolio has an immutable owner, a name, a base
  currency, a status (`ACTIVE` or `ARCHIVED`), and creation and update
  timestamps. A user may own many portfolios.

### ADR-PORT-02 — Base currency is a reporting currency
- **Status:** Accepted · **Source:** Owner · **Phases:** P2A, P5
- **Decision:** The base currency may change after transactions exist.
  Changing it never alters original transactions and never creates an FX
  transaction. Valuations are recalculated on demand in the new base
  currency.

### ADR-PORT-03 — Lifecycle
- **Status:** Accepted · **Source:** Owner · **Phases:** P2A onward
- **Decision:**
  - `ACTIVE`: read, update and financial mutations are allowed.
  - `ARCHIVED`: read and export are allowed; financial mutations are
    prohibited; restore to `ACTIVE` is allowed.
- **Updates while archived: Proposed, implemented provisionally in P2A.**
  Renaming or changing the base currency of an archived portfolio returns
  `409 portfolio_archived`. This follows the owner's lifecycle, which lists
  updates only under `ACTIVE`. Awaiting owner confirmation; see
  [PROJECT_PLAN.md §22](PROJECT_PLAN.md#22-open-questions).
- **Implemented transitions (P2A):**
  - archive from `ARCHIVED` returns `409 portfolio_archived`
  - restore from `ACTIVE` returns `409 portfolio_not_archived`
  - delete is allowed in either status (subject to ADR-PORT-04)
  - every write is a single conditional `UPDATE … WHERE owner AND status`,
    so concurrent requests cannot apply the same transition twice

### ADR-PORT-04 — Deletion only without financial history
- **Status:** Accepted · **Source:** Owner · **Phases:** P2A onward
- **Decision:** A portfolio can be permanently deleted only if it has no
  financial history. Posted financial records are never cascade-deleted.
- **Consequences:**
  - Foreign keys from financial tables to `portfolios` must use immediate
    (not deferrable) `RESTRICT`, never `CASCADE`.
  - The delete use case must check every financial table that exists at
    the time; each new phase extends that check.
  - Deleting a portfolio with history returns `409 portfolio_has_history`.
- **Unused cash accounts: Proposed, implemented provisionally in P2B.**
  A cash account with no movements is not financial history. Deleting a
  portfolio therefore runs as one transaction:
  1. confirm the caller owns the portfolio;
  2. delete its cash accounts;
  3. delete the portfolio.
  Awaiting owner confirmation.
- **History protection:** ledger tables (P3 onward) reference cash accounts
  and portfolios with immediate `RESTRICT`. Any step that would orphan a
  record fails, the whole transaction rolls back, and the API returns
  `409 portfolio_has_history` (ADR-DB-07). Tests prove the rollback with a
  stand-in movements table.
- **Other users:** another user's portfolio returns `404`, because the
  ownership check fails before anything is deleted.
- **Concurrency:** the transaction starts with a guarded write
  (`lock_owned`, ADR-DB-09). A concurrent cash-account create therefore
  waits, then finds the portfolio gone and returns `404`.
  - On PostgreSQL (not supported yet) this is not enough. There, the
    guarded `INSERT … SELECT` must lock the portfolio row (`FOR SHARE`), and
    the delete must take `FOR UPDATE`.
- **New tables:** a phase that adds a table referencing portfolios must
  extend `portfolios::service::delete` explicitly.

### ADR-PORT-05 — Base currency format
- **Status:** **Proposed, implemented provisionally in P2A** · **Source:** Implementation (P2A) · **Phases:** P2A
- **Decision:** A base currency is any three ASCII letters, trimmed and
  stored uppercased. It is not checked against the ISO 4217 list.
- **Consequences:** Tightening to the ISO list later would only affect new
  writes; existing values would need a migration plan. Awaiting owner
  confirmation.

### ADR-PORT-06 — Portfolio name
- **Status:** Accepted · **Source:** Implementation (P2A) · **Phases:** P2A
- **Decision:** A name is trimmed and must be 1–100 characters with no
  control characters. Names are not unique per owner.

---

## 9. Instruments

### ADR-INST-01 — User-scoped instruments with flexible identity
- **Status:** Accepted · **Source:** Owner · **Phases:** P2B
- **Decision:** Instruments belong to a user and have UUID primary keys.
  Identity attributes such as symbol, ISIN and exchange are optional and
  flexible. Duplicate or ambiguous identities are allowed; no global
  uniqueness is forced.
- **Consequences:** Do not assume a ticker is globally unique, and do not
  add a unique constraint on symbol or ISIN.

### ADR-INST-02 — Core columns plus typed JSON metadata
- **Status:** Accepted · **Source:** Owner · **Phases:** P2B
- **Decision:** Common attributes are relational columns. Attributes
  specific to an asset class go in a typed JSON metadata column, validated
  through Rust types.
- **P2B: metadata column deferred (Proposed).** No asset-specific attribute
  has been specified, and an empty column would be a speculative
  abstraction (ADR-ARCH-04). The column, with its Rust types, is added by a
  portable `ADD COLUMN` migration once the first attribute is defined.
  Awaiting owner confirmation.

### ADR-INST-03 — Fixed quote currency
- **Status:** Accepted · **Source:** Owner · **Phases:** P2B
- **Decision:** Each instrument has one fixed quote currency. Different
  listings or currencies of the same security are separate instruments
  where appropriate.

### ADR-INST-04 — Fixed asset classes
- **Status:** Accepted · **Source:** Owner · **Phases:** P2B, P5
- **Decision:** The asset classes are `STOCK`, `ETF`, `MUTUAL_FUND` and
  `CASH`. Thai and international stocks are both `STOCK`.
- **Implemented (P2B):**
  - Instruments use `STOCK`, `ETF` or `MUTUAL_FUND`, enforced by a
    `CHECK` constraint.
  - `CASH` is not an instrument class. Cash, including bank deposits, is
    held in cash accounts.
- **Bank deposits: Proposed, implemented provisionally in P2B.** A bank
  deposit is a cash account with no subtype. Fixed-term deposit attributes
  (maturity, rate) are not modelled. Awaiting owner confirmation.

### ADR-INST-06 — Instrument identifiers (P2B)
- **Status:** Accepted · **Source:** Implementation (P2B) · **Phases:** P2B
- **Decision:**
  - `symbol` and `exchange` are optional, trimmed, at most 32 characters,
    and kept as entered (no case folding). A blank value means none.
  - `isin` is optional. When present it is validated as ISO 6166, including
    the Luhn check digit, and stored uppercase.
  - `name` is 1–200 characters.
  - `quote_currency` is three letters and cannot be updated (ADR-INST-03).
  - None of these are unique (ADR-INST-01).
  - PATCH accepts `null` to clear `symbol`, `isin` or `exchange`.
- **Consequences:** CSV matching (P6) must not assume symbol case or
  uniqueness.

### ADR-INST-05 — No automatic instrument creation on import
- **Status:** Accepted · **Source:** Owner · **Phases:** P6
- **Decision:** CSV import never creates instruments. Unambiguous matches
  may be applied automatically; ambiguous matches require user review.

---

## 10. Transaction ledger

### ADR-LEDG-01 — The ledger is authoritative; balances are derived
- **Status:** Accepted · **Source:** Owner · **Phases:** P3 onward
- **Decision:** Holdings are derived from position movements, and cash
  balances from cash movements. No authoritative computed holdings or
  balances are stored.
- **Consequences:** Caches or snapshots, if they are ever added, are
  non-authoritative and must be rebuildable from the ledger. Adding them
  needs a decision.

### ADR-LEDG-02 — Conceptual ledger entities
- **Status:** Accepted (concepts) / **Open** (physical schema) · **Source:** Owner · **Phases:** P2B–P4
- **Decision:** The conceptual entities are `transactions`,
  `position_movements`, `cash_movements` and `cash_accounts`. This is a
  baseline, **not** an approved physical schema.
- **Consequences:** Propose the physical schema (columns, types,
  constraints, indexes) at the start of P3, and get it approved before
  writing migrations.
- **P3 schema: Proposed, implemented provisionally** (migration
  `m20261024_000006_create_ledger`). It was reviewed by an AGY design pass
  before implementation. Awaiting owner confirmation.
  - **`transactions`:** `id`, `portfolio_id`, `kind`, `trade_date` DATE,
    `sort_order` INT, nullable `instrument_id` / `cash_account_id`, nullable
    decimal-text `quantity`, `price`, `fee`, `tax`, `amount`, `note` (≤500),
    and timestamps. It stores the user's input. Index
    `(portfolio_id, trade_date, sort_order, id)`.
  - **`position_movements`:** `transaction_id`, `portfolio_id`,
    `instrument_id`, `kind` (`ACQUIRE`/`DISPOSE`), positive `quantity`, and
    `amount` (lot cost for an acquisition, net proceeds for a disposal).
    Holdings and FIFO are derived only from these rows.
  - **`cash_movements`:** `transaction_id`, `portfolio_id`,
    `cash_account_id`, and a signed `amount_delta`. Cash balances are
    derived only from these rows.
  - **Foreign keys:** every FK to a portfolio, instrument, cash account or
    transaction is immediate `ON DELETE RESTRICT`.
  - **`kind` columns:** no `CHECK` constraint, because P4 adds values and
    SQLite cannot alter a `CHECK` without rebuilding the table. Rust
    validates them on read and write.
  - **P4 extension:** additive only. Splits and stock dividends add
    movement kinds. Transfers and FX exchanges post two cash movements and
    add nullable transaction columns through `ADD COLUMN`.
  - **P4 cash transactions: Proposed, implemented provisionally**
    (migration `m20261030_000007_add_transfer_destination`). It adds one
    nullable column, `transactions.destination_cash_account_id`, with an
    inline `REFERENCES cash_accounts (id) ON DELETE RESTRICT` and an
    index. The inline form is used because SeaQuery cannot add a foreign
    key to an existing SQLite table; it is also valid PostgreSQL. A test
    asserts the `RESTRICT` rule, and a P3 database upgrades cleanly
    (smoke-tested). The new kinds reuse the existing columns: `amount`
    holds a deposit, withdrawal, fee, tax or transfer amount, or the gross
    income; `fee` and `tax` hold what was withheld from income. Awaiting
    owner confirmation.
  - **P4 FX exchanges and corporate actions: Proposed, implemented
    provisionally** (migration
    `m20261030_000008_add_exchange_and_corporate_actions`):
    - **New columns:** three nullable decimal-text columns on
      `transactions`: `destination_amount` (FX received), `ratio` (a
      split's `new:old`) and `cash_in_lieu_quantity`.
    - **New movement kinds:** `SPLIT` and `STOCK_DIVIDEND` on
      `position_movements`, with no schema change. See ADR-TXN-07 for what
      their `quantity` and `amount` hold.
    - **Order:** a transaction's movements replay in the order they were
      posted (time-ordered ids), so a lot adjustment precedes its cash in
      lieu.
    - **FIFO:** P3's note below is now implemented: `fifo::Event` has
      `Split` and `StockDividend`, which adjust lots.
    - **Evidence:** a P3 database upgrades through migrations 7 and 8
      (smoke-tested).
    - Awaiting owner confirmation.
  - **FIFO in P4:** P3's pure `fifo::Event` only acquires and disposes.
    Splits and stock dividends change existing lots rather than opening
    new ones, so P4 must add lot-adjusting events (raised by the AGY
    review; not built in P3).
  - **Indexes on `cash_movements`:** `(cash_account_id)` and
    `(transaction_id)`. An AGY review suggested adding
    `(portfolio_id, cash_account_id)`; it was not added, because a cash
    account belongs to exactly one portfolio, so the `cash_account_id`
    index already serves balance queries.

### ADR-LEDG-03 — Atomic related writes
- **Status:** Accepted · **Source:** Owner · **Phases:** P3 onward
- **Decision:** A transaction and all of its movements are written,
  edited or deleted in a single database transaction.

### ADR-LEDG-04 — FIFO, long-only, negative cash allowed
- **Status:** Accepted · **Source:** Owner · **Phases:** P3, P4
- **Decision:**
  - Shares, ETFs and mutual funds use FIFO economic cost basis and are
    long-only.
  - Any transaction (or edit) that would make a holding negative at any
    point in history is rejected.
  - Negative cash balances are permitted.

### ADR-LEDG-05 — Cash accounts
- **Status:** Accepted · **Source:** Owner · **Phases:** P2B
- **Decision:** A portfolio may have many cash accounts, including several
  in the same currency. Each cash account has exactly one currency.
- **Implemented (P2B):**
  - **Fields:** `cash_accounts` has `id`, `portfolio_id` (FK `RESTRICT`),
    `name` (1–100 characters), `currency` (three letters), and timestamps.
  - **Currency:** fixed at creation (Implementation P2B). Changing it would
    reinterpret recorded movements.
  - **Writes:** every write is one statement guarded by `EXISTS (portfolio
    owned by the caller AND ACTIVE)`. Ownership, the archive rule and the
    write therefore cannot race.
  - **Archived portfolios:** changes return `409 portfolio_archived` (ADR-PORT-03
    interpretation: cash accounts are part of the portfolio's financial
    setup). Reads remain allowed.

### ADR-LEDG-06 — Trade settlement and optional cash movement
- **Status:** Accepted · **Source:** Owner · **Phases:** P3
- **Decision:** A trade settles in the currency of the selected cash
  account. The cash movement for a buy or sell is optional: a transaction
  may be recorded without updating any cash account.
- **P3 (Implementation):**
  - Prices, fees and taxes are in the instrument's quote currency.
  - A selected cash account must use that same currency, otherwise the
    request gets `422`. Trading across currencies needs FX and waits for P4.
  - Buy: cost = quantity × price + fee + tax, and cash moves by −cost.
  - Sell: proceeds = quantity × price − fee − tax, and cash moves by
    +proceeds.
  - Every amount is computed exactly with checked arithmetic. Net proceeds
    may be negative, which records a loss.

### ADR-LEDG-07 — Opening balances
- **Status:** Accepted · **Source:** Owner · **Phases:** P3
- **Decision:** An opening position (one or several FIFO lots) can be
  recorded without a cash movement. An opening cash balance is recorded as
  a ledger transaction.
- **Several lots: Proposed, implemented provisionally in P3.**
  - Each `OPENING_POSITION` transaction is one lot (`quantity`, `amount` =
    total cost basis), dated by its acquisition date. Several lots are
    entered as several transactions.
  - `OPENING_CASH` sets one account's signed starting balance.
  - Awaiting owner confirmation.

### ADR-LEDG-08 — Historical ordering
- **Status:** Accepted · **Source:** Owner · **Phases:** P3
- **Decision:** Ordering uses the transaction date plus an explicit
  `sort_order`. There is no settlement date in the MVP. Historical replay
  follows this order.
- **`sort_order` semantics: Proposed, implemented provisionally in P3.**
  - It is a portfolio-wide integer, default 0, that orders transactions on
    the same `trade_date` (lower first).
  - Remaining ties are broken by the time-ordered `id`, which is entry
    order. `created_at` is not used, because SQLite stores it as text whose
    sub-second formatting does not sort reliably (AGY design review).
  - Awaiting owner confirmation.

### ADR-LEDG-09 — Direct edit and delete with revalidation
- **Status:** Accepted · **Source:** Owner · **Phases:** P3
- **Decision:** Posted transactions can be edited or deleted directly. The
  ledger is **not** append-only and does not require reversal entries.
  Every edit or delete atomically:
  - updates the affected movements
  - recomputes the affected FIFO history
  - validates the resulting historical holdings
  - rejects the change if anything becomes invalid
- **P3 implementation:** each create, replace (PUT) and delete runs as one
  DB transaction:
  1. a guarded write locks the owned, `ACTIVE` portfolio (ADR-DB-09);
  2. references are checked;
  3. the transaction is written and its movements rewritten;
  4. FIFO is replayed from full history for every affected instrument,
     including both the old and the new one on an edit;
  5. the transaction commits.
  If a holding would go negative at any date, the result is
  `409 insufficient_holdings` naming the date, and nothing is written.
  - **Evidence:** a stress test sent 20 concurrent sells of 3 against a
    holding of 10, over 3 rounds. Each round accepted exactly 3 and refused
    17, with no `500`s.
  - **Deleted references:** if a referenced instrument or cash account is
    deleted between the reference check and the insert, the foreign-key
    refusal is returned as `422 validation_failed` instead of `500`.
    SQLite serializes writers, so this can only happen on a database with
    concurrent writers (such as PostgreSQL); it is not covered by a test.

### ADR-LEDG-10 — Economic cost basis only
- **Status:** Accepted · **Source:** Owner · **Phases:** P3, P4
- **Decision:** The system computes economic cost basis, which includes
  trading fees and taxes. It is kept distinct from jurisdiction-specific
  taxable cost basis. There is no country-specific tax reporting engine.

---

## 11. Financial transactions

### ADR-TXN-01 — MVP transaction types
- **Status:** Accepted · **Source:** Owner · **Phases:** P3, P4
- **Decision:** The MVP transaction types are:
  - Buy, Sell
  - Deposit, Withdrawal
  - Dividend / Distribution, Interest
  - Fee, Tax, and Tax Return (or an equivalent correction)
  - Cash Transfer, FX Exchange
  - Stock Dividend, Stock Split, Reverse Split
- **Tax Return (Accepted · Source: Owner, 2026-10-30):** a separate
  transaction kind, `TAX_RETURN`, not a correction of a `TAX`.
  - **Shape** (Implementation P4, the same as `TAX`): `cash_account_id`
    and a positive `amount`, which is credited to the account.
- **Kinds as implemented (P4):**
  - `OPENING_POSITION`, `OPENING_CASH`, `BUY`, `SELL`
  - `DEPOSIT`, `WITHDRAWAL`, `DIVIDEND`, `INTEREST`
  - `FEE`, `TAX`, `TAX_RETURN`
  - `CASH_TRANSFER`, `FX_EXCHANGE`
  - `STOCK_SPLIT`, `STOCK_DIVIDEND`
  - Stock Split and Reverse Split share `STOCK_SPLIT`. Its ratio says
    which one it is, as Snowball Analytics does (ADR-TXN-07).

### ADR-TXN-02 — Fees and taxes
- **Status:** Accepted · **Source:** Owner · **Phases:** P3, P4
- **Decision:**
  - Fees and taxes are entered manually and never auto-calculated. Their
    original amounts are preserved.
  - Applicable transactions have combined fee and tax fields.
  - For trades, they are denominated in the settlement currency.
  - They are included in economic cost basis (buys) and in net proceeds
    (sells).

### ADR-TXN-03 — Cash transfer is distinct from FX exchange
- **Status:** Accepted · **Source:** Owner · **Phases:** P4
- **Decision:** A cash transfer moves money between cash accounts and
  produces balanced cash movements. An FX exchange is a separate operation.
- **P4 `CASH_TRANSFER` (Accepted · Source: Owner, 2026-10-30):**
  - **Fields:** `cash_account_id` (source), `destination_cash_account_id`
    and a positive `amount`.
  - **Movements:** −amount on the source, +amount on the destination.
  - **Accounts:** both belong to the same portfolio, use the same
    currency, and must differ. Moving money across currencies is an
    `FX_EXCHANGE`.
- **Transfer fee:** an optional `fee`, paid by the source on top of
  `amount`, so the source posts −(amount + fee). This was added when the
  fee rules were settled (ADR-TXN-04). Before that, the owner had accepted
  transfers without fee fields.

### ADR-TXN-04 — FX exchange records executed amounts
- **Status:** Accepted · **Source:** Owner · **Phases:** P4
- **Decision:** An FX exchange records the actually executed amounts and
  rate. Executed rates are kept separate from the historical market FX
  rates used for valuation. Fees are not included in the principal amounts.
- **Fee placement and FX shape (Accepted · Source: Owner, 2026-10-30):**
  follow Snowball Analytics. Its `Cash_Convert` records:
  - the amount spent, in the transaction's currency;
  - the amount received, in the other currency;
  - a commission in the spent currency, paid on top of the amount spent
    (example: 82 EUR bought for 100 USD plus a 2.5 USD fee).
  - The rate is not entered; it is implied by the two amounts.
- **P4 `FX_EXCHANGE` (Implementation P4, following that model):**
  - **Fields:** `cash_account_id` (source) with `amount` (spent, > 0);
    `destination_cash_account_id` with `destination_amount`
    (received, > 0); optional `fee` (≥ 0).
  - **Movements:** the source posts −(amount + fee) and the destination
    +destination_amount.
  - **Accounts:** both belong to the same portfolio, in different
    currencies. The same currency is a `CASH_TRANSFER` (`422`).
  - **Rate:** the executed rate is the exact ratio of the two amounts. It
    is not stored separately, because computing it as a decimal would need
    rounding. Market FX rates (P5) are a separate table, and valuation must
    never read executed amounts.
- **Fee currency (Accepted · Source: Owner, 2026-10-30):**
  - The fee is always in the source currency. Snowball also allows a fee
    in another currency (`FeeCurrency`); here such a fee is recorded as a
    separate `FEE` on the right account.
  - Transfer fees (ADR-TXN-03) follow the same rule: Snowball has no
    account-to-account transfer of its own.

### ADR-TXN-05 — Dividends and interest
- **Status:** Accepted · **Source:** Owner · **Phases:** P4
- **Decision:** Dividends and interest record the gross amount, any fee or
  tax, and the net amount. Updating a cash account is optional where the
  transaction rules permit.
- **P4 `DIVIDEND` and `INTEREST` (Accepted · Source: Owner, 2026-10-30):**
  - **Amounts:** the gross amount is entered as `amount` (> 0), with
    optional `fee` and `tax` (≥ 0, default 0). The net amount,
    `amount − fee − tax`, is derived exactly and is not stored; it is the
    cash movement.
  - **Net amount:** may be zero but not negative (`422` on `amount`).
  - **`DIVIDEND`** (which also covers fund distributions) requires
    `instrument_id`. Its `cash_account_id` is optional, like a trade's.
  - **Currency:** a dividend's account must use the instrument's quote
    currency. Dividends paid in another currency are not supported.
  - **`INTEREST`** requires `cash_account_id` (the account the interest
    was earned on, which also gives its currency) and takes no instrument.
  - **No position effect:** neither changes holdings or cost basis.
  - **No holding check:** a dividend is not checked against the holding
    on its date. A payment can arrive after the shares were sold, and the
    MVP has no ex-date or pay date (ADR-LEDG-08).

### ADR-TXN-06 — Corporate actions
- **Status:** Accepted · **Source:** Owner · **Phases:** P4
- **Decision:**
  - **Supported:** stock dividends, stock splits, reverse splits,
    fractional shares and cash in lieu.
  - **Stock dividends** spread the existing economic cost basis across the
    resulting eligible FIFO lots.
  - **Splits** keep total cost basis unchanged while changing the quantity.
  - **Cash in lieu** allocates cost to the fractional quantity and
    realizes economic profit or loss.
  - **Deferred:** spin-offs, mergers, rights issues and return of capital.
- **P4 inputs:**
  - **`STOCK_SPLIT`:** `instrument_id` and `ratio` as `new:old`, for
    example `4:1`, or `1:3` for a one-for-three reverse split.
  - **`STOCK_DIVIDEND`:** `instrument_id` and `quantity`, the number of
    shares received.
  - **Cash in lieu** (optional on both): `cash_in_lieu_quantity` (the
    fraction, in post-action shares) and `amount` (the cash received)
    together, plus an optional `cash_account_id` in the quote currency.
  - **No holding:** both need a holding on their date; otherwise
    `409 no_holdings`. This also applies when an edit or delete removes
    that holding.

### ADR-TXN-07 — Corporate action allocation and rounding details
- **Status:** Accepted (approach) / **Proposed** (rules below) · **Source:** Owner (2026-10-30: "follow Snowball Analytics; where it has no rule, choose what is appropriate"), Implementation (P4) · **Phases:** P4
- **What Snowball Analytics specifies:**
  - A split takes a coefficient: 4 for a four-for-one split, 0.5 for a
    two-to-one reverse split.
  - A stock dividend takes the number of shares received.
  - It documents no lot-level allocation, rounding or cash-in-lieu rules.
- **Rules chosen in P4 (Proposed):** the owner reviewed them on
  2026-10-30 and could not judge them, so they stay Proposed. Before
  relying on cash-in-lieu profit or loss, check these rules against a
  broker statement or a tax adviser.
  1. **Ratio:** a split is entered as `new:old` rather than a single
     coefficient. Ratios such as 1:3 have no exact decimal coefficient.
     `4:1` means the same as Snowball's 4.
  2. **Lots:** a split or stock dividend changes every open FIFO lot's
     quantity in proportion. Each lot keeps its cost and acquisition
     order, so total cost is unchanged and cost per share falls (or rises)
     evenly. Holding dates are preserved.
  3. **New total:**
     - A stock dividend gives exactly held + received.
     - A split gives held × new ÷ old, rounded half-to-even to 8 decimal
       places (the input precision).
  4. **Per-lot rounding:** each lot's new quantity is rounded half-to-even
     to 8 places. The last lot takes the residual, so the lots add up to
     the new total exactly. Costs are never rounded.
  5. **Too small:** a split that would round the holding, or any lot, to
     zero is rejected (`422` on `ratio`).
  6. **Cash in lieu:** the fraction is disposed of through FIFO (oldest
     lots first) at `amount` proceeds. Its cost comes from the existing
     partial-lot rule (ADR-NUM-05, 10 places, exact remainder kept), and
     the difference is realized profit or loss.
- **Storage:** the position movement for a split is `SPLIT` with
  `quantity` = new and `amount` = old. A stock dividend is
  `STOCK_DIVIDEND` with `quantity` = shares received and `amount` = 0.
  Cash in lieu is an ordinary `DISPOSE` posted after it. FIFO replays
  these from full history on every change, so editing an earlier trade
  re-applies later splits.

### ADR-TXN-08 — Standalone cash transactions (P4)
- **Status:** Accepted · **Source:** Owner (2026-10-30), Implementation (P4) · **Phases:** P4
- **Decision:**
  - **Fields:** `DEPOSIT`, `WITHDRAWAL`, `FEE`, `TAX` and `TAX_RETURN`
    each take `cash_account_id` and a positive `amount`.
  - **Direction:** a deposit or tax return posts +amount; the others post
    −amount.
  - **No instrument:** they take no `instrument_id`, so a standalone fee
    or tax never changes a cost basis. Charges that belong in cost basis
    are entered on the trade (ADR-TXN-02).
  - **No fee fields:** deposits and withdrawals have none; a bank charge
    is a separate `FEE`.

---

## 12. Financial precision

### ADR-NUM-01 — No binary floating point for financial values
- **Status:** Accepted · **Source:** Owner · **Phases:** P2B onward
- **Decision:** Exact financial amounts never use `f32`/`f64` or SQLite
  `REAL`. In Rust they use `rust_decimal::Decimal`.
- **Consequences:** `rust_decimal` 1.43 was added in P3, without serde.
  Decimal strings are parsed and formatted explicitly in
  `crate::ledger::decimal`.

### ADR-NUM-02 — Decimal storage in SQLite
- **Status:** Accepted (principle) / **Open** (per-field choice) · **Source:** Owner · **Phases:** P2B onward
- **Decision:**
  - Store decimals as validated canonical decimal TEXT. Use scaled INTEGER
    only for explicitly defined cases.
  - Never let values silently convert to REAL or NUMERIC affinity.
  - Never rely on SQLite numeric aggregation (`SUM`, `AVG`, comparisons)
    over decimal TEXT. Aggregate in Rust.
- **Consequences:** Before choosing a column type, **verify how SeaORM and
  SQLite map `Decimal`**. Do not trust the default mapping. Use a `TEXT`
  column with explicit conversion if that is what it takes to preserve
  exactness.
- **P3 (Implementation):**
  - Ledger decimals are stored in `VARCHAR`/TEXT columns mapped to Rust
    `String`, and converted explicitly. SeaORM's `Decimal` mapping is never
    used. A test asserts SQLite `typeof(...) = 'text'`.
  - Stored text is re-checked against its sign rule when read
    (`from_stored`).
  - The text keeps the scale that was entered (`"100.50"` stays
    `"100.50"`).

### ADR-NUM-03 — Distinct numeric kinds
- **Status:** Accepted · **Source:** Owner · **Phases:** P2B onward
- **Decision:** Money, quantity, price and rate are distinct kinds, each
  with its own validation and limits.
- **Consequences:** Model them with distinct types (for example newtypes)
  so they cannot be mixed up.

### ADR-NUM-04 — Precision limits
- **Status:** **Proposed, implemented provisionally in P3** · **Phases:** P3
- **Decision (provisional):**
  - **Limits:** every input decimal has at most 8 digits after the point
    and at most 15 significant digits before it.
  - **Sign rules:** quantity > 0; price ≥ 0; fee, tax and cost basis ≥ 0;
    opening cash is signed.
  - **Grammar:** strict. An optional `-`, digits, and an optional `.digits`.
    `+`, exponents, `_` separators and JSON numbers are rejected.
  - **Derived values:** these may have more decimals. For example
    quantity × price can reach 16, and they are stored exactly.
  - **Minor units:** not enforced per currency.
- Awaiting owner confirmation.

### ADR-NUM-05 — Rounding policy
- **Status:** Accepted (principle) / **Open** (per-operation rules) · **Source:** Owner · **Phases:** P3–P5
- **Decision:** Input precision is preserved. Intermediate results are
  never rounded arbitrarily. Rounding happens only at defined boundaries,
  such as display, settlement amounts and allocation residuals, and every
  operation validates against overflow.
- **Open:** the rounding mode and scale for each operation.
- **P5 valuation (Accepted · Source: Owner, 2026-10-30):**
  - quantity × price and multiplying by an FX rate are exact; nothing is
    rounded along the way;
  - a base-currency value is rounded half-to-even to 8 places, once, at
    the end;
  - converting through an inverse pair (dividing by the rate) rounds
    half-to-even to 8 places;
  - allocation percentages round half-to-even to 4 places (`12.3457`) and
    are not forced to add up to exactly 100.
- **P4 operations: Proposed, implemented provisionally.**
  - Cash transactions, transfers and FX exchanges are exact sums; nothing
    is rounded.
  - Splits and stock dividends round holdings half-to-even to 8 places,
    with the residual in the last lot. Cost is never rounded (ADR-TXN-07).
  - The FX executed rate is not computed, so it is never rounded.
- **P3 FIFO allocation: Proposed, implemented provisionally.**
  - **No rounding needed:** trade amounts are exact, with no rounding.
  - **Whole lot:** consuming a whole lot takes its remaining cost exactly.
  - **Partial lot:** consuming part of a lot takes
    `cost × taken / quantity`, rounded half-to-even to 10 decimal places.
  - **Conservation:** the lot keeps the exact remainder, so allocated cost
    plus open cost always equals acquired cost.
  - **Verification:** checked by an independent AGY spec suite with
    hand-derived values, and by mutation testing.
  - **Exact arithmetic everywhere else (Proposed):** `rust_decimal`
    silently rounds a result that needs more than 28 significant digits.
    So trade amounts (quantity × price ± fee ± tax) and every FIFO running
    total (open quantity, open cost, lot remainders, cost basis, realized
    P/L) are computed exactly on the integer mantissa. If the exact result
    cannot be represented, the write is rejected with
    `422 validation_failed` instead of being rounded. The deliberate
    10-place allocation rounding above is the only rounding in P3.
  - Awaiting owner confirmation.

---

## 13. Valuation and FX

### ADR-VAL-01 — On-demand valuation at an explicit `as_of` date
- **Status:** Accepted · **Source:** Owner · **Phases:** P5
- **Decision:** Valuations are computed on request for an explicit `as_of`
  date. No authoritative valuation is stored.

### ADR-VAL-02 — Price lookup
- **Status:** Accepted · **Source:** Owner · **Phases:** P5
- **Decision:** Valuation uses the latest price on or before `as_of`, and
  returns that price's date, its source and a staleness indicator. Missing
  prices are never fabricated.
- **Staleness threshold (Accepted · Source: Owner, 2026-10-30):**
  - A price is stale when its date is more than **7** calendar days
    before `as_of`.
  - An FX rate is stale when its date is more than **3** calendar days
    before `as_of`.
  - Stale values are still used; they are only flagged.

### ADR-VAL-03 — Historical FX rates
- **Status:** Accepted · **Source:** Owner · **Phases:** P5
- **Decision:**
  - FX rates are historical and user-scoped. Valuation uses the latest
    valid rate on or before `as_of`.
  - Direct and inverse pairs are both supported. Cross-rate triangulation
    is excluded.
  - The quote direction is always explicit.
  - A current rate is never applied to an old valuation date.

### ADR-VAL-04 — Partial valuation
- **Status:** Accepted · **Source:** Owner · **Phases:** P5
- **Decision:** When a price or FX rate is missing, the valuation:
  - returns totals for the assets that could be valued
  - lists the unvalued assets separately, with a reason for each
  - marks the result as partial
- **Consequences:** Missing assets are never valued at zero.

### ADR-VAL-05 — Signed allocation on Net Portfolio Value
- **Status:** Accepted · **Source:** Owner · **Phases:** P5
- **Decision:**
  - Allocation by asset class uses market value in the base currency,
    divided by Net Portfolio Value (NPV). Percentages are signed.
  - Example: stocks USD 10,000 and cash USD −2,000 give NPV USD 8,000, so
    stocks are 125% and cash is −25%.
  - If NPV ≤ 0, the percentages are `null`.
  - Negative cash is flagged, and is not interpreted as a margin loan.
  - Partial valuations do not present allocation as complete.

### ADR-VAL-06 — Price and FX-rate storage (P5)
- **Status:** Accepted · **Source:** Owner (2026-10-30), Implementation (P5) · **Phases:** P5, P6
- **Schema** (migration `m20261030_000009_create_market_data`):
  - **`instrument_prices`:** `instrument_id` (`ON DELETE RESTRICT`),
    date-only `price_date`, decimal-text `price`, `source`. Unique on
    (instrument, date). Prices belong to their instrument's owner.
  - **`fx_rates`:** `owner_user_id` (`RESTRICT`), `base_currency`,
    `quote_currency`, date-only `rate_date`, decimal-text `rate`, `source`.
    Unique on (owner, base, quote, date).
  - **`source`:** `MANUAL` (entered through the API) or `IMPORT` (P6), with
    a `CHECK` constraint (ADR-DB-08). The API always sets `MANUAL`; an
    edit keeps the original source.
- **Values:**
  - **Price:** in the instrument's quote currency, ≥ 0 (zero allows a
    worthless or delisted holding).
  - **Rate:** > 0. One `base_currency` is worth `rate` units of
    `quote_currency`. Base and quote must differ.
  - Both use the ledger's decimal limits (8 places, 15 integer digits;
    ADR-NUM-04).
  - **Dates:** 1900-01-01 to 2100-12-31, like trade dates.
- **Pairs:** `USD/THB` and `THB/USD` are different pairs, so both can be
  stored for one date.
- **Direct or inverse (Accepted · Source: Owner, 2026-10-30):** to convert
  USD to THB, valuation takes the latest-dated of the direct (`USD/THB`)
  and inverse (`THB/USD`) rates on or before `as_of`. On the same date the
  direct rate wins. An inverse rate is divided by, rounding half-to-even to
  8 places (ADR-NUM-05).
- **API:** `/api/v1/instruments/{id}/prices[/{price_id}]` and
  `/api/v1/fx-rates[/{id}]`, with create, list (latest first, filtered by
  inclusive `from`/`to` dates and, for rates, by currency), get, replace
  (`PUT`) and delete.
  - A duplicate date returns `409 price_exists` or `fx_rate_exists`; the
    unique indexes enforce it under concurrency too.
  - An instrument with prices cannot be deleted (`409 instrument_in_use`).
  - Another user's instrument, price or rate is `404`.

### ADR-VAL-07 — Valuation details (P5)
- **Status:** Accepted · **Source:** Owner (2026-10-30), Implementation (P5) · **Phases:** P5
- **Context:** these details apply the accepted rules (ADR-VAL-01 … VAL-06,
  ADR-NUM-05) where those rules leave a choice open.
- **API:** `GET /api/v1/portfolios/{id}/holdings`, `/valuation` and
  `/allocation`, each with a required `as_of` (`422` when missing). They
  work on archived portfolios.
- **Holdings:**
  - FIFO is replayed over movements dated on or before `as_of`. The ledger
    keeps every prefix of history valid, so the replay cannot fail.
  - Closed positions (quantity 0) are left out.
  - Every cash account is listed, even with a zero balance.
  - Cost basis is the open lots' cost in the quote currency (ADR-LEDG-10).
- **Values:**
  - `market_value` (quantity × price, quote currency) is exact.
  - Each base-currency `value` is rounded once to 8 places.
  - **Totals** are exact sums of those rounded values, so items always add
    up to the totals.
  - **Zero cash balance:** valued at exactly 0 without needing a rate.
- **Unvalued reasons:** `MISSING_PRICE`, `MISSING_FX_RATE`, and
  `OUT_OF_RANGE` (an exact value too large to represent). They appear on
  the item and in a separate `unvalued` list. A total too large to
  represent is `422` on `as_of`.
- **Allocation:**
  - The classes are the instrument asset classes plus `CASH`; only classes
    with a valued asset are listed.
  - Percentages are over the valued Net Portfolio Value. A partial
    valuation gives `partial: true` rather than hiding them.
  - `negative_cash` is true when any cash account is overdrawn at `as_of`.
- **Snowball Analytics (ADR-SCOPE-08):** Snowball values a portfolio as
  holdings (price × quantity) plus cash, which matches the above
  ([key portfolio indicators](https://help.snowball-analytics.com/review-of-key-portfolio-indicators/)).
  It documents no rules for rounding, missing prices or allocation with
  negative cash, so the details above were proposed here and accepted by
  the owner.

---

## 14. CSV import and export

### ADR-CSV-01 — Import and export types
- **Status:** Accepted · **Source:** Owner · **Phases:** P6
- **Decision:** Import covers transactions, historical instrument prices
  and historical FX rates. CSV export is supported. Both respect ownership
  (ADR-OWN-03).

### ADR-CSV-02 — Staged synchronous workflow
- **Status:** Accepted · **Source:** Owner · **Phases:** P6
- **Decision:** Import runs as upload → preview → map → validate →
  resolve → commit.
  - **Mapping:** dynamic column mapping, plus configurable date and
    decimal formats.
  - **Errors:** reported per row.
  - **Execution:** synchronous in the MVP.
  - **Limits:** 20 MB per file and 50,000 rows.
- **Open detail:** where import state lives between steps, for example a
  server-side staging table versus a stateless re-submit.

### ADR-CSV-03 — Duplicate handling
- **Status:** Accepted · **Source:** Owner · **Phases:** P6
- **Decision:**
  - A source-scoped external ID identifies a confirmed duplicate.
  - A fingerprint flags a potential duplicate.
  - When identity is uncertain, the user reviews it manually.
  - Imports are never merged automatically with manually entered
    transactions.

### ADR-CSV-04 — Atomic commit with ledger revalidation
- **Status:** Accepted · **Source:** Owner · **Phases:** P6
- **Decision:** The user-selected valid rows are committed in one atomic
  operation, after FIFO and ledger invariants are revalidated against the
  combined history.

### ADR-CSV-05 — No broker adapter framework
- **Status:** Accepted · **Source:** Owner · **Phases:** P6
- **Decision:** No broker-specific adapter framework in the MVP. Generic
  column mapping only.

---

## 15. Operations and quality

### ADR-OPS-01 — Configuration via environment variables
- **Status:** Accepted · **Source:** Owner (policy), Implementation (P0/P1) (variables) · **Phases:** all
- **Decision:**
  - All configuration comes from environment variables, with optional
    `.env` loading. It is validated at startup, and the server refuses to
    start if any value is invalid.
  - Secrets are never logged or echoed in error messages, and `Debug`
    output redacts them.
  - The current variables are listed in [`../README.md`](../README.md).

### ADR-OPS-02 — Container defaults
- **Status:** Accepted · **Source:** Implementation (P0/P1) · **Phases:** all
- **Decision:**
  - **Image:** multi-stage build that runs as a non-root user (uid 10001).
  - **Storage:** SQLite on the `merton-data` named volume at `/app/data`.
  - **Network:** Compose publishes the port on `127.0.0.1` only.
  - **Secrets:** Compose refuses to start without `APP_SECRET_KEY`.

### ADR-QA-01 — Risk-based testing with incremental development
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:**
  - 100% coverage is not required.
  - Security boundaries and financial invariants **must** be covered
    meaningfully.
  - Each feature ships its tests in the same phase.
  - Required kinds of tests: unit, HTTP integration, SQLite integration,
    financial invariants, cross-user authorization, import atomicity,
    migrations, concurrency (where relevant), and OpenAPI consistency.

### ADR-QA-02 — CI gate
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** GitHub Actions (`.github/workflows/ci.yml`) runs these on
  every push to `main` and on every pull request:
  - `cargo fmt --all -- --check`
  - `cargo check --all-targets --locked`
  - `cargo clippy --all-targets --locked -- -D warnings`
  - `cargo test --locked`

### ADR-QA-03 — Isolated tests
- **Status:** Accepted · **Source:** Implementation (P1) · **Phases:** all
- **Decision:** Each integration test builds its own in-memory SQLite
  database (single connection) and its own `AppState`, through
  `tests/common`. There is no shared global state. Concurrency tests use
  barriers or channels, not sleeps.

### ADR-AGENT-01 — Multi-agent development
- **Status:** Accepted · **Source:** Owner · **Phases:** all
- **Decision:** The parent agent (team leader) owns architecture decisions
  and integration. Workers get scoped tasks, preferably read-only analysis
  in parallel. Agents never edit the same files at the same time; parallel
  edits use isolated worktrees. Worker output is reviewed and verified
  with real tests. A worker's result is never reported unless that worker
  actually ran. Nothing is committed, pushed or opened as a PR
  automatically.

---

## 16. Open decisions summary

| ID | Topic | Blocks |
| --- | --- | --- |
| ADR-ARCH-03 | Feature-first vs layer-first module organization (P2A/P2B used the current hybrid) | P3 onward |
| ADR-API-04 | Confirm page size 50 / 200 (implemented provisionally) | Confirmation only |
| ADR-PORT-03 | Confirm archived portfolios cannot be updated (implemented provisionally) | Confirmation only |
| ADR-PORT-05 | Confirm format-only currency validation (implemented provisionally) | Confirmation only |
| ADR-PORT-04 | Confirm unused cash accounts are deleted with their portfolio (implemented provisionally) | Confirmation only |
| ADR-INST-04 | Confirm bank deposits are plain cash accounts (implemented provisionally) | Confirmation only |
| ADR-INST-02 | Confirm deferring the instrument metadata column (implemented provisionally) | Confirmation only |
| ADR-NUM-04 | Confirm decimal limits (8 dp, 15 integer digits) (implemented provisionally) | Confirmation only |
| ADR-LEDG-02 | Confirm the P3 ledger schema (implemented provisionally) | Confirmation only |
| ADR-LEDG-07 | Confirm one OPENING_POSITION per lot (implemented provisionally) | Confirmation only |
| ADR-LEDG-08 | Confirm `sort_order` semantics (implemented provisionally) | Confirmation only |
| ADR-NUM-05 | Confirm FIFO allocation rounding (implemented provisionally) | Confirmation only |
| ADR-LEDG-02 | Confirm the P4 schema: `destination_cash_account_id`, `destination_amount`, `ratio`, `cash_in_lieu_quantity`, and the `SPLIT`/`STOCK_DIVIDEND` movements (implemented provisionally) | Confirmation only |
| ADR-TXN-07 | Review the corporate-action rules chosen under the owner's delegation (`new:old` ratio, proportional lots, 8-place rounding, residual to the last lot, cash in lieu through FIFO). Owner could not judge them on 2026-10-30. | Confirmation only |
| ADR-NUM-05 | Confirm the P4 rounding (holdings to 8 places after splits) | Confirmation only |
| ADR-CSV-02 | Import staging model | P6 (deferred by the owner until the rest of the MVP is done) |
| ADR-AUTH-10 | Account recovery | Before production use |
| ADR-AUTH-11 | Expired session cleanup | Before production use |
| — | Performance limits for large ledger replay | P3/P5 |
