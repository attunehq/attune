// Generate static migrator from Prisma migrations.
//
// Note that this particular symbol cannot be `#[cfg(test)]`, because it's used
// in the integration test binary, which forms a separate crate.
attune_macros::prisma_migrate!("docker/migrate/prisma/migrations");

mod ephemeral;
mod server;

pub use ephemeral::*;
pub use server::*;
