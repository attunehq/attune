# Attune User Guide

Attune is a tool for securely publishing and hosting Linux packages.

For a quick start guide to get up and running in 5 minutes, see the [Quick Start section in the main README](../../README.md#quick-start).

> [!NOTE]
> This guide is for **Attune Cloud** users. Are you using a self-hosted Attune instance? Check out our [self-hosting guide](./self-hosting.md) instead.

<!-- TODO: Maybe we should move Cloud documentation to the docs site, and instead spotlight the self-hosting documentation? -->

## Getting Started

Download the `attune` CLI from [GitHub Releases](https://github.com/attunehq/attune/releases).

Once that's ready, you'll need to set your `$ATTUNE_API_TOKEN` environment variable to the API token that you received during signup.

## Publishing packages

### Basic concepts

Every APT package is stored in a _repository_. This is what your users will ultimately install your packages from.

Your account should come provisioned with a default repository. You can view it using:

```bash
$ attune apt repo list
```

This repository is tied to the subdomain configured during signup. When you publish packages to this repository, they'll be available at your configured subdomain.

Each repository is also split into a set of _distributions_ and a set of _components_. For complicated projects, these can be used to group your packages. For example, you might want to have a different distribution for each version line of your package, or a `stable` distribution separate from a `canary` one.

**Most projects don't need these features.** By default, Attune provides smart defaults for these fields for you. You don't need to worry about them at all. If you want to set your own defaults, check out:

```bash
$ attune apt distribution --help
```

### Publishing packages

In order to publish a package, you'll need the package file (i.e. a `.deb` file), and a GPG signing key for signing your repository indexes.

If you don't have one, you can generate one using `gpg --generate-key`. **Keep this signing key safe!** Attackers who gain access to your signing key may attempt to construct fake repositories that appear real, and use that to phish your users into installing their malicious code. By default, Attune always signs locally, so your GPG key never leaves your machine. This is provides much less attack surface than cloud-hosted signing services.

Once you have your package and GPG key, run:

```bash
$ attune apt package add \
  --repo $YOUR_REPO_NAME \
  --key-id $YOUR_GPG_KEY_ID \
  $PATH_TO_YOUR_PACKAGE
```

And that's it! Your package has been published, and should be available on the Internet now.

### Installing your published packages

Now that your packages are published, your users can install them. For your users to install your packages, they'll need to configure their `apt` client to use your repository.

First, they'll need to download your repository's public key and make it available to their `apt` keyring. To get your public key, use:

```bash
$ gpg --armor --export $YOUR_GPG_KEY_ID
```

You should name this file after your organization (e.g. `attune.asc`), and upload it somewhere that your users can access (e.g. your website). They'll need to add this file to their system in `/etc/apt/keyrings/`. For example, if you named your public key `example.asc`, your users would add it to `/etc/apt/keyrings/example.asc`.

Next, they'll need to add the following line to `/etc/apt/sources.list`:

```
deb [signed-by=/etc/apt/keyrings/$YOUR_PUBLIC_KEY_FILE] $YOUR_REPOSITORY_URL $DISTRIBUTION $COMPONENT
```

> [!TIP]
> If you didn't specify a distribution and component name when you uploaded your package, then your default distribution name is `stable` and your default component name is `main`.

For example, if your key was named `example.asc`, your repository URL was `apt.example.attunehq.com/debian`, and you used the default distribution and component, your users might add:

```
deb [signed-by=/etc/apt/keyrings/example.asc] apt.example.attunehq.com/debian stable main
```

Once this is done, they can run `apt update` to update their package list, and then `apt install` to install your package.
