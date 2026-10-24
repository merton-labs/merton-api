# Merton API

Backend for **Merton**, an open-source, self-hosted, multi-user investment portfolio tracker.

> **Status: authentication.** The server foundation plus local email/password
> accounts with server-managed cookie sessions are implemented. No portfolio
> or financial features have been built yet.

## Technology stack

| Concern       | Choice                                         |
| ------------- | ---------------------------------------------- |
| Language      | Rust (stable, edition 2024, MSRV 1.94)         |
| HTTP          | Axum 0.8, Tower / tower-http                   |
| Runtime       | Tokio                                          |
| Database      | SQLite via SeaORM 2 (PostgreSQL planned)       |
| Migrations    | SeaORM Migration (inside this crate)           |
| OpenAPI       | utoipa                                         |
| Passwords     | Argon2id (`argon2`, OWASP default parameters)  |
| Logging       | tracing + tracing-subscriber                   |
| Deployment    | Docker + Docker Compose                        |

## Prerequisites

- Rust stable 1.94 or newer (`rustup`, with `rustfmt` and `clippy`)
- Docker with Compose v2, only for running in containers

## Local development

```bash
cp .env.example .env
# Set APP_SECRET_KEY in .env, e.g. to the output of: openssl rand -base64 48
cargo run
```

Then open the API:

```bash
curl http://127.0.0.1:8080/api/v1/health
# {"status":"ok"}
```

The SQLite database is created at `data/merton.db`. Pending migrations run
automatically before the server starts accepting requests.

### Endpoints

| Method | Path                    | Auth            | Description                                         |
| ------ | ----------------------- | --------------- | --------------------------------------------------- |
| GET    | `/api/v1/health`        | –               | `200 {"status":"ok"}`, or `503` if the database is unreachable |
| GET    | `/api/v1/openapi.json`  | –               | OpenAPI document                                    |
| POST   | `/api/v1/auth/register` | –               | Create an account (`201`); does not log in          |
| POST   | `/api/v1/auth/login`    | –               | Log in; sets the session cookie (`200`)             |
| POST   | `/api/v1/auth/logout`   | cookie + CSRF   | Revoke the session and clear the cookie (`204`)     |
| GET    | `/api/v1/auth/me`       | cookie          | Current user `{id, email}`                          |
| GET    | `/api/v1/auth/csrf`     | cookie          | CSRF token for the current session                  |

Errors are [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457) Problem Details
(`application/problem+json`) with a stable `code` field, for example
`invalid_credentials`, `email_taken`, `validation_failed`, `csrf_invalid`,
`untrusted_origin`, `rate_limited`.

A browser client logs in, then calls `GET /api/v1/auth/csrf` and sends the
returned token as `X-CSRF-Token` on authenticated state-changing requests.

## Configuration

All configuration comes from environment variables. A `.env` file in the
working directory is loaded if present.

| Variable                   | Default                                       | Description                              |
| -------------------------- | --------------------------------------------- | ---------------------------------------- |
| `APP_HOST`                 | `127.0.0.1`                                   | IP address to bind                       |
| `APP_PORT`                 | `8080`                                        | Port to bind                             |
| `DATABASE_URL`             | `sqlite://data/merton.db`                     | Database URL                             |
| `DATABASE_MAX_CONNECTIONS` | `5`                                           | Maximum pool size (must be > 0)          |
| `DATABASE_BUSY_TIMEOUT_MS` | `5000`                                        | SQLite busy timeout in milliseconds      |
| `APP_SECRET_KEY`           | **required**                                  | Server secret, ≥ 32 bytes; keep it stable across restarts |
| `APP_TRUSTED_ORIGINS`      | `http://localhost:8080,http://127.0.0.1:8080` | Comma-separated origins allowed to send mutating browser requests |
| `SESSION_COOKIE_SECURE`    | `true`                                        | `false` only allowed when every trusted origin is loopback HTTP |
| `TRUSTED_PROXIES`          | empty                                         | Comma-separated proxy IPs whose `X-Forwarded-For` is trusted |
| `RUST_LOG`                 | `merton_api=info,tower_http=info,sqlx=warn`   | Log filter (`tracing` EnvFilter syntax)  |

SQLite connections always enable foreign keys and WAL journaling, and the
database file is created if it does not exist.

Behind a reverse proxy, set `APP_TRUSTED_ORIGINS` to the public URL (for
example `https://merton.example.com`) and `TRUSTED_PROXIES` to the proxy's IP
address. Serve the public site over HTTPS and keep `SESSION_COOKIE_SECURE=true`.

## Security model

- **Passwords**: 12–128 characters, hashed with Argon2id and a random salt.
  Passwords and hashes are never logged or returned.
- **Emails**: unquoted ASCII addresses with a dotted domain. Uniqueness uses a
  normalized form (trimmed, lowercased); the original is kept for display.
- **Sessions**: 256-bit random tokens in an `HttpOnly`, `SameSite=Lax`,
  `Secure`, `__Host-merton_session` cookie. Only the SHA-256 of the token is
  stored. Sessions expire after 7 days idle or 30 days total, enforced on the
  server. Activity extends the idle window at most once every 15 minutes, to
  avoid a database write per request. Logging out revokes the session, and
  logging in revokes any session the browser already presented.
- **CSRF**: register, login and logout require a trusted `Origin` (or a
  same-origin `Sec-Fetch-Site`; clients that send neither are not browsers).
  Logout additionally requires `X-CSRF-Token`: an HMAC-SHA256 of the session id
  under `APP_SECRET_KEY`, verified in constant time. No CORS headers are sent.
- **Login rate limiting**: 5 failed logins per account and 20 attempts
  (login and registration) per client IP (IPv6 grouped by /64) per 15 minutes.
  Exceeding either returns `429` with `Retry-After`. Unknown accounts are
  checked against a dummy Argon2 hash so response times do not reveal whether
  an email is registered.

### Known limitations

- Rate limits are kept in memory per process: they reset on restart and are
  not shared between instances. Running more than one instance needs a shared
  store.
- `X-Forwarded-For` is honoured only from exact `TRUSTED_PROXIES` addresses
  (no CIDR ranges).
- There is no account recovery or password reset yet. This is a required
  follow-up for self-hosted operation; until then an operator must manage
  accounts directly in the database.
- Expired and revoked session rows are not purged automatically yet.
- Registration reveals whether an email is already registered (`409`), as
  required by the API contract.

## Docker

```bash
export APP_SECRET_KEY="$(openssl rand -base64 48)"   # or put it in .env
docker compose up --build     # build and start
docker compose down           # stop (data is kept)
docker compose down -v        # stop and delete the data volume
```

The API is published on `http://127.0.0.1:8080`. SQLite data is stored in the
`merton-data` named volume, mounted at `/app/data`. The container runs as a
non-root user. Compose refuses to start without `APP_SECRET_KEY`.

## Tests and checks

```bash
cargo fmt --all -- --check
cargo check --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Tests use an isolated in-memory SQLite database per test and need no external
services. CI runs the same checks on every push to `main` and every pull request.

## Project layout

```text
src/
├── main.rs              # Process entry: .env, logging, runtime
├── lib.rs
├── app.rs               # Wiring: state, database, migrations, server, shutdown
├── config.rs            # Environment configuration
├── auth/                # Authentication, independent of HTTP
│   ├── email.rs         #   Validation and normalization
│   ├── password.rs      #   Policy and Argon2id hashing
│   ├── session.rs       #   Tokens and expiry policy
│   ├── csrf.rs          #   Session-bound HMAC tokens
│   ├── rate_limit.rs    #   In-memory login limits
│   ├── repository.rs    #   User and session queries
│   └── service.rs       #   Register, login, authenticate, logout
├── http/                # Router, handlers, extractors, errors, OpenAPI
└── infrastructure/
    ├── database.rs      # Connection pool setup
    ├── entities/        # SeaORM entities
    └── migrations/      # SeaORM migrations
tests/                   # Integration tests
```

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
