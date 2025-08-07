use md5::Md5;
use sha1::Sha1;
use sha2::{Digest as _, Sha256};
use sqlx::{FromRow, Postgres, Transaction};

use crate::{
    api::TenantID,
    apt::{Package, PublishedPackage},
};

#[derive(Clone, Debug, FromRow)]
pub struct PackagesIndexMeta {
    pub component: String,
    pub architecture: String,

    pub size: i64,

    pub md5sum: String,
    pub sha1sum: String,
    pub sha256sum: String,
}

impl PackagesIndexMeta {
    pub async fn query_from_release<'a>(
        tx: &mut Transaction<'a, Postgres>,
        tenant_id: &TenantID,
        repository: &str,
        release: &str,
    ) -> Vec<Self> {
        sqlx::query_as!(PackagesIndexMeta, r#"
            SELECT
                debian_repository_component.name AS component,
                debian_repository_index_packages.architecture::TEXT AS "architecture!: String",
                debian_repository_index_packages.size,
                debian_repository_index_packages.md5sum,
                debian_repository_index_packages.sha1sum,
                debian_repository_index_packages.sha256sum
            FROM
                debian_repository
                JOIN debian_repository_release ON debian_repository_release.repository_id = debian_repository.id
                JOIN debian_repository_component ON debian_repository_component.release_id = debian_repository_release.id
                JOIN debian_repository_index_packages ON debian_repository_index_packages.component_id = debian_repository_component.id
            WHERE
                debian_repository.tenant_id = $1
                AND debian_repository.name = $2
                AND debian_repository_release.distribution = $3
            "#,
            tenant_id.0,
            repository,
            release,
        )
        .fetch_all(&mut **tx)
        .await
        .unwrap()
    }
}

#[derive(Clone, Debug, FromRow)]
pub struct PackagesIndex {
    #[sqlx(flatten)]
    pub meta: PackagesIndexMeta,
    pub contents: String,
    packages: Vec<PublishedPackage>,
}

impl PackagesIndex {
    pub fn from_packages(
        component: &str,
        architecture: &str,
        packages: Vec<PublishedPackage>,
    ) -> Self {
        let rendered = Self::render(packages.iter());
        Self {
            meta: PackagesIndexMeta {
                component: component.to_string(),
                architecture: architecture.to_string(),
                size: rendered.len() as i64,
                md5sum: hex::encode(Md5::digest(&rendered)),
                sha1sum: hex::encode(Sha1::digest(&rendered)),
                sha256sum: hex::encode(Sha256::digest(&rendered)),
            },
            packages,
            contents: rendered,
        }
    }

    fn render<'a>(packages: impl Iterator<Item = &'a PublishedPackage>) -> String {
        // TODO: A better way to ensure index reproducibility is to sort the
        // packages before we render them.
        let mut index = packages
            .map(|published| {
                let pkg = &published.package;
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

    /// Add a package to this Packages index. This will re-render the index,
    /// updating the size, checksums, and contents.
    ///
    /// If the package is already present in the index, this is a no-op.
    pub fn add_package(&mut self, added: Package) {
        // TODO: What if these fields are the same, but other fields (e.g. the
        // package hashes) are different? Should we crash? Should we push that
        // invariant checking outwards?
        if self.packages.iter().any(|p| {
            p.package.name == added.name
                && p.package.version == added.version
                && p.package.architecture == added.architecture
        }) {
            return;
        }
        self.packages
            .push(PublishedPackage::from_package(added, &self.meta.component));
        self.rerender();
    }

    /// Remove a package from this Packages index. This will re-render the
    /// index, updating the size, checksums, and contents.
    ///
    /// If the package is not present in the index, this is a no-op.
    pub fn remove_package(&mut self, removed: PublishedPackage) {
        self.packages.retain(|p| {
            !(p.package.name == removed.package.name
                && p.package.version == removed.package.version
                && p.package.architecture == removed.package.architecture)
        });
        self.rerender();
    }

    /// Re-render the index, updating the size, checksums, and contents.
    fn rerender(&mut self) {
        let rendered = Self::render(self.packages.iter());
        self.meta.size = rendered.len() as i64;
        self.meta.md5sum = hex::encode(Md5::digest(&rendered));
        self.meta.sha1sum = hex::encode(Sha1::digest(&rendered));
        self.meta.sha256sum = hex::encode(Sha256::digest(&rendered));
        self.contents = rendered;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generating a Packages index that contains zero packages is guaranteed to
    /// produce the empty string.
    #[test]
    fn empty_when_no_packages() {
        assert_eq!(PackagesIndex::render(vec![].into_iter()), "");
    }

    /// Rendering the same set of packages multiple times should produce the
    /// same result.
    #[test]
    fn deterministic_render() {
        let packages = (1..10)
            .map(|i| {
                PublishedPackage::from_package(
                    Package {
                        name: format!("foo_{}", i),
                        version: String::from("1.0.0"),
                        architecture: String::from("amd64"),
                        paragraph: serde_json::Value::Object(serde_json::Map::new()),
                        size: 0,
                        s3_bucket: format!("fake_bucket_{}", i),
                        md5sum: format!("fake_md5sum_{}", i),
                        sha1sum: format!("fake_sha1sum_{}", i),
                        sha256sum: format!("fake_sha256sum_{}", i),
                    },
                    "fake_component",
                )
            })
            .collect::<Vec<PublishedPackage>>();
        let first = PackagesIndex::from_packages("main", "amd64", packages.clone()).contents;
        let second = PackagesIndex::from_packages("main", "amd64", packages).contents;
        assert_eq!(first, second);
    }

    /// Adding a package that is already in the index is a no-op.
    #[test]
    fn idempotent_when_add_existing() {
        let package = Package {
            name: String::from("foo"),
            version: String::from("1.0.0"),
            architecture: String::from("amd64"),
            paragraph: serde_json::Value::Object(serde_json::Map::new()),
            size: 0,
            s3_bucket: String::from("fake_bucket"),
            md5sum: String::from("fake_md5sum"),
            sha1sum: String::from("fake_sha1sum"),
            sha256sum: String::from("fake_sha256sum"),
        };
        let published = PublishedPackage::from_package(package.clone(), "fake_component");
        let mut index = PackagesIndex::from_packages("main", "amd64", vec![published]);
        let before = index.contents.clone();
        index.add_package(package);
        let after = index.contents.clone();
        assert_eq!(before, after);
    }

    // TODO: `debian_packaging::repository::ReleaseReader` provides a parser for
    // Packages indexes via `ControlParagraphReader` and
    // `BinaryPackageControlFile::from`. We can use that to create a
    // property-based test to check that our renderer is correct. See also:
    // https://docs.rs/debian-packaging/0.18.0/src/debian_packaging/repository/mod.rs.html#468-497
}
