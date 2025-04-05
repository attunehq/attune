# Design

## Signing workflow

1. Set up a repository in the control plane database. This is currently not self-serveable. On our side, we'll set up a Custom Hostname manually in Cloudflare.

2. Users upload package files into the repository using the CLI:
   ```
   armor pkgs add \
     --repository apt.releases.example.com \
     --dist bookworm \
     --component stable/v17 \
     ./package.deb
   ```

3. Once all desired packages are uploaded, users use the CLI to generate, sign, and upload updated repository indexes:
   ```
   armor index generate \
     --repository apt.releases.example.com \
     --dist bookworm \
     > ./unsigned.toml
   armor index sign --key=ABC123 ./unsigned.toml > ./signed.toml
   armor index upload \
     --repository apt.releases.example.com \
     --dist bookworm \
     ./signed.toml
   ```

   :warning: TODO: Update this to reflect the new release ID / active release model.

<!-- How do we make index generation fast?

We probably need to make releases mutable, and re-generate index files incrementally as packages are added. Or maybe we can track which indexes need to be regenerated / have been invalidated, and just regenerate those? (To avoid eagerly regenerating intermediate index files)
- Maybe we can track this using createdAt/updatedAt timestamps

We can probably also incrementally regenerate indexes in the background asynchronously, although synchronization might get a little messy

-->

## Conceptual model

### Repositories

Repositories are the top-level object in the system. Each repository represents a single Debian Repository.

Repositories have a base URL (e.g. `apt.releases.example.com`) and a distribution name (e.g. `bookworm`), which are configured by the user. These define the URL at which the repository will be served.

Each repository has multiple _releases_, which are snapshots of the repository at a specific point in time. At any point in time, a repository may also have a single _active release_, which is the snapshot that is currently being served to clients.

To update a repository's contents, users should create a new release, modify that release, and then switch the active release to the new release. Once a release has been activated, it can no longer be modified.

### Releases

Releases are copy-on-write snapshots of packages.

When releases are created, they can be either _empty_ or they can be made _from_ an existing release. If a release is made from an existing release, it will inherit all packages from the existing release. Note that this does not increase storage requirements, because package files are served from a content-addressed store.

### Packages

:warning: TODO: Finish this.

## Architecture

Overall, the architecture consists of:

1. A CLI that users run locally on their CI machines.
2. A data plane that hosts and stores packages, running on Cloudflare.
3. A control plane that manages repository metadata, package uploads, and index generation.

### CLI

The CLI is a Go binary that users run locally on their CI machines.

We use Go for its dead simple cross-compilation story.

### Data plane

The data plane is a Cloudflare Worker handling custom hostnames using Cloudflare for Platforms.

Users can serve packages on their own domain by adding a `CNAME` record from their desired domain to `package-cdn.armorcd.dev` and a `TXT` record for ACME certification issuance (see [here](https://developers.cloudflare.com/cloudflare-for-platforms/cloudflare-for-saas/start/getting-started/#3-have-customer-create-cname-record)).

When the data plane Worker receives an incoming request, it checks Cloudflare D1 to see whether that request is for a configured, recognized hostname. If so, it looks up the latest release ID for that hostname and uses that to serve the requested object from R2.

:construction: TODO: Talk about content addressing and D1 schema.

We use Cloudflare R2 and Cloudflare Workers here because they have no egress cost.

### Control plane

The control plane is a Rust server and Postgres database that runs in AWS. It manages uploads, access control, and keeping the metadata database up-to-date. This metadata database allows us to quickly re-generate indexes when needed, so users can quickly add and remove packages.

## Future work

- Adding support for more package managers.
- Adding support for users to set manager-specific metadata (e.g. Ubuntu phased updates).
- Generating `keyring` packages for users to easily rotate and revoke GPG keys.
- Package generation and signing, possibly using secure enclaves with HSMs.
- Webhooks and API for user automation.
