# Attune User Guide

Attune is a tool for securely publishing and hosting Linux packages.

> :construction: This project is under early construction. There are a bunch of rough edges right now which we plan to implement soon (e.g. Docker-based deployment).

## Setting up Attune

To run Attune, you'll need to configure three pieces:

1. The **CLI** ([`attune`](../../cli)), which you'll use in your development or CI environment to publish packages.
2. The **control plane** ([`attune-controlplane`](../../controlplane)), which runs a service that manages repository operations.
3. The **data plane**, which actually serves the repository packages.

### Setting up the control plane

To start, we'll set up the control plane.

1. Set up a Postgres instance. If you're self-hosting, you can use our provided [`docker-compose.yml`](../../docker-compose.yml) file, which will start a local Postgres instance inside of a Docker container. Once the Postgres database is up, you'll need to run initial migrations using [`sqlx migrate run`](https://github.com/launchbadge/sqlx/blob/main/sqlx-cli/README.md).
2. Set up an S3-compatible object storage bucket. If you're self-hosting, you can use our provided [`docker-compose.yml`](../../docker-compose.yml) file, which will start a local Minio instance inside of a Docker container.
3. Once these are available, you'll want to set up environment variables for the control plane server. For a list of variables, see [`.env.example`](../../.env.example).
4. Once the environment variables are configured, you can build Attune using `cargo build --release` in the `controlplane` folder. If you don't have Rust installed, you can download it using [`rustup`](https://rustup.rs).
5. You can now deploy the built binary with the environment variables in any environment. The server will listen on `0.0.0.0:3000`.

### Setting up the CLI

You can build the Attune CLI by using `go install ./...` in the `cli` folder. If you don't have Go installed, see [the installation docs](https://go.dev/doc/install).

The Attune CLI needs to run in an environment where `ATTUNE_API_TOKEN` is set. Make sure to set this to the same value as the `ATTUNE_SECRET` of the control plane.

If your control plane is running on a different host than your CLI, you can set that host using `ATTUNE_API_ENDPOINT`.

### Setting up the data plane

Attune publishes a Linux repository to any S3-compatible object storage bucket. To serve this repository, you'll need to serve contents out of that bucket. How to do this will depend on exactly which object storage provider you use.

A non-exhaustive list of providers includes:

- AWS S3
- Cloudflare R2
- Backblaze B2
- Self-hosted Minio

## Publishing packages

Once everything is set up, here's how you can publish a package:

1. Create a repository using `attune repo create`. You'll need to pass some repository fields as flags. These are defined by [the Debian repository format for Release files](https://wiki.debian.org/DebianRepository/Format#A.22Release.22_files). <!-- TODO: We should really make these optional, and set default values. --> If you're self-hosting, you can set `uri` to anything.
2. Once created, you should be able to see your repository with `attune repo list`.
3. Now you can add packages with `attune repo pkg add`. This uploads the package to the backend, which stores it in a _staging area_ in your object storage.
4. Once you're done adding packages, you can use `attune repo sync` to publish the repository. This will require you to provide an ASCII-armored GPG private key file for signing the indexes. You can generate this file using `gpg --armor --export-secret-keys $KEYID`, where `$KEYID` is from `gpg --list-secret-keys`.

   During this step, Attune will generate your repository indexes, sign them locally on the CLI, and then upload the new indexes and staged packages into the active release area of your object storage bucket.

That's it! Your packages have now been published.

## Testing your Linux repository

If you want to test your new repository:

1. Start a new Debian container using `docker run -it --rm --network=host debian /bin/bash`.
2. Run `echo 'deb YOUR_REPOSITORY_URI YOUR_DISTRIBUTION_NAME YOUR_COMPONENT_NAME' > /etc/apt/sources.list` to add your new repository to the list of sources in the Debian container. For example, if you're using Minio locally using our Docker Compose file, you might run `echo 'deb http://localhost:9000/your-bucket-name bookworm main' > /etc/apt/sources.list`.
3. Copy the public key (from `gpg --armor --export $KEYID`) of the key that you signed your repository with into `/etc/apt.trusted.gpg.d/attune.asc`. This will tell APT to trust the key you signed the repository with.
4. Now you can run `apt update` and then `apt install` one of the packages you published.
