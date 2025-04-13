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

The data plane is served by a pluggable CDN and object storage provider. We currently support two configurations:

1. Cloudflare Workers and R2 in a cloud configuration (your packages will be hosted in Armor's cloud).
2. S3 and CloudFront in a self-hosted configuration (your packages will be hosted on your own infrastructure).

#### Cloudflare (cloud)

:rotating_light: TODO: This needs to be updated after implementation.

The data plane is a Cloudflare Worker handling custom hostnames using Cloudflare for Platforms.

Developers can serve packages on their own domain by adding a `CNAME` record from their desired domain to `package-cdn.armorcd.dev` and a `TXT` record for ACME certification issuance (see [here](https://developers.cloudflare.com/cloudflare-for-platforms/cloudflare-for-saas/start/getting-started/#3-have-customer-create-cname-record)).

When the Worker receives an incoming request, it checks Cloudflare D1 to see whether that request is for a known developer repository. If so, it looks up the correct developer, repository, and release for the request using its URL, and then uses that information to serve the appropriate object from R2.

We use Cloudflare R2 and Cloudflare Workers because they have no egress cost.

#### S3 (self-hosted)

:construction: TODO: Finish design here.

#### Determining which configuration to use

:construction: TODO: Finish.

<!-- Main differences are cost, self-hosting, egress, AWS committed spend, etc. -->

## Getting started

To set up an account with Armor:

1. We will provision an account for you. This step is not currently self-serviceable.

If you are using the cloud configuration:

2. Serve packages on your own subdomain by adding a `CNAME` record from your desired subdomain to `package-cdn.armorcd.dev` and a `TXT` record for ACME certification issuance (see [here](https://developers.cloudflare.com/cloudflare-for-platforms/cloudflare-for-saas/start/getting-started/#3-have-customer-create-cname-record)).

## Signing workflow

1. Create a repository with the subdomain you created during onboarding.
   ```
   armor repositories create --uri https://apt.releases.example.com --distribution bookworm
   ```

3. Add packages to the staging area of the repository:
   ```
   armor pkgs --repository-id 123 add --component main ./your_package.deb
   ```

4. Review your changes to make sure everything looks correct:
   ```
   armor repositories --repository-id 123 status
   ```

5. If everything looks right, commit the changes:
   ```
   armor repositories --repository-id 123 sync --index-signing-key=ABC123
   ```

## Future work

- Adding support for more package managers.
- Adding support for users to set manager-specific metadata (e.g. Ubuntu phased updates).
- Add multi-tenant support.
- Generating `keyring` packages for users to easily rotate and revoke GPG keys.
- Package generation and signing, possibly using secure enclaves with HSMs.
- Webhooks and API for user automation.
