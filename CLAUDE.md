# CLAUDE.md — Merton API

Entry point for AI coding agents (Claude Code, Gemini/AGY, others) working in
this repository. Keep this file short; details live in the linked documents.

## Project

**Merton Labs** is an Apache-2.0, self-hosted, multi-user investment
portfolio tracker. This repo, `merton-labs/merton-api`, is the Rust REST
backend. The SvelteKit frontend is a separate repository.

## Sources of truth: read before implementing a phase

1. [`docs/PROJECT_PLAN.md`](docs/PROJECT_PLAN.md): scope, non-goals,
   roadmap (P0–P6), phase deliverables, acceptance criteria, **current
   status (§23)**, **next task (§24)**, and open questions (§22).
2. [`docs/ARCHITECTURE_DECISIONS.md`](docs/ARCHITECTURE_DECISIONS.md):
   every decision, marked Accepted / Superseded / Proposed / Open.
3. [`README.md`](README.md): how to run, configure and test the service.

If the docs and the code disagree, the **code and CI are the evidence**.
Report the mismatch and fix the docs; do not "fix" the code to match a plan
that was never approved.

## Fixed stack (do not change without owner approval)

- **Language:** Rust stable, edition 2024 (MSRV 1.94).
- **Single Cargo crate:** no workspace, no extra crates.
- **HTTP:** Axum, Tokio and Tower/tower-http; REST under `/api/v1`, with
  OpenAPI via utoipa.
- **Persistence:** SeaORM, plus SeaORM Migration (in-crate, applied at
  startup). SQLite now; schemas and queries stay PostgreSQL-portable.
- **Other crates:** Serde; tracing.
- **Deployment:** Docker and Docker Compose. No Redis, Kubernetes, message
  brokers or mandatory cloud services.
- **Frontend:** SvelteKit with Svelte 5 runes and TypeScript, in a separate
  repo. It calls this API directly, deployed same-origin behind a reverse
  proxy. No BFF, no CORS.

## Non-negotiable rules

**Scope**
- Excluded from the MVP: bonds, crypto, derivatives, margin, brokerage
  integration, automated market data, sharing, and teams. Never add them
  back.
- Do not implement later phases early. Do not add speculative abstractions,
  such as generic repository traits, RBAC or plugin systems.

**Security**
- Identity comes **only** from the verified session, through the
  `CurrentSession` or `CsrfProtectedSession` extractors in
  `src/http/extract.rs`.
  - Never trust a `user_id` or `owner_user_id` sent in a request.
  - Owner-scoped code takes `auth::UserId`.
- **Every authenticated mutation uses `CsrfProtectedSession`.**
- Every query on user-owned data is **owner-scoped** in the query itself.
  Related IDs must belong to the same owner. A resource that belongs to
  another user returns `404 not_found`.
- Never log, or return, passwords, password hashes, session tokens, CSRF
  tokens, `APP_SECRET_KEY`, SQL errors or other users' data.
- Argon2 runs only through `HashPermit`. Keep `password::hash` and
  `password::verify` private.

**Finances**
- Never use `f32`/`f64` or SQLite `REAL` for financial values. Use
  `rust_decimal`, serialize it as JSON strings, and store it as validated
  decimal TEXT.
- Holdings and cash are **derived** from ledger movements and never stored
  authoritatively.
- Related ledger writes are atomic. Never cascade-delete financial
  records.
- A SQLite transaction that reads and then writes must start with a write
  (ADR-DB-09); otherwise it fails with "database is locked" under
  concurrency.
- Do not invent financial rules (rounding, allocation, fee placement).
  If a rule is Open in the decisions doc, stop and ask.

**Code**
- Keep handlers thin. Financial logic must not depend on Axum or SeaORM.
- No `unsafe` (it is forbidden by lint). No `unwrap`/`expect` in runtime
  code (clippy denies them outside tests).
- Errors are RFC 9457 Problem Details with a stable `code`; never change
  an existing code's meaning.
- Never edit a committed migration; add a new one.

## Before editing

1. Run `git status`, `git log -5 --oneline` and `git branch --show-current`.
   Preserve any uncommitted work.
2. Read the code you will touch, plus its tests. Verify crate APIs and
   versions from `Cargo.lock` and the registry sources; do not guess.
3. Check the decisions doc for anything Open that affects the task. Ask
   the owner rather than choosing yourself.
4. Follow the existing hybrid module layout (ADR-ARCH-03, accepted by
   the owner): do not reorganize without approval.

## Verify (all must pass; never claim a pass you did not run)

```bash
cargo fmt --all -- --check
cargo check --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Tests use `tests/common::TestApp`, which gives each test an isolated
in-memory SQLite database. Concurrency tests use barriers or channels,
never sleeps. When a change touches runtime or deployment, also smoke-test
it with `cargo run` plus curl, and with `docker compose up --build`.

## Definition of Done

- [ ] The four commands above pass.
- [ ] Tests are added in the same phase. Owned resources have cross-user
      tests. Financial changes have invariant tests.
- [ ] OpenAPI is updated for every new or changed endpoint, response and
      error.
- [ ] Migrations apply cleanly on a fresh database.
- [ ] No secrets or other users' data in logs or responses.
- [ ] Docs are updated (see below).
- [ ] The final `git diff` has been reviewed for unintended changes.

**Security-sensitive or financial changes** (auth, sessions, CSRF,
ownership, ledger, FIFO, decimals, valuation, import) need deeper review:
- re-read the relevant ADRs
- add negative and adversarial tests
- check concurrency and atomicity
- prefer an independent reviewer pass, from a read-only worker or a
  second agent

## Documentation discipline

- When a phase completes, update `docs/PROJECT_PLAN.md`: §23 status, the
  phase **Status** and its checklist in §17/§18, and §24 (next task).
- When a decision changes, add or supersede an entry in
  `docs/ARCHITECTURE_DECISIONS.md` and check whether the plan must change
  too.
- Never mark a Proposed or Open decision as Accepted. Only the project
  owner accepts product and architecture decisions.
  - Implementation details that shipped in reviewed code may be recorded
    as `Accepted · Source: Implementation (Pn)`.
  - Anything that answers an Open question stays `Proposed` until the
    owner confirms it.
- Keep README's configuration table in sync with `src/config.rs`.
- Do not tie these docs to a specific AI model version.

## Git safety

- **Do not** commit, push, open or merge pull requests, or change GitHub
  settings unless the owner explicitly asks.
- No destructive commands (`reset --hard`, `clean -fd`, force push,
  branch deletion) on the owner's work.
- Do not create or switch branches unless the owner asks. The owner
  reviews and commits changes.

## Multi-agent coordination

- The parent agent is the team leader. It owns architecture decisions,
  integration and the final verification.
- Give workers scoped tasks. Prefer **read-only** analysis or review
  first: for AGY, `agy -p "<task>" --mode plan`, run from the repo root.
  Parallel runs are fine when they are read-only.
- Never let two agents edit the same working tree at once. Parallel edits
  go in isolated `git worktree`s, then the leader reviews and integrates
  them.
- Review worker output critically, and accept or reject each point with a
  reason. Verify everything with real tests.
- Never report a worker's result unless that worker actually ran. Keep
  the evidence (commands, output files).
