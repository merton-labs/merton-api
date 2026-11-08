# Changelog

All notable changes to merton-api are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Before 1.0.0 the API may still
change between minor versions.

## [Unreleased]

### Added

- `GET /api/v1/portfolios/{id}/dividends?from=&to=`: dividends and interest
  received after tax and fees, in the base currency, with totals by month,
  holding or cash account, asset class and currency. Each payment is
  converted at the FX rate on its own date; payments without a rate are
  listed and the result is partial (ADR-DIV-01).
- `GET /api/v1/portfolios/{id}/returns?as_of=`: per holding, open or sold,
  the capital gain, realized P&L, dividends, fees, taxes, total profit and
  their percentages in the quote currency, as in Snowball's Holdings table
  (ADR-RET-01).

- PostgreSQL support: set `DATABASE_URL` to a `postgres://` URL (TLS
  included, for hosted services such as Neon). Ledger writes lock the
  portfolio row and reads use one snapshot, so behaviour matches SQLite.
  Existing SQLite data is not migrated (ADR-DB-10). Tests run on
  PostgreSQL with `TEST_DATABASE_URL`.

### Fixed

- PostgreSQL: timestamps in create and update responses (`created_at`,
  `updated_at`) now equal the stored values. They were returned with
  sub-microsecond digits that PostgreSQL does not keep, so reading the same
  row back gave a different value. All stored timestamps are now whole
  microseconds on both backends (ADR-DB-10).

### Changed

- The default port is now 18080 instead of 8080: `APP_PORT`, the default
  `APP_TRUSTED_ORIGINS`, the Docker image and Compose. On Windows, Hyper-V
  and WSL often reserve 8080. Set `APP_PORT` and `APP_TRUSTED_ORIGINS` to
  keep the old port.
- Docker Compose: `DATABASE_URL` can be set in `.env` or the shell, for
  example to a PostgreSQL (Neon) URL; it still defaults to SQLite on the
  `merton-data` volume. It was fixed to SQLite before.
- The Docker image no longer declares `VOLUME ["/app/data"]`. Compose still
  mounts the `merton-data` volume there; with plain `docker run` and SQLite,
  mount a volume at `/app/data` (`-v`) or the data goes with the container.
- The Dockerfile no longer uses BuildKit cache mounts, which Railway rejects
  without a Railway-specific `id`. Local image rebuilds recompile the
  dependencies.
- Everything outside the MVP, including CSV import and export (P6), is now
  a backlog item the owner can schedule, instead of an exclusion
  (ADR-SCOPE-10, `docs/PROJECT_PLAN.md` §4). Docs only.

### Removed

- `package.json`, `package-lock.json` and `neon.ts`, left over from the Neon
  CLI setup. Nothing used them; the API reaches Neon through `DATABASE_URL`.

## [0.1.0] - 2026-11-01

The first release: the MVP (phases P0–P5 in
[docs/PROJECT_PLAN.md](docs/PROJECT_PLAN.md)). Use it with
[merton-web 0.1.0](https://github.com/merton-labs/merton-web/releases/tag/v0.1.0).

### Added

- Axum server with SQLite through SeaORM, health check, OpenAPI document at
  `GET /api/v1/openapi.json`, and RFC 9457 problem details for errors.
- Local email and password accounts: Argon2 hashing with bounded
  concurrency, server-side cookie sessions with expiry, revocation and an
  hourly purge, CSRF and `Origin` checks, and atomic login and register
  rate limits.
- Private portfolios with a base currency, archive and restore, and a
  delete that refuses portfolios with financial history.
- Instruments with a fixed quote currency, and cash accounts with one
  currency each; balances may go negative.
- Transaction ledger with exact decimals (at most 8 places) and FIFO lots:
  opening positions and balances, buys and sells, deposits and withdrawals,
  dividends (with an optional payout per share), interest, fees, taxes, tax
  returns and other expenses, cash transfers, FX exchanges, stock dividends,
  stock splits and reverse splits. Edits and deletes replay the history and
  never allow a negative holding.
- Expenses follow Snowball Analytics: they may name an instrument, and
  moving cash is optional (ADR-TXN-09).
- Fees and taxes stay out of cost basis, as in Snowball Analytics
  (ADR-LEDG-11).
- Manually entered prices and FX rates, and holdings, valuation and signed
  allocation by asset class at any `as_of` date, with partial results and
  staleness made explicit.
- Dockerfile and Compose file, and CI that runs format, lint, tests and a
  Docker build with a smoke test.

### Not included

- CSV import and export (moved after the MVP, ADR-SCOPE-09).
- Account recovery (ADR-AUTH-10).

[0.1.0]: https://github.com/merton-labs/merton-api/releases/tag/v0.1.0
