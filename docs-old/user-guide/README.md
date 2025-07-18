# Attune User Guide

Attune is a tool for securely publishing and hosting Linux packages.

For a quick start guide to get up and running in 5 minutes, see the [Quick Start section in the main README](../../README.md#quick-start).

If you are self-hosting Attune, check out our [self-hosting guide](./self-hosting.md).

## Getting the CLI

Download the CLI from [GitHub Releases](https://github.com/attunehq/attune/releases).

## Authenticating with the CLI

Set `ATTUNE_API_TOKEN` to your provided API token.

<details>
<summary>Additional setup for self-hosting</summary>

If you're self-hosting, use the same `ATTUNE_API_TOKEN` as the one you set for your control plane. You'll also need to set `ATTUNE_API_ENDPOINT` to the base URL of your control plane (e.g. `http://localhost:3000`).

</details>

## Publishing packages

Once everything is set up, here's how you can publish a package:

1. Create a repository using `attune repo create`. You'll need to pass some repository fields as flags. These are defined by [the Debian repository format for Release files](https://wiki.debian.org/DebianRepository/Format#A.22Release.22_files). If you're self-hosting, you can set `uri` to anything.
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
