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
    let input_path = PathBuf::from(input_lit.value());
    if !input_path.exists() {
        return syn::Error::new_spanned(
            input_lit,
            format!("Migrations directory not found: {input_path:?}"),
        )
        .to_compile_error()
        .into();
    }

    let entries = match collect_migrations(&input_path) {
        Ok(entries) => entries,
        Err(err) => {
            return syn::Error::new_spanned(input_lit, format!("Failed to read migrations: {err}"))
                .to_compile_error()
                .into();
        }
    };

    let migrations = entries.iter().map(|(version, description, sql_path)| {
        quote! {
            sqlx::migrate::Migration {
                version: #version,
                description: std::borrow::Cow::Borrowed(#description),
                migration_type: sqlx::migrate::MigrationType::Simple,
                sql: std::borrow::Cow::Borrowed(include_str!(#sql_path)),
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

fn collect_migrations(dir: &Path) -> Result<Vec<(i64, String, String)>, std::io::Error> {
    let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());

    let mut migrations = Vec::new();
    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };

        // Parse migration directory name format: {timestamp}_{description}
        let parts: Vec<&str> = dir_name.splitn(2, '_').collect();
        if parts.len() != 2 {
            continue;
        }

        let version = match parts[0].parse::<i64>() {
            Ok(v) => v,
            Err(_) => continue,
        };

        let description = parts[1].to_string();
        let migration_sql_path = path.join("migration.sql");

        if !migration_sql_path.exists() {
            continue;
        }

        // Convert to path relative to the crate root for include_str!
        // The migration sql path is absolute, we need it relative to the crate root
        let relative_sql_path = format!("../../../{}", migration_sql_path.to_string_lossy());

        migrations.push((version, description, relative_sql_path));
    }

    Ok(migrations)
}
