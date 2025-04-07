# Design

ArmorCD is a secure, fast publishing tool for Debian packages.

## Definitions

In this document, there are two kinds of users:

1. **Distributors** are users of Armor, who develop and distribute packages using Armor.
2. **End users** are users of distributors' repositories, who install packages from distributors' repositories.

## Requirements

In this design, we focused on satisfying 3 primary requirements:

1. **Package and index signing occurs in the distributor's environment.** Signing keys must never leave the distributor's environment. We accomplish this with our local index signing workflow.

2. **Minimize hosting costs.** Make it cheap to host packages for distribution. We accomplish this by using Cloudflare as a data plane for distribution.

3. **Optimize publishing performance.** Make it fast to push changes to the repository. We accomplish this by saving incremental repository state and avoiding unnecessary recomputation where possible.

## Conceptual overview

### Repositories

Repositories are the top-level object in the system. Each repository represents a single Debian Repository.

Each repository has multiple _releases_, which are snapshots of the repository at a specific point in time. At any point in time, a repository may also have a single _active release_, which is the snapshot that is currently being served to clients.

To update a repository's contents, a distributor creates a new draft release, edits that draft release until it's ready for publishing, and then promotes the draft release to the active release. Index generation and signing occurs only once, when the active release is promoted. Once promoted, active releases are immutable.

Draft releases allow distributors to make incremental changes to the repository without needing to re-sign the index after every incremental change while still making sure that end users always see a valid repository state. When a release is promoted to active, end users see the repository change atomically from the old release to the new release.

### Releases

Releases are copy-on-write snapshots of packages.

When releases are created, they can be either _empty_ or they can be _copied from_ an existing release. If a release is copied from an existing release, it will inherit all fields and packages from the existing release. This makes it fast and easy to create new releases that are incremental changes from a previous release (e.g. with a new version of packages added, or with an undesirable package removed) without needing to rebuild the entire repository.

Note that this does not increase storage requirements, because package files are served from a content-addressed store.

### Packages

Releases contain packages. Distributors are responsible for building and signing packages before they're added to the repository.

## Architecture overview

Armor is composed of three pieces:

1. A CLI written in Go that runs in distributor environments (like the distributor's CI or their developers' laptops).
2. A data plane that hosts and stores packages, running on Cloudflare.
3. A control plane that manages repository metadata, package uploads, and index generation.

### CLI

The CLI is a Go binary that distributors run locally on their CI machines.

We use Go for its dead simple cross-compilation story.

### Data plane

The data plane is a Cloudflare Worker handling custom hostnames using Cloudflare for Platforms.

Distributors can serve packages on their own domain by adding a `CNAME` record from their desired domain to `package-cdn.armorcd.dev` and a `TXT` record for ACME certification issuance (see [here](https://developers.cloudflare.com/cloudflare-for-platforms/cloudflare-for-saas/start/getting-started/#3-have-customer-create-cname-record)).

When the Worker receives an incoming request, it checks Cloudflare D1 to see whether that request is for a known distributor repository. If so, it looks up the correct distributor, repository, and release for the request using its URL, and then uses that information to serve the appropriate object from R2.

We use Cloudflare R2 and Cloudflare Workers because they have no egress cost.

### Control plane

The control plane is a Rust server and Postgres database that runs in AWS. It manages uploads, access control, and keeping the metadata database up-to-date. This metadata database allows us to quickly re-generate indexes when needed, so distributors can quickly add and remove packages.

This control plane server lives in its own service because there are too many limitations in Cloudflare Workers (e.g. allowed runtime) for us to use that for our control plane.

## Getting started

To set up an account with Armor:

1. We will provision an account for you. This step is not currently self-serviceable.

2. Serve packages on your own subdomain by adding a `CNAME` record from your desired subdomain to `package-cdn.armorcd.dev` and a `TXT` record for ACME certification issuance (see [here](https://developers.cloudflare.com/cloudflare-for-platforms/cloudflare-for-saas/start/getting-started/#3-have-customer-create-cname-record)).

## Signing workflow

1. Create a repository with the subdomain you created during onboarding.
   ```
   armor repositories create --uri https://apt.releases.example.com --distribution bookworm
   ```

2. Create a release in the repository:
   ```
   armor releases --repository-id 123 create
   ```

3. Add packages to the release:
   ```
   armor pkgs --release-id 456 add --component main ./your_package.deb
   ```

4. Once all desired packages are uploaded, users use the CLI to generate, sign, and upload updated repository indexes:
   ```
   armor releases --repository-id 123 promote --release-id 456 --key=ABC123
   ```

## Future work

- Adding support for more package managers.
- Adding support for users to set manager-specific metadata (e.g. Ubuntu phased updates).
- Add multi-tenant support.
- Generating `keyring` packages for users to easily rotate and revoke GPG keys.
- Package generation and signing, possibly using secure enclaves with HSMs.
- Webhooks and API for user automation.
