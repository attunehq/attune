pub mod api;
pub mod apt;
pub mod server;

// Generate static migrator from Prisma migrations.
attune_macros::prisma_migrate!("docker/migrate/prisma/migrations");
