# Attune User Guide

Attune is a tool for securely publishing and hosting Linux packages.

For a quick start guide to get up and running in 5 minutes, see the [Quick Start section in the main README](../../README.md#quick-start).

## Setting up Attune

To run Attune, you'll need to configure three pieces:

1. The **CLI** ([`attune`](../../cli)), which you'll use in your development or CI environment to publish packages.
2. The **control plane** ([`attune-controlplane`](../../controlplane)), which runs a service that manages repository operations.
3. The **data plane**, which actually serves the repository packages.

### Setting up the Attune service

#### Quick start

1. Create a `.env` file containing the environment variables you've configured. Use [`.env.example`](../../.env.example) as a template and reference for the available variables. **You should set `ATTUNE_SECRET` to your own secret API token.**
2. Run `docker compose up` in the root directory of the Attune repository. This will start local instances of Postgres, Minio, and Attune as Docker containers.

#### Using your own database and data plane

If you'd like to use your own providers for Postgres and Minio, you can configure the control plane to use them by setting the `ATTUNE_DATABASE_URL` and `AWS_ENDPOINT_URL_S3` environment variables. For details, see [`.env.example`](../../.env.example).

### Setting up the CLI

You can download the CLI from [GitHub Releases](https://github.com/attunehq/attune/releases).

To use the CLI, you need to configure two environment variables:

- `ATTUNE_API_TOKEN`: This should be set to the same value as the `ATTUNE_SECRET` of the control plane.
- `ATTUNE_API_ENDPOINT`: This should be set to the base URL of the control plane (e.g. `http://localhost:3000`).

Like for the control plane, you can see details about the available environment variables in [`.env.example`](../../.env.example).

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
3. Copy the public key (from `gpg --armor --export $KEYID`) of the key that you signed your repository with into `/etc/apt/trusted.gpg.d/attune.asc`. This will tell APT to trust the key you signed the repository with.
4. Now you can run `apt update` and then `apt install` one of the packages you published.
