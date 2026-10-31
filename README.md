# Merton API

Backend for **Merton**, an open-source, self-hosted, multi-user investment portfolio tracker.

> **Status: ledger.** The server foundation, local email/password accounts
> with server-managed cookie sessions, private portfolios, instruments, cash
> accounts, and the transaction ledger (opening balances, buys and sells
> with FIFO validation, deposits, withdrawals, dividends, interest, fees,
> taxes and tax returns, cash transfers, FX exchanges, stock splits and
> stock dividends) are implemented, along with manually entered
> historical prices and FX rates, and holdings, valuation and allocation
> at any date. CSV import and export are not built yet. See
> [docs/PROJECT_PLAN.md](docs/PROJECT_PLAN.md) for the roadmap and
> [CLAUDE.md](CLAUDE.md) for contributor rules.

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
| GET    | `/api/v1/portfolios`    | cookie          | List own portfolios (`limit` 1–200, default 50; `offset`; `status`) |
| POST   | `/api/v1/portfolios`    | cookie + CSRF   | Create a portfolio `{name, base_currency}` (`201` + `Location`) |
| GET    | `/api/v1/portfolios/{id}` | cookie        | Get an own portfolio                                |
| PATCH  | `/api/v1/portfolios/{id}` | cookie + CSRF | Rename or change base currency (active only)        |
| POST   | `/api/v1/portfolios/{id}/archive` | cookie + CSRF | Archive (read-only until restored)        |
| POST   | `/api/v1/portfolios/{id}/restore` | cookie + CSRF | Restore to active                         |
| DELETE | `/api/v1/portfolios/{id}` | cookie + CSRF | Delete a portfolio without financial history, with its unused cash accounts (`204`) |
| GET    | `/api/v1/instruments`   | cookie          | List own instruments (`limit`, `offset`, `asset_class`) |
| POST   | `/api/v1/instruments`   | cookie + CSRF   | Create `{asset_class, name, symbol?, isin?, exchange?, quote_currency}` |
| GET    | `/api/v1/instruments/{id}` | cookie       | Get an own instrument                               |
| PATCH  | `/api/v1/instruments/{id}` | cookie + CSRF | Update descriptive fields; `null` clears symbol/isin/exchange; quote currency is fixed |
| DELETE | `/api/v1/instruments/{id}` | cookie + CSRF | Delete an unused instrument (`204`)                |
| GET    | `/api/v1/instruments/{id}/prices` | cookie | List an instrument's prices, latest first (`limit`, `offset`, `from`, `to`) |
| POST   | `/api/v1/instruments/{id}/prices` | cookie + CSRF | Record `{price_date, price}` in the quote currency; one per date |
| GET    | `/api/v1/instruments/{id}/prices/{price_id}` | cookie | Get a price |
| PUT    | `/api/v1/instruments/{id}/prices/{price_id}` | cookie + CSRF | Replace a price's date and value |
| DELETE | `/api/v1/instruments/{id}/prices/{price_id}` | cookie + CSRF | Delete a price (`204`) |
| GET    | `/api/v1/fx-rates` | cookie | List own FX rates, latest first (`limit`, `offset`, `base_currency`, `quote_currency`, `from`, `to`) |
| POST   | `/api/v1/fx-rates` | cookie + CSRF | Record `{base_currency, quote_currency, rate_date, rate}`: 1 base = `rate` quote; one per pair and date |
| GET    | `/api/v1/fx-rates/{id}` | cookie | Get an FX rate |
| PUT    | `/api/v1/fx-rates/{id}` | cookie + CSRF | Replace an FX rate |
| DELETE | `/api/v1/fx-rates/{id}` | cookie + CSRF | Delete an FX rate (`204`) |
| GET    | `/api/v1/portfolios/{id}/cash-accounts` | cookie | List a portfolio's cash accounts           |
| POST   | `/api/v1/portfolios/{id}/cash-accounts` | cookie + CSRF | Create `{name, currency}` (active portfolios only) |
| GET    | `/api/v1/portfolios/{id}/cash-accounts/{account_id}` | cookie | Get a cash account          |
| PATCH  | `/api/v1/portfolios/{id}/cash-accounts/{account_id}` | cookie + CSRF | Rename (currency is fixed) |
| DELETE | `/api/v1/portfolios/{id}/cash-accounts/{account_id}` | cookie + CSRF | Delete an account with no movements (`204`) |
| GET    | `/api/v1/portfolios/{id}/transactions` | cookie | List transactions in ledger order (`limit`, `offset`, `instrument_id`) |
| POST   | `/api/v1/portfolios/{id}/transactions` | cookie + CSRF | Record a transaction (kinds below) |
| GET    | `/api/v1/portfolios/{id}/transactions/{transaction_id}` | cookie | Get a transaction |
| PUT    | `/api/v1/portfolios/{id}/transactions/{transaction_id}` | cookie + CSRF | Replace a transaction; history is re-validated |
| DELETE | `/api/v1/portfolios/{id}/transactions/{transaction_id}` | cookie + CSRF | Delete a transaction; history is re-validated |
| GET    | `/api/v1/portfolios/{id}/holdings?as_of=` | cookie | Open positions (quantity, cost basis) and cash balances at a date |
| GET    | `/api/v1/portfolios/{id}/valuation?as_of=` | cookie | Value in the base currency, with the price and FX rate used for each asset |
| GET    | `/api/v1/portfolios/{id}/allocation?as_of=` | cookie | Signed allocation by asset class on Net Portfolio Value |

Errors are [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457) Problem Details
(`application/problem+json`) with a stable `code` field, for example
`invalid_credentials`, `email_taken`, `validation_failed`, `csrf_invalid`,
`untrusted_origin`, `rate_limited`, `service_overloaded`, `not_found`,
`portfolio_archived`, `portfolio_not_archived`, `portfolio_has_history`,
`instrument_in_use`, `cash_account_in_use`, `insufficient_holdings`,
`no_holdings`, `price_exists`, `fx_rate_exists`.

Instruments use the asset classes `STOCK`, `ETF` and `MUTUAL_FUND`. Their
symbol, ISIN and exchange are optional and need not be unique, and ISINs
are checked against their check digit. Cash, including bank deposits, lives
in cash accounts: a portfolio can have several, even in the same currency.

Prices and FX rates are entered by hand (CSV import comes later); nothing is
fetched automatically. A price is in its instrument's quote currency and
may be zero; a rate must be greater than zero. Market FX rates are separate
from the amounts recorded by `FX_EXCHANGE` transactions. An instrument with
prices cannot be deleted (`409 instrument_in_use`).

### Valuation

- **On request, for a date:** `as_of` is required. Only transactions,
  prices and rates dated on or before it are used, and nothing is stored.
- **Prices and rates:** each asset uses the latest price on or before
  `as_of`. To convert, valuation uses the latest of the direct rate
  (USD/THB) and the inverse one (THB/USD); the direct rate wins a tie.
  There is no triangulation through a third currency.
- **Staleness:** a price older than 7 days, or a rate older than 3, is
  flagged `stale` but still used.
- **Missing data:** an asset without a price or rate is never valued at
  zero. It is listed under `unvalued` with a reason, left out of the
  totals, and the result is marked `partial`.
- **Rounding:** values are exact until each base-currency value, which is
  rounded half-to-even to 8 places. Totals add those values exactly.
  Allocation percentages have 4 places and may not add up to exactly 100.
- **Allocation:** signed, on Net Portfolio Value. Stocks of 10,000 with
  cash of −2,000 give 125% and −25%; percentages are `null` when the Net
  Portfolio Value is zero or negative. Overdrawn cash is flagged, not
  treated as a margin loan.

### Ledger

- **Ordering:** transactions are ordered by `trade_date`, then
  `sort_order` (default 0), then entry order.
- **Kinds and required fields:**
  - `OPENING_POSITION`: `instrument_id`, `quantity` and `amount` (total cost
    basis of one lot).
  - `OPENING_CASH`: `cash_account_id` and a signed `amount`.
  - `BUY` and `SELL`: `instrument_id`, `quantity` and `price`, with optional
    `fee`, `tax` and `cash_account_id`. The cash account must use the
    instrument's quote currency.
  - `DEPOSIT`, `WITHDRAWAL`, `FEE`, `TAX` and `TAX_RETURN`:
    `cash_account_id` and a positive `amount`. A deposit or tax return adds
    to the account; the others take from it.
  - `DIVIDEND`: `instrument_id` and the gross `amount`, with optional `fee`
    and `tax` withheld and an optional `cash_account_id` (in the
    instrument's quote currency), which is credited with the net amount.
  - `INTEREST`: `cash_account_id` and the gross `amount`, with optional
    `fee` and `tax`; the account is credited with the net amount.
  - For `DIVIDEND` and `INTEREST`, `fee` + `tax` may not exceed `amount`.
  - `CASH_TRANSFER`: `cash_account_id` (source),
    `destination_cash_account_id` (another account of the same portfolio, in
    the same currency) and a positive `amount`, with an optional `fee`
    paid by the source on top of the amount.
  - `FX_EXCHANGE`: `cash_account_id` and `amount` (spent),
    `destination_cash_account_id` (an account in another currency) and
    `destination_amount` (received), with an optional `fee` in the source
    currency, paid on top of the amount spent. The executed rate is the
    ratio of the two amounts.
  - `STOCK_SPLIT`: `instrument_id` and `ratio` as `new:old`, for example
    `4:1`, or `1:3` for a reverse split. Every lot's quantity changes in
    proportion and total cost is unchanged.
  - `STOCK_DIVIDEND`: `instrument_id` and `quantity` (shares received),
    spread over the open lots at no cost.
  - Cash in lieu, on `STOCK_SPLIT` and `STOCK_DIVIDEND`:
    `cash_in_lieu_quantity` (the fraction, in post-action shares) and
    `amount` (cash received), with an optional `cash_account_id`. The
    fraction is disposed of through FIFO, realizing profit or loss.
  - Holdings after a split are rounded half-to-even to 8 decimal places,
    with the residual in the last lot. A split or stock dividend with no
    holding on its date is rejected with `409 no_holdings`.
- **Decimals:** sent as JSON **strings** (at most 8 decimals and 15
  integer digits) and stored exactly as entered. Binary floating point is
  never used. Amounts and FIFO totals are computed exactly; a result too
  large or too precise to represent exactly is rejected with `422`, never
  rounded. The only rounding is the cost taken from a partly sold lot
  (half-to-even, 10 places), and the lot keeps the exact remainder.
- **Validation:** every change replays FIFO from full history. A sale that
  exceeds holdings at its date, or an edit or delete that would leave a
  later sale uncovered, is rejected with `409 insufficient_holdings` and
  nothing is written.
- **Derived values:** holdings and cash balances come from the movements;
  they are not stored.

Portfolios belong to exactly one user. Another user's portfolio is reported
as `404`, exactly like one that does not exist.

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
| `AUTH_MAX_CONCURRENT_HASHES` | `4`                                         | Concurrent Argon2 operations (1–64, ~19 MiB each) |
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
  logging in revokes any session the browser already presented. Sessions
  that expired or were revoked more than a day ago are deleted at startup and
  then every hour.
- **CSRF**: register, login and logout require a trusted `Origin` (or a
  same-origin `Sec-Fetch-Site`; clients that send neither are not browsers).
  Logout additionally requires `X-CSRF-Token`: an HMAC-SHA256 of the session id
  under `APP_SECRET_KEY`, verified in constant time. No CORS headers are sent.
- **Login rate limiting**: 20 attempts (login and registration) per client IP
  (IPv6 grouped by /64) and 5 unsuccessful logins per account, per 15-minute
  fixed window. Exceeding either returns `429` with `Retry-After` (rounded up).
  Quota is checked and consumed in one locked step. An account slot is
  reserved before the password is checked and returned only when a login
  succeeds, so concurrent guesses cannot exceed the limit. Attempts against a
  locked account still use up the client's budget. When the key table is full,
  the entries with the lowest counts are evicted first, so flooding it with
  fresh keys cannot lift an active lockout. Unknown accounts are checked
  against a dummy Argon2 hash so response times do not reveal whether an email
  is registered.
- **Hashing capacity**: at most `AUTH_MAX_CONCURRENT_HASHES` Argon2 operations
  run at once, covering registration, login and dummy-hash checks. When all
  are busy, requests are rejected at once with `503 service_overloaded` and
  `Retry-After: 1`; nothing queues. A permit is held by the blocking
  computation itself, so a disconnected client cannot free capacity while its
  hash is still running. Overload is detected before the account's failure
  budget is touched and does not depend on whether the account exists.

### Known limitations

- Rate limits are kept in memory per process: they reset on restart and are
  not shared between instances. Running more than one instance needs a shared
  store.
- Fixed windows allow up to twice the limit in a burst straddling a window
  boundary.
- A successful login clears the account's failure count, including slots held
  by guesses that are still in flight at that moment. An attacker who has
  recorded some failures can therefore notice that the owner signed in during
  the window.
- If every tracked key is exhausted at once (10,000 keys), eviction must drop
  an exhausted entry.
- Overload rejects legitimate sign-ins too; the limiter protects memory and
  CPU, not availability against a large distributed attacker.
- `X-Forwarded-For` is honoured only from exact `TRUSTED_PROXIES` addresses
  (no CIDR ranges).
- There is no account recovery or password reset in the MVP (owner
  decision, ADR-AUTH-10). An operator must manage accounts directly in the
  database.
- Registration reveals whether an email is already registered (`409`), as
  required by the API contract.

## Docker

### Standalone API

```bash
export APP_SECRET_KEY="$(openssl rand -base64 48)"   # or put it in .env
docker compose up --build     # build and start
docker compose down           # stop (data is kept)
docker compose down -v        # stop and delete the data volume
```

The API is published on `http://127.0.0.1:8080`. SQLite data is stored in the
`merton-data` named volume, mounted at `/app/data`. The container runs as a
non-root user. Compose refuses to start without `APP_SECRET_KEY`.

### Full stack (with `merton-web`)

`merton-web/compose.yaml` runs `merton-api`, `merton-web`, and a Caddy reverse
proxy on the same origin (`http://localhost:8080`), routing `/api/*` to
`api:8080` and everything else to `web:3000`:

```bash
cd ../merton-web
cp .env.example .env          # set APP_SECRET_KEY (e.g. openssl rand -base64 48)
docker compose up --build -d
./scripts/smoke.sh            # health, OpenAPI, HTML root, and auth + CSRF round-trip
```

In `merton-web/compose.yaml`, the proxy has a static IP (`172.28.0.10`) on the
compose network matching `TRUSTED_PROXIES`, `APP_TRUSTED_ORIGINS` is set to
`http://localhost:8080`, and `SESSION_COOKIE_SECURE` remains at its default
(`true`). Production deployments behind HTTPS must set `APP_TRUSTED_ORIGINS` to
the public `https://` origin and keep `SESSION_COOKIE_SECURE=true`.

## Tests and checks

```bash
cargo fmt --all -- --check
cargo check --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Tests use an isolated in-memory SQLite database per test and need no external
services. CI runs the same checks on every push to `main` and every pull request,
and also builds the Docker image and smoke-tests `GET /api/v1/health`.

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
├── portfolios/          # Portfolio rules, owner-scoped queries, use cases
├── instruments/         # Instruments (asset class, identifiers, quote currency)
├── cash_accounts/       # Cash accounts, owned through their portfolio
├── ledger/              # Decimals, FIFO, transactions and their movements
├── market_data/         # Historical instrument prices and FX rates
├── valuation/           # Holdings, valuation and allocation at a date
├── validation.rs        # Shared value types (names, currency codes)
├── pagination.rs        # Shared limit/offset paging
├── http/                # Router, handlers, extractors, errors, OpenAPI
└── infrastructure/
    ├── database.rs      # Connection pool setup
    ├── entities/        # SeaORM entities
    └── migrations/      # SeaORM migrations
tests/                   # Integration tests
```

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
