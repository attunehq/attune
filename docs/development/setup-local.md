# Attune local dev setup

This document walks though setting up Attune to run locally, and then testing that it is working end-to-end using a Docker container.

Each sub-heading is meant to be read as an ordered step.

## Clone Attune

Clone the main repository, which you've probably already done since you're reading this:
```sh
git clone git@github.com:attunehq/attune.git
```

If you're not an employee, you'll need to delete the `packages/attune-ee` folder if it exists so that Cargo can properly build:
```sh
rm -rf packages/attune-ee
```

If you're an employee, you'll instead need to clone the git submodules:
```sh
git submodule init
git submodule update
```

## Install prerequisites

You'll need the following tools installed and configured:
- Docker
- Rust
- Go
- GPG

These aren't required, but are recommended:
- `direnv`
- `minio-mc`

> [!NOTE]
> While you can set up service dependencies like Postgres and MinIO locally, Attune optimizes developer workflow and documentation around using `docker compose` to manage these services. If you _really must_ use them locally, use the `docker-compose.yml` file to reconstruct how they should be configured.

## Build the Attune CLI

We'll need to use the Attune CLI as part of these steps.

Build and install it:
```sh
cd cli
go install ./...
```

## Run local services

In the root of the workspace, run the local services using Docker:
```sh
docker compose up minio postgres
```

In another terminal, migrate the database:
```sh
docker compose run migrate
```

## Configure local object storage

You can validate that minio is running with the `mc` command:
```sh
mc alias set 'attune' 'http://127.0.0.1:9000' 'attuneminio' 'attuneminio'
mc ready attune
```

> [!WARNING]
> Attune uses S3 "virtual-hosted-style" requests, which means it uses subdomains to access the bucket. On macOS you'll need to manually add the bucket name you configured as the subdomain:
> ```text
> 127.0.0.1	      localhost      attune-dev-0.localhost
> 255.255.255.255	broadcasthost
> ::1             localhost
> ```
>
> You can validate that it is working properly by hitting its health endpoint:
> ```sh
> curl attune-dev-0.localhost:9000/minio/health/live -v
> ```

Finally, you _may_ need to set the bucket as "public read only" for it to be read by `apt`:
```sh
mc anonymous set download attune/attune-dev-0 -r
```

You can validate that this is configured with this:
```sh
mc anonymous list attune/attune-dev-0 -r
```

## Set up `.env` file

Set up your `.env` file using the `.env.example` file as a base.

Make sure to follow any instructions in the comments, and make sure that the values match the values you used when configuring services earlier.

> [!TIP]
> We recommend you use `direnv` to automatically load this file.

## Run Attune

Finally, you can now run the Attune server!

Make sure that your `.env` file is loaded or that you specify the arguments manually, then run Attune with the following:
```sh
cargo run --bin attune-server
```

You can validate that the server is running by using the installed `attune` CLI:
```sh
attune repo list
```

Keep this server running; future steps will be in a new terminal window.

## Publish a package

Here, we'll publish a package to the locally running Attune control plane.

We have prebuilt packages for this use case, just grab one off the
[`attune-test-package`](https://github.com/attunehq/attune-test-package/releases)
release page.

> [!NOTE]
> The following steps assume that the path you've downloaded the package to test is at `$ATTUNE_TEST_PACKAGE_PATH`; either set that variable or replace it as desired.

Generate a GPG key and copy the 40-character hex encoded ID of the one you want to use:
```sh
# Fill out as desired; for development and local testing you can probably leave
# all the settings as their defaults.
gpg --generate-key

# Read the ID from the output of this command.
# For example, the ID here is 571B4D41C200EB7254ED932103A515124FB4EF14.
#
# [keyboxd]
# ---------
# sec   ed25519 2025-07-29 [SC] [expires: 2028-07-28]
#       571B4D41C200EB7254ED932103A515124FB4EF14
# uid           [ultimate] Jessica Black <me@jessica.black>
# ssb   cv25519 2025-07-29 [E] [expires: 2028-07-28]
gpg --list-secret-keys
```

> [!NOTE]
> The following steps assume that the ID of the GPG key you want to use is set to `$ATTUNE_GPG_KEY_ID`; either set that as a variable or replace it as desired.

Create a "repository" in Attune and record the ID:
```sh
# Read the ID from the output of this command.
# For example, the ID here is 1.
#
# Created new repository:
# ID  URI                           Distribution
# 1   http://localhost:9000/debian  stable
attune repo create -u 'http://localhost:9000/debian'
```

> [!NOTE]
> The following steps assume that the ID of the Attune repository you want to use is set to `$ATTUNE_REPOSITORY_ID`; either set that as a variable or replace it as desired.

Add the package to the repository and sync the repository:
```sh
attune repo -r $ATTUNE_REPOSITORY_ID pkg add $ATTUNE_TEST_PACKAGE_PATH
attune repo -r $ATTUNE_REPOSITORY_ID sync -i $ATTUNE_GPG_KEY_ID
```

## Set up a testing Docker container

We'll set up a Docker container that pulls from the control plane that is running locally.

> [!WARNING]
> macOS users need to enable "host networking" in Docker Desktop to follow these steps.

You'll need to find the S3 prefix for the repository. This is computed using the ID of the tenant performing the release in the database, followed by a slash, followed by the hex-encoded sha256 hash of the repository URI provided when the repository was created.

The simplest way to find this is to look at the `debian_repository` database table, or just look in the MinIO interface if you have a small number of repositories.

> [!NOTE]
> The following steps assume that the S3 prefix you want to use is set to `$ATTUNE_REPOSITORY_S3_PREFIX`; either set that as a variable or replace it as desired.

Copy the public key of the GPG key that you used to sign the release:
```sh
gpg --armor --export $ATTUNE_GPG_KEY_ID
```

> [!NOTE]
> The following steps assume that the public key you want to use is set to `$ATTUNE_REPOSITORY_PUBLIC_KEY`; either set that as a variable or replace it as desired.

Start a Docker debian container and configure it to connect to the Attune repository:
```sh
# Start the docker container; commands after this are inside this context.
docker run -it --rm --network=host debian /bin/bash

# Configure the system to use Attune as a repository.
echo 'deb http://localhost:9000/attune-dev-0/$ATTUNE_REPOSITORY_S3_PREFIX stable main' > /etc/apt/sources.list
cat >> /etc/apt/trusted.gpg.d/attune.asc << EOF
$ATTUNE_REPOSITORY_PUBLIC_KEY
EOF

# Install the test package
apt update
apt install attune-test-package

# Validate the package was installed successfully
attune-test-package --version
```

You can also specify `platform` to test other platforms; for example if you're on an M-series MacBook you can run something like the below to test `amd64` packages:
```sh
docker run -it --rm --network=host --platform=linux/amd64 debian /bin/bash
```
