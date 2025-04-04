# Design

## Signing workflow

1. Set up a repository in the control plane database. Users cannot currently self-provision this step.
2. Users upload package files into the repository using the CLI.
3. Once all desired packages are uploaded, users use the CLI to generate, sign, and upload updated repository indexes.

## Architecture

Overall, the architecture consists of:

1. A CLI that users run locally on their CI machines.
2. A data plane that hosts and stores packages, running on Cloudflare.
3. A control plane that manages repository metadata, package uploads, and index generation.

### CLI

The CLI is a Go binary that users run locally on their CI machines.

We use Go for its dead simple cross-compilation story.

### Data plane

The data plane is an extremely simple Cloudflare Worker. When the worker receives an incoming request, it checks whether the corresponding object exists in R2, and if so, returns it. If not, it returns a 404.

We use Cloudflare R2 and Cloudflare Workers here because they have no egress cost.

### Control plane

The control plane is a Haskell server and Postgres database that runs in AWS. It manages uploads, access control, and keeping the metadata database up-to-date. This metadata database allows us to quickly re-generate indexes when needed, so users can quickly add and remove packages.

## Future work

- Adding support for more package managers.
- Adding support for users to set manager-specific metadata (e.g. Ubuntu phased updates).
- Generating `keyring` packages for users to easily rotate and revoke GPG keys.
- Package generation and signing, possibly using secure enclaves with HSMs.
- Webhooks and API for user automation.
