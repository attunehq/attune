// Note that these symbols are not behind `#[cfg(test)]`, because they need to
// be included in other crates (specifically, the integration test crate and the
// attune binary crate's unit tests).

// Generate static migrator from Prisma migrations.
attune_macros::prisma_migrate!("docker/migrate/prisma/migrations");

pub mod ephemeral;
pub mod fixtures;
pub mod server;

pub use ephemeral::*;
pub use fixtures::*;
pub use server::*;
