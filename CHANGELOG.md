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

### Changed

- Everything outside the MVP, including CSV import and export (P6), is now
  a backlog item the owner can schedule, instead of an exclusion
  (ADR-SCOPE-10, `docs/PROJECT_PLAN.md` §4). Docs only.

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
