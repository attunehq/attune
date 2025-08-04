use std::{collections::BTreeSet, io::Write, iter::once};

use axum::http::StatusCode;
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest as _, Sha256};
use sqlx::{Postgres, Transaction, prelude::FromRow, types::JsonValue};
use tabwriter::{Alignment, TabWriter};
use time::{OffsetDateTime, format_description::well_known::Rfc2822};
use tracing::{debug, instrument};

use crate::{api::ErrorResponse, auth::TenantID};

pub mod generate;
pub mod sign;

#[derive(Serialize, Deserialize, Debug)]
pub struct PackageChange {
    pub repository: String,
    pub distribution: String,
    pub component: String,

    pub action: PackageChangeAction,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum PackageChangeAction {
    Add {
        package_sha256sum: String,
    },
    Remove {
        name: String,
        version: String,
        architecture: String,
    },
}

#[derive(Debug)]
struct PackageChangeResult {
    release_file: ReleaseFile,
    changed_packages_index: PackagesIndex,
    changed_packages_index_contents: String,
    changed_package: PublishedPackage,
}

/// Given a single package change, generate the new release file and the changed
/// Packages index based off of the current state of the repository.
#[instrument(skip(tx))]
async fn generate_release_file_with_change(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &TenantID,
    change: &PackageChange,
    release_ts: OffsetDateTime,
) -> Result<PackageChangeResult, ErrorResponse> {
    // FIXME: I think there's a bug where if you generate an index by adding a
    // package that already exists, it gets added twice into the release file.

    // Load the package to be either added or removed. If it does not exist,
    // return an error.
    let changed_package = match &change.action {
        PackageChangeAction::Add { package_sha256sum } => {
            Package::query_from_sha256sum(&mut *tx, tenant_id, package_sha256sum).await
        }
        PackageChangeAction::Remove {
            name,
            version,
            architecture,
        } => Package::query_from_meta(&mut *tx, tenant_id, name, version, architecture).await,
    }
    .ok_or(ErrorResponse::new(
        StatusCode::NOT_FOUND,
        "PACKAGE_NOT_FOUND".to_string(),
        "package not found".to_string(),
    ))?;

    // Load the repository. If it does not exist, return an error.
    let repo = sqlx::query!(
        r#"
        SELECT id, name
        FROM debian_repository
        WHERE tenant_id = $1 AND name = $2
        "#,
        tenant_id.0,
        change.repository
    )
    .fetch_optional(&mut **tx)
    .await
    .unwrap()
    .ok_or(ErrorResponse::new(
        StatusCode::NOT_FOUND,
        "REPOSITORY_NOT_FOUND".to_string(),
        "repository not found".to_string(),
    ))?;

    // Load the release. Note that the release may not exist if no packages have
    // been added to this distribution.
    let release = sqlx::query!(
        r#"
        SELECT
            debian_repository_release.id,
            debian_repository_release.origin,
            debian_repository_release.label,
            debian_repository_release.version,
            debian_repository_release.suite,
            debian_repository_release.codename,
            debian_repository_release.description
        FROM debian_repository_release
        WHERE
            debian_repository_release.repository_id = $1
            AND debian_repository_release.distribution = $2
        LIMIT 1
        "#,
        repo.id,
        change.distribution,
    )
    .map(|row| {
        (
            row.id,
            Release {
                description: row.description,
                origin: row.origin,
                label: row.label,
                version: row.version,
                suite: row.suite,
                codename: row.codename,
            },
        )
    })
    .fetch_optional(&mut **tx)
    .await
    .unwrap();

    // Load the other packages in the changed `Packages` index, which is all
    // packages that have the same (release, component, architecture) as the
    // changed package.
    let packages = if let Some((release_id, _)) = release {
        sqlx::query!(r#"
            SELECT
                debian_repository_package.package,
                debian_repository_package.version,
                debian_repository_package.architecture::TEXT AS "architecture!: String",
                debian_repository_package.paragraph,
                debian_repository_package.size,
                debian_repository_package.s3_bucket,
                debian_repository_package.md5sum,
                debian_repository_package.sha1sum,
                debian_repository_package.sha256sum,
                debian_repository_component_package.filename
            FROM
                debian_repository_component
                JOIN debian_repository_component_package ON debian_repository_component_package.component_id = debian_repository_component.id
                JOIN debian_repository_package ON debian_repository_package.id = debian_repository_component_package.package_id
            WHERE
                debian_repository_component.release_id = $1
                AND debian_repository_component.name = $2
        "#, release_id, change.component)
        .map(|row| {
            PublishedPackage::from_package(
                Package {
                    name: row.package,
                    version: row.version,
                    architecture: row.architecture,
                    paragraph: row.paragraph,
                    size: row.size,
                    s3_bucket: row.s3_bucket,
                    md5sum: row.md5sum,
                    sha1sum: row.sha1sum,
                    sha256sum: row.sha256sum,
                },
                &change.component,
            )
        })
        .fetch_all(&mut **tx)
        .await
        .unwrap()
    } else {
        // If the release doesn't exist, there are can't possibly be any
        // existing packages in the release.
        Vec::new()
    };

    // Create the new package set.
    let (packages, changed_package) = match change.action {
        PackageChangeAction::Add { .. } => {
            let added = PublishedPackage::from_package(changed_package.clone(), &change.component);
            (
                packages
                    .into_iter()
                    .chain(once(added.clone()))
                    .collect::<Vec<_>>(),
                added,
            )
        }
        PackageChangeAction::Remove { .. } => {
            // Technically, I guess we should be querying for things like `Filename` instead of reconstructing them.
            let removed =
                PublishedPackage::from_package(changed_package.clone(), &change.component);
            (
                packages
                    .into_iter()
                    .filter(|p| {
                        !(p.package.name == changed_package.name
                            && p.package.version == changed_package.version
                            && p.package.architecture == changed_package.architecture)
                    })
                    .collect(),
                removed,
            )
        }
    };
    debug!(?packages, ?changed_package, "index package set");

    // Generate the `Packages` index for the `(distribution, component, arch)`
    // that is being changed.
    let changed_packages_index = PackagesIndex::from_packages(
        &change.component,
        &changed_package.package.architecture,
        packages.into_iter(),
    );
    debug!(?changed_packages_index, "changed packages index");

    // Generate the `Release` file for the distribution.
    let release_file = {
        // Load all other `Packages` indexes for the release.
        let packages_indexes = if let Some((release_id, _)) = release {
            sqlx::query_as!(PackagesIndex, r#"
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
                release_id,
            )
            .fetch_all(&mut **tx)
            .await
            .unwrap()
        } else {
            // If the distribution doesn't exist, it can't possibly have existing
            // `Packages` indexes.
            Vec::new()
        }.into_iter();

        // Remove the changed `Packages` index.
        let packages_indexes = packages_indexes.filter(|packages_index| {
            !(packages_index.component == change.component
                && packages_index.architecture == changed_package.package.architecture)
        });

        // Add the new `Packages` index if it's non-empty.
        let packages_indexes = if changed_packages_index.0.size == 0 {
            packages_indexes.collect::<Vec<_>>()
        } else {
            packages_indexes
                .chain(once(changed_packages_index.0.clone()))
                .collect::<Vec<_>>()
        };

        // When the Release is missing, we use one with default values instead.
        let release = match release {
            Some((_, release)) => release,
            None => Release {
                description: None,
                origin: None,
                label: None,
                version: None,
                suite: change.distribution.clone(),
                codename: change.distribution.clone(),
            },
        };

        ReleaseFile::from_indexes(release, release_ts, packages_indexes.into_iter())
    };

    Ok(PackageChangeResult {
        release_file,
        changed_packages_index: changed_packages_index.0,
        changed_packages_index_contents: changed_packages_index.1,
        changed_package,
    })
}

#[derive(FromRow, Clone, Debug)]
struct Package {
    #[sqlx(rename = "package")]
    name: String,
    version: String,
    architecture: String,

    paragraph: JsonValue,
    size: i64,

    s3_bucket: String,

    md5sum: String,
    sha1sum: String,
    sha256sum: String,
}

impl Package {
    fn pool_filename_in_component(&self, component: &str) -> String {
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
        let source_package_name = &self.name;
        let source_package_name_start = source_package_name.chars().next().unwrap();

        let binary_package_name = &self.name;
        let version = &self.version;
        let architecture = &self.architecture;
        format!(
            "pool/{component}/{source_package_name_start}/{source_package_name}/{binary_package_name}_{version}_{architecture}.deb"
        )
    }

    async fn query_from_meta<'a>(
        tx: &mut Transaction<'a, Postgres>,
        tenant_id: &TenantID,
        package: &str,
        version: &str,
        architecture: &str,
    ) -> Option<Self> {
        sqlx::query_as!(
            Self,
            r#"
                SELECT
                    package AS name,
                    version,
                    architecture::TEXT AS "architecture!: String",
                    paragraph,
                    size,
                    s3_bucket,
                    md5sum,
                    sha1sum,
                    sha256sum
                FROM debian_repository_package
                WHERE
                    tenant_id = $1
                    AND package = $2
                    AND version = $3
                    AND architecture = $4::debian_repository_architecture
            "#,
            tenant_id.0,
            package,
            version,
            architecture as _
        )
        .fetch_optional(&mut **tx)
        .await
        .unwrap()
    }

    async fn query_from_sha256sum<'a>(
        tx: &mut Transaction<'a, Postgres>,
        tenant_id: &TenantID,
        sha256sum: &str,
    ) -> Option<Self> {
        sqlx::query_as!(
            Self,
            r#"
                SELECT
                    package AS name,
                    version,
                    architecture::TEXT AS "architecture!: String",
                    paragraph,
                    size,
                    s3_bucket,
                    md5sum,
                    sha1sum,
                    sha256sum
                FROM debian_repository_package
                WHERE
                    tenant_id = $1
                    AND sha256sum = $2
            "#,
            tenant_id.0,
            sha256sum
        )
        .fetch_optional(&mut **tx)
        .await
        .unwrap()
    }
}

#[derive(FromRow, Clone, Debug)]
struct PublishedPackage {
    #[sqlx(flatten)]
    package: Package,

    filename: String,
}

impl PublishedPackage {
    fn from_package(package: Package, component: &str) -> Self {
        Self {
            filename: package.pool_filename_in_component(component),
            package,
        }
    }
}

#[derive(Clone, Debug)]
struct PackagesIndex {
    component: String,
    architecture: String,

    size: i64,

    md5sum: String,
    sha1sum: String,
    sha256sum: String,
}

impl PackagesIndex {
    fn from_packages(
        component: &str,
        architecture: &str,
        packages: impl Iterator<Item = PublishedPackage>,
    ) -> (Self, String) {
        let rendered = Self::packages_to_index(packages);
        (
            Self {
                component: component.to_string(),
                architecture: architecture.to_string(),
                size: rendered.len() as i64,
                md5sum: hex::encode(Md5::digest(&rendered)),
                sha1sum: hex::encode(Sha1::digest(&rendered)),
                sha256sum: hex::encode(Sha256::digest(&rendered)),
            },
            rendered,
        )
    }

    fn packages_to_index(packages: impl Iterator<Item = PublishedPackage>) -> String {
        let mut index = packages
            .map(|published| {
                let pkg = published.package;
                let fields = pkg
                    .paragraph
                    .as_object()
                    .unwrap()
                    .into_iter()
                    .map(|(k, v)| format!("{}: {}", k, v.as_str().unwrap()))
                    .chain(vec![
                        format!("Filename: {}", published.filename),
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
        if index.is_empty() {
            return String::new();
        }
        index.push('\n');
        index
    }
}

#[derive(FromRow, Debug)]
struct Release {
    description: Option<String>,
    origin: Option<String>,
    label: Option<String>,
    version: Option<String>,
    suite: String,
    codename: String,
}

#[derive(Debug)]
struct ReleaseFile {
    release: Release,
    contents: String,
}

impl ReleaseFile {
    fn from_indexes(
        release: Release,
        release_ts: OffsetDateTime,
        packages_indexes: impl Iterator<Item = PackagesIndex>,
    ) -> Self {
        // TODO: Worth it to make this streaming?
        let packages_indexes = packages_indexes.collect::<Vec<_>>();

        // Note that the date format is RFC 2822. _Technically_, the Debian spec
        // says it should be the date format of `date -R -u`, which technically
        // is RFC 5322, but these formats are compatible. 5322 is a later
        // revision of 2822 that retains backwards compatibility.
        let date = release_ts.format(&Rfc2822).unwrap();

        // Prepare "Architectures" and "Components" fields. We use BTreeSets
        // instead of HashSets to get deterministic iterator order, since index
        // generation needs to be deterministically replayed.
        let mut arch_set = BTreeSet::new();
        let mut comp_set = BTreeSet::new();
        for p in &packages_indexes {
            arch_set.insert(p.architecture.as_str());
            comp_set.insert(p.component.as_str());
        }
        let archs = arch_set
            .into_iter()
            .fold(String::new(), |acc_archs, arch| acc_archs + " " + arch);
        let archs = archs.strip_prefix(" ").unwrap_or("");
        let comps = comp_set
            .into_iter()
            .fold(String::new(), |acc_comps, comp| acc_comps + " " + comp);
        let comps = comps.strip_prefix(" ").unwrap_or("");

        // Write release fields.
        let release_fields: Vec<(&str, Option<String>)> = vec![
            ("Origin", release.origin.clone()),
            ("Label", release.label.clone()),
            ("Version", release.version.clone()),
            ("Suite", Some(release.suite.clone())),
            ("Codename", Some(release.codename.clone())),
            ("Date", Some(date)),
            ("Architectures", Some(archs.to_string())),
            ("Components", Some(comps.to_string())),
            ("Description", release.description.clone()),
            ("Acquire-By-Hash", Some(String::from("yes"))),
        ];
        let mut release_file = String::new();
        for (k, v) in release_fields {
            if let Some(v) = v {
                release_file.push_str(&format!("{k}: {v}\n"));
            }
        }

        // Write index fingerprints.
        release_file += "MD5Sum:\n";
        let mut md5writer = TabWriter::new(vec![])
            .alignment(Alignment::Right)
            .padding(1);
        for index in &packages_indexes {
            // TODO: Handle compressed indexes.
            writeln!(
                &mut md5writer,
                " {}\t{}\t{}/binary-{}/Packages",
                index.md5sum, index.size, index.component, index.architecture
            )
            .unwrap();
        }
        md5writer.flush().unwrap();
        release_file = release_file + &String::from_utf8(md5writer.into_inner().unwrap()).unwrap();

        release_file += "SHA256:\n";
        let mut sha256writer = TabWriter::new(vec![])
            .alignment(Alignment::Right)
            .padding(1);
        for index in &packages_indexes {
            // TODO: Handle compressed indexes.
            writeln!(
                &mut sha256writer,
                " {}\t{}\t{}/binary-{}/Packages",
                index.sha256sum, index.size, index.component, index.architecture
            )
            .unwrap();
        }
        sha256writer.flush().unwrap();

        release_file =
            release_file + &String::from_utf8(sha256writer.into_inner().unwrap()).unwrap();
        Self {
            release,
            contents: release_file,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packages_to_index_empty_when_no_packages() {
        assert_eq!(PackagesIndex::packages_to_index(vec![].into_iter()), "");
    }
}
