
# attune local dev setup

This document walks though setting up Attune to run locally outside of Docker,
and then testing that it is working end-to-end using a Docker container.

Each sub-heading is meant to be read as an ordered step.

## Clone Attune

Clone the main repository, which you've probably already done since you're reading this:
```sh
; git clone git@github.com:attunehq/attune.git
```

If you're not an employee, you probably want to delete the `packages/attune-ee` folder if it exists
so that Cargo can properly build:
```sh
; rm -rf packages/attune-ee
```

If you're an employee, you'll also want to clone the git submodules:
```sh
; git submodule init
; git submodule update
```

## Install prerequisites

You'll need the following tools installed and configured:
- Docker ([desktop](https://docs.docker.com/get-started/get-docker/), [engine](https://docs.docker.com/engine/install/))
- PostgreSQL ([Docker](https://hub.docker.com/_/postgres), [macOS](https://postgresapp.com/))
- Minio ([quickstart](https://charts.min.io/))
- Rust ([rustup](https://rustup.rs/), [mise](https://mise.jdx.dev/lang/rust.html))
- Go ([go.dev](https://go.dev/), [mise](https://mise.jdx.dev/lang/go.html))
- NPM and Node ([nvm](https://github.com/nvm-sh/nvm), [volta](https://volta.sh/), [mise](https://mise.jdx.dev/lang/node.html))
- GPG (install using your package manager)

## Build the Attune CLI

We'll need to use the Attune CLI as part of these steps.

Build and install it:
```sh
; cd cli
; go install ./...
```

## Configure local database

This service requires PostgreSQL.

If you're running in Docker, you can configure the options directly:
```sh
# These match the defaults in the `.env.example` file.
# You can use different values here, just make sure to modify your `.env` file accordingly.
; docker run --name attune-postgres \
-e POSTGRES_USER=attune \
-e POSTGRES_PASSWORD=attune \
-d postgres
```

If you're running natively, once it's running configure it with user credentials and a database:
```sh
# Log in to your postgres service.
; psql

# These match the defaults in the `.env.example` file.
# You can use different values here, just make sure to modify your `.env` file accordingly.
; create user attune with login password 'attune' superuser;
; create database attune owner attune;
```

## Configure local object storage

This services requires S3 compatible object storage,
these instructions walk through using Minio.

If running Minio natively, provide it with the same settings as you'll use in the `.env` file:
```sh
# These match the defaults in the `.env.example` file.
# You can use different values here, just make sure to modify your `.env` file accordingly.
MINIO_DOMAIN=localhost MINIO_ROOT_USER=attuneminio MINIO_ROOT_PASSWORD=attuneminio minio server ~/.minio/data
```

You can validate that minio is running with the `mc` command:
```sh
; mc alias set 'attune' 'http://127.0.0.1:9000' 'attuneminio' 'attuneminio'
; mc ready attune
```

> [!WARNING]
> Minio uses subdomains for the bucket.
> On macOS you'll need to manually add the bucket name you configured as the subdomain:
> ```text
> 127.0.0.1	      localhost      attune-dev-0.localhost
> 255.255.255.255	broadcasthost
> ::1             localhost
> ```
>
> You can validate that it is working properly by hitting its health endpoint:
> ```sh
> ; xh attune-dev-0.localhost:9000/minio/health/live
> ```

Finally, you may need to set the bucket as "public read only" for it to be read by `apt`:
```sh
; mc anonymous set download attune/attune-dev-0 -r
```

You can validate that this works with this:
```sh
; mc anonymous list attune/attune-dev-0 -r
attune-dev-0/* => readonly
```

## Set up `.env` file

Set up your `.env` file using the `.env.example` file as a base.

Make sure to follow any instructions in the comments,
and make sure that the values match the values you used when configuring services earlier.

> [!TIP]
> A convenient one-liner for loading the contents of the `.env` file into your current session
> is as follows:
> ```sh
> set -a; source .env; set +a
> ```

## Migrate the database

Migrations currently use Prisma.

To run a migration manually, install Node and navigate to the `docker/migrate` folder.
Install the dependencies and run migrations; make sure that `ATTUNE_DATABASE_URL` is set correctly.

```sh
; npm i
; npm run migrate
```

## Run Attune

Finally, you can now run the Attune server!

Make sure that your `.env` file is loaded or that you specify the arguments manually,
then run Attune with the following:
```sh
; cargo run attune
```

You can validate that the server is running by hitting the `/api/v0/repositories` endpoint.
Here's an example using [`xh`](https://github.com/ducaale/xh):
```sh
; xh :3000/api/v0/repositories --bearer $ATTUNE_API_TOKEN
```

Keep this server running; future steps will be in a new terminal window.

## Publish a package

Here, we'll publish a package to the locally running Attune control plane.

We have prebuilt packages for this use case, just grab one off the
[`attune-test-package`](https://github.com/attunehq/attune-test-package/releases)
release page.

> [!NOTE]
> The following steps assume that the path you've downloaded the package to test
> is at `$ATTUNE_TEST_PACKAGE_PATH`; either set that variable or replace it as desired.

Generate a GPG key and copy the 40-character hex encoded ID of the one you want to use:
```sh
# Fill out as desired; for development and local testing you can probably leave
# all the settings as their defaults.
; gpg --generate-key

# The ID here is 571B4D41C200EB7254ED932103A515124FB4EF14
; gpg --list-secret-keys
[keyboxd]
---------
sec   ed25519 2025-07-29 [SC] [expires: 2028-07-28]
      571B4D41C200EB7254ED932103A515124FB4EF14
uid           [ultimate] Jessica Black <me@jessica.black>
ssb   cv25519 2025-07-29 [E] [expires: 2028-07-28]
```

> [!NOTE]
> The following steps assume that the ID of the GPG key you want to use is set to
> `$ATTUNE_GPG_KEY_ID`; either set that as a variable or replace it as desired.

Create a "repository" in Attune and record the ID:
```sh
; attune repo create -u 'http://localhost:9000/debian'
Created new repository:
ID  URI                           Distribution
1   http://localhost:9000/debian  stable
```

> [!NOTE]
> The following steps assume that the ID of the Attune repository you want to use is set to
> `$ATTUNE_REPOSITORY_ID`; either set that as a variable or replace it as desired.

Add the package to the repository and sync the repository:
```sh
; attune repo -r $ATTUNE_REPOSITORY_ID pkg add $ATTUNE_TEST_PACKAGE_PATH
; attune repo -r $ATTUNE_REPOSITORY_ID sync -i $ATTUNE_GPG_KEY_ID
```

## Set up a testing Docker container

We'll set up a Docker container that pulls from the control plan that is running locally.

> [!NOTE]
> macOS users need to enable "host networking" in Docker Desktop to follow these steps.

You'll need the S3 prefix for the repository.
This is computed using the ID of the tenant performing the release in the database,
followed by a slash, followed by the hex-encoded sha256 hash of the repository URI.

The simplest way to find this is to look at the `debian_repository` database table,
or just look in the Minio interface if you have 1 or a small number of repositories.

> [!NOTE]
> The following steps assume that the S3 prefix you want to use is set to
> `$ATTUNE_REPOSITORY_S3_PREFIX`; either set that as a variable or replace it as desired.

Copy the public key of the GPG key that you used to sign the release:
```sh
gpg --armor --export $ATTUNE_GPG_KEY_ID
```

Start a Docker debian container and configure it to connect to the Attune repository:
```sh
; docker run -it --rm --network=host debian /bin/bash
; echo 'deb http://localhost:9000/attune-dev-0/$ATTUNE_REPOSITORY_S3_PREFIX stable main' > /etc/apt/sources.list
; cat >> /etc/apt/trusted.gpg.d/attune.asc << EOF
----- Paste your public key content, overwriting this entire line -----
EOF

# Install the test package
; apt update
; apt install attune-test-package

# Validate the package was installed successfully
; attune-test-package --version
```

You can also specify `platform` to test other platforms; for example if you're on an M-series MacBook you
can run something like the below to test x86_64 packages:
```sh
; docker run -it --rm --network=host --platform=linux/amd64 debian /bin/bash
```
