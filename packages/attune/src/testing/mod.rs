// Generate static migrator from Prisma migrations.
//
// Note that this particular symbol cannot be `#[cfg(test)]`, because it's used
// in the integration test binary, which forms a separate crate.
attune_macros::prisma_migrate!("docker/migrate/prisma/migrations");

#[cfg(test)]
mod server;

#[cfg(test)]
pub use server::*;

#[cfg(test)]
pub mod ephemeral;
