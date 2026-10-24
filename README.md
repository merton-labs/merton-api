# Merton API

Backend for **Merton**, an open-source, self-hosted, multi-user investment portfolio tracker.

> **Status: project bootstrap.** Only the server foundation exists so far:
> configuration, logging, a SQLite connection pool, migration support (with no
> migrations yet), OpenAPI generation, and one health endpoint. No portfolio,
> user or financial features have been built yet.

## Technology stack

| Concern       | Choice                                         |
| ------------- | ---------------------------------------------- |
| Language      | Rust (stable, edition 2024, MSRV 1.94)         |
| HTTP          | Axum 0.8, Tower / tower-http                   |
| Runtime       | Tokio                                          |
| Database      | SQLite via SeaORM 2 (PostgreSQL planned)       |
| Migrations    | SeaORM Migration (inside this crate)           |
| OpenAPI       | utoipa                                         |
| Logging       | tracing + tracing-subscriber                   |
| Deployment    | Docker + Docker Compose                        |

## Prerequisites

- Rust stable 1.94 or newer (`rustup`, with `rustfmt` and `clippy`)
- Docker with Compose v2, only for running in containers

## Local development

```bash
cp .env.example .env    # optional; the defaults work without it
cargo run
```

Then open the API:

```bash
curl http://127.0.0.1:8080/api/v1/health
# {"status":"ok"}
```

The SQLite database is created at `data/merton.db`. Pending migrations run
automatically at startup.

### Endpoints

| Method | Path                   | Description                                                                |
| ------ | ---------------------- | -------------------------------------------------------------------------- |
| GET    | `/api/v1/health`       | `200 {"status":"ok"}`, or `503 {"status":"unavailable"}` if the database is unreachable |
| GET    | `/api/v1/openapi.json` | OpenAPI document                                                           |

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
| `RUST_LOG`                 | `merton_api=info,tower_http=info,sqlx=warn`   | Log filter (`tracing` EnvFilter syntax)  |

SQLite connections always enable foreign keys and WAL journaling, and the
database file is created if it does not exist.

## Docker

```bash
docker compose up --build     # build and start
docker compose down           # stop (data is kept)
docker compose down -v        # stop and delete the data volume
```

The API is published on `http://127.0.0.1:8080`. SQLite data is stored in the
`merton-data` named volume, mounted at `/app/data`. The container runs as a
non-root user.

## Tests and checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Tests use in-memory SQLite and need no external services. CI runs the same
checks on every push to `main` and every pull request.

## Project layout

```text
src/
├── main.rs              # Process entry: .env, logging, runtime
├── lib.rs
├── app.rs               # Wiring: database, migrations, server, shutdown
├── config.rs            # Environment configuration
├── http/                # Router, handlers, OpenAPI document
└── infrastructure/
    ├── database.rs      # Connection pool setup
    └── migrations/      # SeaORM migrator
tests/                   # Integration tests
```

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
