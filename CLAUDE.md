# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository Overview

Attune is a tool for securely publishing and hosting Linux packages, currently supporting APT (Debian/Ubuntu) repositories. It's implemented as a Rust monorepo with two editions:
- **Community Edition** (open-source in `packages/attune`)
- **Enterprise Edition** (in `packages/attune-ee` git submodule)

## Development Commands

### Building and Testing

```bash
# Build all packages
cargo build --workspace

# Build release version
cargo build --workspace --release

# Run tests
cargo test --workspace --all-targets --all-features

# Check code without building
cargo check --workspace --all-targets --all-features

# Run clippy linter
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Format code
cargo fmt --all

# Build and run the CLI
cargo run --bin attune -- [args]

# Build and run the server
cargo run --bin attune-server
```

### Running the Development Environment

The standard development workflow is to run the database and MinIO in Docker, while running the server locally:

```bash
# Start only the database and MinIO services
docker compose up -d minio postgres

# Wait for services to be healthy, then migrate the database
docker compose run migrate

# For schema changes without creating a migration yet
cd docker/migrate && npx prisma db push

# Run the server locally (in a separate terminal)
cargo run --bin attune-server

# View logs for Docker services
docker compose logs -f [service_name]

# Stop all services
docker compose down
```

Alternative: Run everything in Docker:
```bash
# Start all services including control plane
docker compose up -d

# Rebuild control plane after code changes
docker compose up -d --build controlplane
```

### Environment Setup

```bash
# Copy environment template
cp .env.example .env

# Edit .env file with your configuration
# Key variables:
# - ATTUNE_API_TOKEN (generate with: openssl rand -hex 32)
# - ATTUNE_DATABASE_URL (default: postgres://attune:attune@localhost:5432/attune)
# - AWS_* variables for S3/MinIO access
```

## Architecture

### Three-Tier Architecture

1. **CLI** (`packages/attune/src/bin/attune/`)
   - Command-line tool for developers
   - Handles local signing of repository indexes
   - Manages package uploads and repository operations

2. **Control Plane** (`packages/attune/src/bin/attune-server/`)
   - REST API server using Axum framework
   - Manages repository metadata in PostgreSQL
   - Coordinates package uploads to S3
   - Generates repository indexes for signing

3. **Data Plane**
   - S3-compatible object storage (MinIO in dev, any S3 provider in production)
   - Serves packages and signed indexes via CDN
   - Stores packages at `/packages/<sha256sum>` paths

### Key Design Principles

1. **Security First**: Package signing happens locally in developer environment - signing keys never leave the developer's machine
2. **Performance**: Incremental repository index rebuilds for fast package additions/removals
3. **Flexibility**: Self-hosted or managed cloud deployment options

### Database Schema

The PostgreSQL database (defined in `docker/migrate/prisma/schema.prisma`) includes:
- `AttuneTenant`: Multi-tenant support for organizations
- `DebianRepository`: Top-level repository objects
- `DebianRepositoryRelease`: Debian releases (distributions) like bookworm, jammy
- `DebianRepositoryComponent`: Components like main, contrib, non-free
- `DebianRepositoryPackage`: Individual .deb packages with metadata
- `DebianRepositoryPackagesIndex`: Pre-computed package indexes for fast serving

### Repository Workflow

Repositories follow a Git-like model:
- **Active Release**: Currently published content served to users
- **Staging Area**: Where changes are prepared before publishing
- Changes are "committed" from staging to active via the `sync` command

## Project Structure

```
attune/
├── packages/
│   ├── attune/              # Community Edition
│   │   ├── src/
│   │   │   ├── api/         # API client code
│   │   │   ├── auth.rs      # Authentication
│   │   │   ├── server/      # Server-side handlers
│   │   │   └── bin/
│   │   │       ├── attune/          # CLI binary
│   │   │       │   └── cmd/         # CLI commands
│   │   │       │       └── apt/     # APT-specific commands
│   │   │       │           ├── dist/    # Distribution management
│   │   │       │           ├── pkg/     # Package management
│   │   │       │           └── repo/    # Repository management
│   │   │       └── attune-server/   # Server binary
│   │   └── Cargo.toml
│   └── attune-ee/           # Enterprise Edition (git submodule)
├── docker/
│   ├── attune/              # Control plane Dockerfile
│   ├── migrate/             # Database migrations (Prisma)
│   └── minio/               # MinIO configuration
├── docs/
│   ├── rfd/                 # Request for Discussion documents
│   └── development/         # Development guides
└── docker-compose.yml       # Local dev environment
```

## Common Tasks

### Creating a Repository
```bash
attune apt repo create -u 'http://localhost:9000/debian'
```

### Adding Packages
```bash
attune apt repo -r <repo-id> pkg add <path-to-package.deb> -i <gpg-key-id>
```

### Publishing Changes
```bash
attune apt repo -r <repo-id> sync
```

### Listing Repositories
```bash
attune apt repo list
```

## Important Notes

- The Rust toolchain version is pinned at 1.88.0 (see `rust-toolchain.toml`)
- The project uses workspace dependencies defined in the root `Cargo.toml`
- The enterprise edition crate must be removed for open-source CI builds
- Database migrations use Prisma and run automatically in Docker Compose
- MinIO requires proper domain configuration for vhost-style S3 requests

## Code Style Guidelines

- Prefer functional style, using combinators like `ok_or_else` or `tap::Pipe` over mutation.
- Prefer to interpolate `format!()` strings inline: `format!("username: {user_name}")` instead of `format!("username: {}, user_name)`. Note that this can only be done with simple variables: `format!("username: {user.name}")`, functions, or other similar non-simple evaluations doesn't work.

## Rust Coding Tips

- `bon::Builder` doesn't need to be unwrapped; if you get it wrong it'll fail to compile. If you need to set an optional field with an option variable, use the field name with a `maybe_` prefix, e.g. `maybe_description`.
```