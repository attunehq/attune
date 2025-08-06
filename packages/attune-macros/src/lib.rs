use proc_macro::TokenStream;
use quote::quote;
use std::path::Path;
use std::{fs, path::PathBuf};
use syn::{parse_macro_input, LitStr};

/// Generate a static migrator from Prisma migrations directory.
///
/// Usage:
/// ```ignore
/// prisma_migrate!("path/to/prisma/migrations")
/// ```
///
/// This generates a `pub static MIGRATOR: sqlx::migrate::Migrator` that can be used
/// with sqlx's migration system.
#[proc_macro]
pub fn prisma_migrate(input: TokenStream) -> TokenStream {
    let input_lit = parse_macro_input!(input as LitStr);
    let migrations_dir = PathBuf::from(input_lit.value());
    if !migrations_dir.exists() {
        return syn::Error::new_spanned(
            input_lit,
            format!("Migrations directory not found: {migrations_dir:?}"),
        )
        .to_compile_error()
        .into();
    }

    let entries = match collect_migrations(&migrations_dir) {
        Ok(entries) => entries,
        Err(err) => {
            return syn::Error::new_spanned(input_lit, format!("Failed to read migrations: {err}"))
                .to_compile_error()
                .into();
        }
    };

    // It's a big pain to do checksums today, so we're putting that off until they
    // are proven to be worthwhile.
    let migrations = entries.into_iter().map(|migration| {
        let version = migration.version;
        let description = migration.description;
        let path = migration.path;
        quote! {
            sqlx::migrate::Migration {
                version: #version,
                description: std::borrow::Cow::Borrowed(#description),
                migration_type: sqlx::migrate::MigrationType::Simple,
                sql: std::borrow::Cow::Borrowed(include_str!(#path)),
                checksum: std::borrow::Cow::Borrowed(&[]),
                no_tx: false,
            }
        }
    });

    let expanded = quote! {
        pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Borrowed(&[
                #(#migrations),*
            ]),
            ignore_missing: false,
            locking: true,
            no_tx: false,
        };
    };

    TokenStream::from(expanded)
}

struct PrismaMigration {
    version: i64,
    description: String,
    path: String,
}

fn collect_migrations(dir: &Path) -> Result<Vec<PrismaMigration>, std::io::Error> {
    let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());

    let mut migrations = Vec::new();
    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let dir = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };

        let path = path.join("migration.sql");
        if !path.exists() {
            continue;
        }

        // Parse migration directory name format: {timestamp}_{description}
        let Some((version, description)) = dir.split_once('_') else {
            continue;
        };

        let version = match version.parse::<i64>() {
            Ok(v) => v,
            Err(_) => continue,
        };

        let path = format!("../../../{}", path.to_string_lossy());
        let description = description.to_string();
        migrations.push(PrismaMigration {
            version,
            description,
            path,
        });
    }

    Ok(migrations)
}
