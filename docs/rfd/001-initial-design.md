# Design

ArmorCD is a secure, fast publishing tool for Debian packages.

## Definitions

In this document, there are two kinds of users:

1. **Developers** are users of Armor, who develop and distribute packages using Armor.
2. **Consumers** are users of developers' repositories, who install packages from developers' repositories.

## Requirements

In this design, we focused on satisfying 3 primary requirements:

1. **Package and index signing occurs in the developer's environment.** Signing keys must never leave the developer's environment. We accomplish this with our local index signing workflow.

2. **Minimize hosting costs.** Make it cheap to host packages for distribution. We accomplish this by using CDNs and object storage providers as a data plane for distribution.

3. **Optimize publishing performance.** Make it fast to push changes to the repository. We accomplish this by saving incremental repository state and avoiding unnecessary recomputation where possible.

## Conceptual overview

### Repositories

Repositories are the top-level object in the system. Each repository represents a single Debian Repository.

Each repository has an _active release_ and a _staging area_, just like Git. The _active release_ contains the contents being served to consumers at any particular point in time.

In order to update a repository's contents, a developer makes changes in the repository's _staging area_. When these changes are ready, they _commit_ the changes into the active release. These changes then become visible to consumers.

### Packages

Releases contain packages. Developers are responsible for building and signing packages before they're added to the repository.

## Architecture overview

Armor is composed of three pieces:

1. A CLI that runs in developer environments (like the developer's CI or their developers' laptops).
2. A control plane that manages repository metadata, package uploads, and index generation.
3. A data plane that hosts and stores packages.

### CLI

The CLI is a Go binary that developers run locally on their CI machines.

We use Go for its dead simple cross-compilation story.

### Control plane

The control plane is a Rust server and Postgres database. It manages uploads, access control, and keeping the metadata database up-to-date. This metadata database allows us to quickly re-generate indexes when needed, so developers can quickly add and remove packages.

### Data plane

The data plane is served by an object storage provider behind a CDN. We currently publish to any S3-compatible object storage provider. For users doing a self-hosted deployment, we recommend [Minio](https://min.io/).

## Signing workflow

1. Create a repository with the subdomain you created during onboarding.
   ```
   armor repo create --uri https://apt.releases.example.com --distribution bookworm
   ```

3. Add packages to the staging area of the repository:
   ```
   armor repo pkg add --repo-id 123 add --component main ./your_package.deb
   ```

4. Review your changes to make sure everything looks correct:
   ```
   armor repo status --repo-id 123
   ```

5. If everything looks right, commit the changes:
   ```
   armor repo sync --repo-id 123 --signing-key-file=/tmp/signing.asc
   ```
   During the signing workflow, the backend generates indexes and sends them to the CLI for signing. The CLI then signs these indexes locally and sends the signed indexes back up to the backend. The signing key never leaves the developer environment.

## Future work

- Adding support for more package managers.
- Adding support for users to set manager-specific metadata (e.g. Ubuntu phased updates).
- Add multi-tenant support.
- Generating `keyring` packages for users to easily rotate and revoke GPG keys.
- Package generation and signing, possibly using secure enclaves with HSMs.
- Webhooks and API for user automation.
