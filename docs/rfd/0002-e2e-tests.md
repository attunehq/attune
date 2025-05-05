# RFD 0002: End-to-End Testing with testcontainers

**Authors:**
**Status:** Draft  
**Created:** 2025-05-01  
**Updated:** 2025-05-01

---

## Summary

This RFD proposes introducing automated end-to-end (E2E) testing for the Attune backend using the [testcontainers](https://crates.io/crates/testcontainers) Rust crate. All core services—including the control plane, PostgreSQL, and MinIO—will be started as containers to closely mirror the production and documented setup. The goal is to ensure that our documented setup and workflow (as outlined in the README) work reliably and reproducibly in CI and local development, covering integration between the control plane, PostgreSQL, MinIO, and the Attune CLI.

---

## Motivation

- **Reliability:** Manual testing is error-prone and time-consuming. Automated E2E tests ensure our critical workflows (repo creation, package upload, signing, etc.) are always functional.
- **Documentation Drift:** The README and onboarding docs can become outdated. E2E tests validate that instructions work as described.
- **Regression Prevention:** Catch integration issues early when dependencies or code change.
- **CI Integration:** Automated E2E tests can run in CI, providing confidence in every PR.

---

## Guide-Level Explanation

### What will be tested?

- The full Attune backend setup: control plane, PostgreSQL, and MinIO, all running as containers.
- Attune CLI workflows: repo creation, package upload, signing, and syncing.
- GPG key generation and usage for signing repositories.

### How will it work?

- Use testcontainers to spin up PostgreSQL, MinIO, and the Attune control plane as isolated Docker containers for each test run.
- The control plane container will be configured to communicate with the test containers for PostgreSQL and MinIO.
- Use Rust’s `std::process::Command` to invoke the Attune CLI and GPG commands as subprocesses.
- Download a sample `.deb` package for upload as part of the test.
- Verify expected results at each step (e.g., repo created, package uploaded, repo signed).

---

## Reference-Level Explanation

### Implementation Plan

1. **Add testcontainers as a dev-dependency in Cargo.toml.**
2. **Write integration tests under `controlplane/tests/` that:**
   - Use testcontainers to start PostgreSQL, MinIO, and the Attune control plane (all as containers).
   - Launch the Attune CLI and GPG commands using `std::process::Command`.
   - Automate GPG key generation and export.
   - Download a sample `.deb` package for upload.
   - Assert correct workflow and output at each step.
3. **Integrate the test into CI.**
   - Ensure Docker is available in the CI environment.
   - Run the E2E test as part of the test suite.

### Example Test Flow

```rust
// Pseudocode outline
let docker = clients::Cli::default();
let postgres = images::postgres::Postgres::default();
let minio = images::generic::GenericImage::new("minio/minio", "latest");
let control_plane = images::generic::GenericImage::new("attune/controlplane", "latest");

let pg_container = docker.run(postgres);
let minio_container = docker.run(minio);
let cp_container = docker.run(control_plane);
// ...rest of test flow
```

---

## Drawbacks

- **Test speed:** E2E tests are slower than unit tests due to container startup and process orchestration.
- **Complexity:** Increases test and CI complexity.
- **Docker dependency:** Requires Docker to be available in dev and CI environments.

---

## Alternatives

- Mocking external services (PostgreSQL, MinIO) instead of running real containers (less realistic).
- Manual E2E testing (not scalable, error-prone).
- Using a different E2E framework (e.g., shell scripts, Python’s pytest + docker-compose).

---

## Unresolved Questions

- How do we build and tag the control plane container image for testing (e.g., local build vs. pulling from a registry)?
- How do we inject configuration and environment variables into the control plane container at test time?
- How do we manage test data and cleanup between runs?
- How do we handle flaky tests or network issues in CI?

---

## Prior Art

- [testcontainers-rs documentation](https://docs.rs/testcontainers/latest/testcontainers/)
- Existing RFDs: 0000-rfds.md, 0001-initial-design.md

---

## Next Steps

- Review and approve this RFD.
- Implement a prototype E2E test using testcontainers.
- Integrate with CI and iterate.
