# Attune Self-Hosting Guide

Self-hosting Attune is more involved than using Attune Cloud.

This guide will assume you understand the basics of Debian package repositories and hosting. If you need to brush up, check out the sections in the [user guide](./README.md) about publishing packages.

## Prerequisites

Before you begin, ensure you have the following installed:

- **Docker**: Required for running the Attune control plane and required services (PostgreSQL and MinIO).
- **Rust**: Required for building the Attune CLI.

## 1. Clone the repository

```bash
git clone https://github.com/attunehq/attune.git
cd attune
```

## 2. Set up environment variables

Copy the example environment file and modify it as needed:

```bash
cp .env.example .env
```

Make sure the values in the `.env` file match your local setup. The defaults should work with the Docker Compose configuration.

We use [`direnv`](https://direnv.net/) to manage environment variables, but you can use whatever solution works for you.

## 3. Spin up Docker containers

Start the Attune control plane, PostgreSQL and MinIO in the background:

```bash
docker compose up --detach
```

This will start:
- Attune control plane on port `3000`
- PostgreSQL on port `5432` (default credentials: `attune`/`attune`)
- MinIO on ports `9000`/`9001` (default credentials: `attuneminio`/`attuneminio`)

You can check if the containers are running with:

```bash
docker compose ps
```

You can also check the logs with:

```bash
docker compose logs --follow
```

## 4. Build and install the CLI

You can build and install the CLI with:

```bash
cargo install --path ./packages/attune
```

By default, this installs the binary into `$HOME/.cargo/bin`, so make sure that directory is in your `$PATH`.

## 5. Publish packages

Now you can follow the [publishing packages](./README.md#publishing-packages) section in the user guide!

There are some small key differences to be aware of:

1. When running the CLI, your `$ATTUNE_API_TOKEN` should be set to the same value that the control plane has (i.e. the same value as the one in `.env`).
2. When running the CLI, you will additionally need to set `$ATTUNE_API_ENDPOINT` to `http://localhost:3000`.
3. You will _not_ have any repositories pre-provisioned for you. To start, you'll need to create one using:

   ```bash
   attune apt repo create $YOUR_REPO_NAME
   ```

   The repository's name can be anything, but every repository must have a unique name.

By default, Attune will publish packages to S3-compatible object storage, as configured via the `.env` file in the `AWS_*` environment variables and the `ATTUNE_S3_BUCKET_NAME` environment variable.

Each repository has its own "S3 prefix" where its published repository files are stored. If you want to serve your repository on the internet, you can serve objects at this prefix (e.g. by using Amazon CloudFront with Amazon S3).
