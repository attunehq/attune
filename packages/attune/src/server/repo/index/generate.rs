use std::{collections::HashSet, io::Write, iter::once};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest as _, Sha256};
use sqlx::types::JsonValue;
use tabwriter::TabWriter;
use time::{OffsetDateTime, format_description::well_known::Rfc2822};
use tracing::instrument;

use crate::{
    api::ErrorResponse,
    auth::TenantID,
    server::{ServerState, repo::decode_repo_name},
};

#[derive(Serialize, Deserialize, Debug)]
pub struct Repository {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GenerateIndexRequest {
    pub repository: String,
    pub distribution: String,
    pub component: String,

    pub package_sha256sum: String,
    pub change: IndexChange,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum IndexChange {
    Add,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GenerateIndexResponse {
    pub release: String,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(repo_name): Path<String>,
    Json(req): Json<GenerateIndexRequest>,
) -> Result<Json<GenerateIndexResponse>, ErrorResponse> {
    // The repository name in the path is percent-encoded.
    let repo_name = decode_repo_name(&repo_name)?;

    let mut tx = state.db.begin().await.unwrap();
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();

    // Load the package to be either added or removed.
    let changed_package = match sqlx::query!(
        r#"
            SELECT
                paragraph,
                package,
                version,
                architecture::TEXT AS "architecture!: String",
                size,
                md5sum,
                sha1sum,
                sha256sum
            FROM debian_repository_package
            WHERE
                tenant_id = $1
                AND sha256sum = $2
        "#,
        tenant_id.0,
        req.package_sha256sum
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap()
    {
        Some(pkg) => pkg,
        None => {
            return Err(ErrorResponse::new(
                StatusCode::NOT_FOUND,
                "PACKAGE_NOT_FOUND".to_string(),
                "package not found".to_string(),
            ));
        }
    };

    // Load the release. Note that the release may not exist if no packages have
    // been added to this distribution.
    let release = sqlx::query_as!(Release, r#"
        SELECT
            debian_repository_release.id,
            debian_repository_release.origin,
            debian_repository_release.label,
            debian_repository_release.version,
            debian_repository_release.suite,
            debian_repository_release.codename,
            debian_repository_release.description
        FROM
            debian_repository
            JOIN debian_repository_release ON debian_repository_release.repository_id = debian_repository.id
        WHERE
            debian_repository.tenant_id = $1
            AND debian_repository.name = $2
            AND debian_repository_release.distribution = $3
        LIMIT 1
        "#,
        tenant_id.0,
        repo_name,
        req.distribution,
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap();

    // Load the other packages in the changed `Packages` index, which is all
    // packages that have the same (release, component, architecture) as the
    // changed package.
    let packages = if let Some(ref release) = release {
        sqlx::query_as!(Package, r#"
            SELECT
                debian_repository_package.paragraph,
                debian_repository_component_package.filename,
                debian_repository_package.size,
                debian_repository_package.md5sum,
                debian_repository_package.sha1sum,
                debian_repository_package.sha256sum
            FROM
                debian_repository_component
                JOIN debian_repository_component_package ON debian_repository_component_package.component_id = debian_repository_component.id
                JOIN debian_repository_package ON debian_repository_package.id = debian_repository_component_package.package_id
            WHERE
                debian_repository_component.release_id = $1
                AND debian_repository_component.name = $2
        "#, release.id, req.component)
        .fetch_all(&mut *tx)
        .await
        .unwrap()
    } else {
        // If the release doesn't exist, there are can't possibly be any
        // existing packages in the release.
        Vec::new()
    };

    // Create the new package set.
    let packages = match req.change {
        IndexChange::Add => {
            let filename = render_pool_filename(PackageId {
                name: &changed_package.package,
                version: &changed_package.version,
                architecture: &changed_package.architecture,
                component: &req.component,
            });
            let added = Package {
                paragraph: changed_package.paragraph,
                filename,
                size: changed_package.size,
                md5sum: changed_package.md5sum,
                sha1sum: changed_package.sha1sum,
                sha256sum: changed_package.sha256sum,
            };
            packages.into_iter().chain(once(added))
        }
    };

    // Generate the `Packages` index for the `(distribution, component, arch)`.
    let changed_packages_index = {
        let rendered = render_package_index(packages);
        PackageIndexMetadata::from_package_index(
            &req.component,
            &changed_package.architecture,
            &rendered,
        )
    };

    // Generate the `Release` file for the distribution.
    let release_file = {
        // Load all other `Packages` indexes for the release.
        let packages_indexes = if let Some(ref release) = release {
            sqlx::query_as!(PackageIndexMetadata, r#"
                SELECT
                    debian_repository_component.name AS component,
                    debian_repository_index_packages.architecture::TEXT AS "architecture!: String",
                    debian_repository_index_packages.size,
                    debian_repository_index_packages.md5sum,
                    debian_repository_index_packages.sha1sum,
                    debian_repository_index_packages.sha256sum
                FROM
                    debian_repository_release
                    JOIN debian_repository_component ON debian_repository_component.release_id = debian_repository_release.id
                    JOIN debian_repository_index_packages ON debian_repository_index_packages.component_id = debian_repository_component.id
                WHERE
                    debian_repository_release.id = $1
                "#,
                release.id,
            )
            .fetch_all(&mut *tx)
            .await
            .unwrap()
        } else {
            // If the distribution doesn't exist, it can't possibly have existing
            // `Packages` indexes.
            Vec::new()
        }.into_iter();

        // Remove the changed `Packages` index.
        let packages_indexes = packages_indexes.filter(|packages_index| {
            !(packages_index.component == req.component
                && packages_index.architecture == changed_package.architecture)
        });

        // Add the new `Packages` index.
        let packages_indexes = packages_indexes.chain(once(changed_packages_index));

        // When the Release is missing, we use one with default values instead.
        let release = match release {
            Some(release) => release,
            None => Release {
                // HACK: Can set `id` to whatever since it isn't used in
                // rendering.
                id: 0,
                description: None,
                origin: None,
                label: None,
                version: None,
                suite: req.distribution.clone(),
                codename: req.distribution.clone(),
            },
        };

        render_release_file(release, packages_indexes)
    };

    tx.commit().await.unwrap();

    Ok(Json(GenerateIndexResponse {
        release: release_file,
    }))
}

struct Package {
    paragraph: JsonValue,
    filename: String,
    size: i64,
    md5sum: String,
    sha1sum: String,
    sha256sum: String,
}

struct PackageId<'a> {
    name: &'a str,
    version: &'a str,
    architecture: &'a str,
    component: &'a str,
}

fn render_pool_filename(id: PackageId) -> String {
    let component = id.component;
    // FIXME: This isn't actually correct! Some documentation online
    // indicates that the package name in the pool filename should
    // actually be the _source_ package name, not the binary package
    // name.
    //
    // The source package's name might be different from the binary
    // package! However, most users of our tool generally don't care
    // about grouping their binary packages into source packages, and
    // there's no way to determine the origin source package by just
    // examining a binary package, so we just pretend it's the binary
    // package name and call it a day.
    let source_package_name = id.name;
    let source_package_name_start = source_package_name.chars().next().unwrap();

    let binary_package_name = id.name;
    let version = id.version;
    let architecture = id.architecture;
    format!(
        "pool/{component}/{source_package_name_start}/{source_package_name}/{binary_package_name}_{version}_{architecture}.deb"
    )
}

fn render_package_index(packages: impl Iterator<Item = Package>) -> String {
    let mut index = packages
        .map(|pkg| {
            let fields = pkg
                .paragraph
                .as_object()
                .unwrap()
                .into_iter()
                .map(|(k, v)| format!("{}: {}", k, v.as_str().unwrap()))
                .chain(vec![
                    format!("Filename: {}", pkg.filename),
                    format!("Size: {}", pkg.size.to_string()),
                    format!("MD5sum: {}", pkg.md5sum),
                    format!("SHA1: {}", pkg.sha1sum),
                    format!("SHA256: {}", pkg.sha256sum),
                ])
                .collect::<Vec<String>>();
            fields.join("\n")
        })
        .collect::<Vec<String>>()
        .join("\n\n");
    index.push('\n');
    index
}

struct PackageIndexMetadata {
    component: String,
    architecture: String,
    size: i64,
    md5sum: String,
    sha1sum: String,
    sha256sum: String,
}

impl PackageIndexMetadata {
    fn from_package_index(component: &str, architecture: &str, package_index: &str) -> Self {
        Self {
            component: component.to_string(),
            architecture: architecture.to_string(),
            size: package_index.len() as i64,
            md5sum: hex::encode(Md5::digest(package_index)),
            sha1sum: hex::encode(Sha1::digest(package_index)),
            sha256sum: hex::encode(Sha256::digest(package_index)),
        }
    }
}

struct Release {
    id: i64,
    description: Option<String>,
    origin: Option<String>,
    label: Option<String>,
    version: Option<String>,
    suite: String,
    codename: String,
}

fn render_release_file(
    release: Release,
    package_indexes: impl Iterator<Item = PackageIndexMetadata>,
) -> String {
    // TODO: Worth it to make this streaming?
    let package_indexes = package_indexes.collect::<Vec<_>>();

    // Note that the date format is RFC 2822. _Technically_, the Debian spec
    // says it should be the date format of `date -R -u`, which technically
    // is RFC 5322, but these formats are compatible. 5322 is a later
    // revision of 2822 that retains backwards compatibility.
    let date = OffsetDateTime::now_utc().format(&Rfc2822).unwrap();
    let mut arch_set = HashSet::new();
    let mut comp_set = HashSet::new();
    for p in &package_indexes {
        arch_set.insert(p.architecture.as_str());
        comp_set.insert(p.component.as_str());
    }
    let archs = arch_set
        .into_iter()
        .fold(String::new(), |acc_archs, arch| acc_archs + " " + &arch);
    let comps = comp_set
        .into_iter()
        .fold(String::new(), |acc_comps, comp| acc_comps + " " + &comp);
    let release_fields: Vec<(&str, Option<String>)> = vec![
        ("Origin", release.origin.clone()),
        ("Label", release.label.clone()),
        ("Version", release.version.clone()),
        ("Suite", Some(release.suite.clone())),
        ("Codename", Some(release.codename.clone())),
        ("Date", Some(date)),
        ("Architectures", Some(archs)),
        ("Components", Some(comps)),
        ("Description", release.description.clone()),
    ];
    let mut release_file = String::new();
    for (k, v) in release_fields {
        if let Some(v) = v {
            release_file.push_str(&format!("{k}: {v}\n"));
        }
    }

    release_file += "MD5Sum:\n";
    let mut md5writer = TabWriter::new(vec![]);
    for index in &package_indexes {
        // TODO: Handle compressed indexes.
        writeln!(
            &mut md5writer,
            " {}\t{} {}/binary-{}/Packages",
            index.md5sum, index.size, index.component, index.architecture
        )
        .unwrap();
    }
    md5writer.flush().unwrap();
    release_file = release_file + &String::from_utf8(md5writer.into_inner().unwrap()).unwrap();

    release_file += "SHA256:\n";
    let mut sha256writer = TabWriter::new(vec![]);
    for index in &package_indexes {
        // TODO: Handle compressed indexes.
        writeln!(
            &mut sha256writer,
            " {}\t{} {}/binary-{}/Packages",
            index.sha256sum, index.size, index.component, index.architecture
        )
        .unwrap();
    }
    sha256writer.flush().unwrap();
    release_file = release_file + &String::from_utf8(sha256writer.into_inner().unwrap()).unwrap();
    release_file
}
