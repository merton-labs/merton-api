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
- **Status:** Accepted (mechanism) / **Open** (limits) · **Source:** Owner · **Phases:** P2A onward
- **Decision:** List endpoints accept `limit` and `offset`, return results in
  a stable deterministic order (with a unique tie-breaker, e.g. `id`), and
  enforce a maximum page size.
- **Open:** the default and maximum page size. *Proposed:* default 50, maximum 200.

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
- **Open detail:** whether name or base-currency updates are allowed while
  archived. See [PROJECT_PLAN.md §22](PROJECT_PLAN.md#22-open-questions).

### ADR-PORT-04 — Deletion only without financial history
- **Status:** Accepted · **Source:** Owner · **Phases:** P2A onward
- **Decision:** A portfolio can be permanently deleted only if it has no
  financial history. Posted financial records are never cascade-deleted.
- **Consequences:**
  - Foreign keys from financial tables to `portfolios` must use `RESTRICT`
    (or `NO ACTION`), never `CASCADE`.
  - The delete use case must check every financial table that exists at
    the time; each new phase extends that check.
  - Deleting a portfolio with history returns `409`.

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

### ADR-INST-03 — Fixed quote currency
- **Status:** Accepted · **Source:** Owner · **Phases:** P2B
- **Decision:** Each instrument has one fixed quote currency. Different
  listings or currencies of the same security are separate instruments
  where appropriate.

### ADR-INST-04 — Fixed asset classes
- **Status:** Accepted · **Source:** Owner · **Phases:** P2B, P5
- **Decision:** The asset classes are `STOCK`, `ETF`, `MUTUAL_FUND` and
  `CASH`. Thai and international stocks are both `STOCK`.
- **Open detail:** how bank deposits are represented. *Technical
  interpretation:* as cash accounts in the `CASH` class. Confirm, in
  particular for fixed-term deposits.

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

### ADR-LEDG-06 — Trade settlement and optional cash movement
- **Status:** Accepted · **Source:** Owner · **Phases:** P3
- **Decision:** A trade settles in the currency of the selected cash
  account. The cash movement for a buy or sell is optional: a transaction
  may be recorded without updating any cash account.

### ADR-LEDG-07 — Opening balances
- **Status:** Accepted · **Source:** Owner · **Phases:** P3
- **Decision:** An opening position (one or several FIFO lots) can be
  recorded without a cash movement. An opening cash balance is recorded as
  a ledger transaction.

### ADR-LEDG-08 — Historical ordering
- **Status:** Accepted · **Source:** Owner · **Phases:** P3
- **Decision:** Ordering uses the transaction date plus an explicit
  `sort_order`. There is no settlement date in the MVP. Historical replay
  follows this order.
- **Open detail:** the exact `sort_order` semantics (scope, default, and how
  ties are broken).

### ADR-LEDG-09 — Direct edit and delete with revalidation
- **Status:** Accepted · **Source:** Owner · **Phases:** P3
- **Decision:** Posted transactions can be edited or deleted directly. The
  ledger is **not** append-only and does not require reversal entries.
  Every edit or delete atomically:
  - updates the affected movements
  - recomputes the affected FIFO history
  - validates the resulting historical holdings
  - rejects the change if anything becomes invalid

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
- **Open detail:** how "Tax Return" is modelled, as its own type or as a
  correction of a Tax.

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

### ADR-TXN-04 — FX exchange records executed amounts
- **Status:** Accepted · **Source:** Owner · **Phases:** P4
- **Decision:** An FX exchange records the actually executed amounts and
  rate. Executed rates are kept separate from the historical market FX
  rates used for valuation. Fees are not included in the principal amounts.
- **Open detail:** the exact rules for charging transfer or exchange fees
  to the source or destination account have been referenced but not
  written down. They must be specified before P4.

### ADR-TXN-05 — Dividends and interest
- **Status:** Accepted · **Source:** Owner · **Phases:** P4
- **Decision:** Dividends and interest record the gross amount, any fee or
  tax, and the net amount. Updating a cash account is optional where the
  transaction rules permit.

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

### ADR-TXN-07 — Corporate action allocation and rounding details
- **Status:** **Open** · **Phases:** P4
- **Decision:** Not finalized:
  - how cost is allocated across lots for stock dividends and cash in lieu
  - the lot-level rounding policy
  - where rounding residuals go
- **Consequences:** Write these as explicit rules and get them approved
  before implementing P4. Do not silently assume them.

---

## 12. Financial precision

### ADR-NUM-01 — No binary floating point for financial values
- **Status:** Accepted · **Source:** Owner · **Phases:** P2B onward
- **Decision:** Exact financial amounts never use `f32`/`f64` or SQLite
  `REAL`. In Rust they use `rust_decimal::Decimal`.
- **Consequences:** `rust_decimal` has not been added yet; add it in the
  first phase that stores a financial value.

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

### ADR-NUM-03 — Distinct numeric kinds
- **Status:** Accepted · **Source:** Owner · **Phases:** P2B onward
- **Decision:** Money, quantity, price and rate are distinct kinds, each
  with its own validation and limits.
- **Consequences:** Model them with distinct types (for example newtypes)
  so they cannot be mixed up.

### ADR-NUM-04 — Precision limits
- **Status:** **Open** · **Phases:** P2B, P3
- **Decision:** The maximum scale and integer digits for each kind, and
  the supported currency minor units, are undecided.

### ADR-NUM-05 — Rounding policy
- **Status:** Accepted (principle) / **Open** (per-operation rules) · **Source:** Owner · **Phases:** P3–P5
- **Decision:** Input precision is preserved. Intermediate results are
  never rounded arbitrarily. Rounding happens only at defined boundaries,
  such as display, settlement amounts and allocation residuals, and every
  operation validates against overflow.
- **Open:** the rounding mode and scale for each operation.

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
- **Open detail:** the staleness threshold.

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
| ADR-ARCH-03 | Feature-first vs layer-first module organization | P2A (or explicit approval to continue the current hybrid) |
| ADR-API-04 | Default and maximum page size | P2A |
| ADR-PORT-03 | Whether archived portfolios may be renamed or change base currency | P2A |
| ADR-INST-04 | Representation of bank deposits (incl. fixed-term) | P2B |
| ADR-NUM-02 / NUM-04 | Decimal column types and precision limits per numeric kind | P2B |
| ADR-LEDG-02 | Physical ledger schema | P3 |
| ADR-LEDG-08 | `sort_order` semantics | P3 |
| ADR-NUM-05 | Per-operation rounding rules | P3–P5 |
| ADR-TXN-01 | Modelling of Tax Return | P4 |
| ADR-TXN-04 | Fee placement rules for transfers and FX exchanges | P4 |
| ADR-TXN-07 | Stock-dividend and cash-in-lieu allocation and rounding | P4 |
| ADR-VAL-02 | Price/FX staleness threshold | P5 |
| ADR-CSV-02 | Import staging model | P6 |
| ADR-AUTH-10 | Account recovery | Before production use |
| ADR-AUTH-11 | Expired session cleanup | Before production use |
| — | Performance limits for large ledger replay | P3/P5 |
