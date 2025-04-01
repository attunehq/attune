# Design

## Workflow

1. Set up repositories
2. Upload package files
   1. Provide signed package file, and flags for repository, distribution, component
3. Database stores package files, parsed package metadata, and repository metadata
4. On each upload, re-generate Release and Packages indexes using incremental changes
5. Give generated indexes back to CLI for signing locally
6. Upload signed indexes

## Architecture

CLI runs in customer environment:
- Built in Rust? Go?
  - Probably not Haskell because we need a cross-compilation story
  - Go can use libraries from `nfpm` and friends
  - Rust has sane language design
- Talk to the API for uploads and control

API runs in Cloudflare Worker:
- Use Hono router
- TODO:
  - Will need to re-deploy whenever bindings are updated, if we switch to per-tenant D1 and R2 bindings
  - We could theoretically host this in a server, although we'd need to keep the server up and use a cheap-egress host because the server host's egress will still be billed

D1 is used for database of metadata:
- Put everything in one database for now
  - Tenants
  - For each tenant:
    - Repositories, packages, files, etc.
- TODO:
  - Break out tenants into their own databases
  - Move off of D1 onto something more scalable when we near the 10G limit

R2 used for package storage:
- Put everything in one bucket for now
- TODO:
  - Separate buckets for each tenant

## Backlog

- Terraform provider?
- Webhooks?
- Monitoring?
  - Integration testing to make sure package signed correctly
- Ubuntu phased updates
- Other package managers
- Generating the packages themselves
- Making signing nicer, maybe using enclave or KMS for signing or auto-generating a keyring package for rotation?
  - Add `Depends: tenant-keyring` to all packages
