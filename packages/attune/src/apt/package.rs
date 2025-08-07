use sqlx::{FromRow, Postgres, Transaction, types::JsonValue};

use crate::api::TenantID;

#[derive(FromRow, Clone, Debug)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub architecture: String,

    pub paragraph: JsonValue,
    pub size: i64,

    pub s3_bucket: String,

    pub md5sum: String,
    pub sha1sum: String,
    pub sha256sum: String,
}

impl Package {
    pub async fn query_from_meta<'a>(
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

    pub async fn query_from_sha256sum<'a>(
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

    pub fn pool_filename_in_component(&self, component: &str) -> String {
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
}

/// This newtype wraps Package for use cases (e.g. sets) where you want Packages
/// to have equality by their (name, version, architecture) fields.
#[derive(Clone, Debug)]
pub struct PackageByMeta(pub Package);

fn package_eq_by_meta(a: &Package, b: &Package) -> bool {
    a.name == b.name && a.version == b.version && a.architecture == b.architecture
}

impl Eq for PackageByMeta {}

impl PartialEq for PackageByMeta {
    fn eq(&self, other: &Self) -> bool {
        package_eq_by_meta(&self.0, &other.0)
    }
}

#[derive(FromRow, Clone, Debug)]
pub struct PublishedPackage {
    #[sqlx(flatten)]
    pub package: Package,
    pub filename: String,
}

impl PublishedPackage {
    pub fn from_package(package: Package, component: &str) -> Self {
        Self {
            filename: package.pool_filename_in_component(component),
            package,
        }
    }
    pub async fn query_from_meta<'a>(
        tx: &mut Transaction<'a, Postgres>,
        tenant_id: &TenantID,
        repository: &str,
        release: &str,
        component: &str,
        package: &str,
        version: &str,
        architecture: &str,
    ) -> Option<Self> {
        // Note that we don't use `query_as!` here because the macros (which
        // have compile-time query checking) don't actually work with `FromRow`
        // instances and annotations like `flatten`. Otherwise, we would be able
        // to skip the `.map` after the query. For details, see:
        //
        // - https://github.com/launchbadge/sqlx/issues/2188
        // - https://github.com/launchbadge/sqlx/issues/514
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
                debian_repository
                JOIN debian_repository_release ON debian_repository_release.repository_id = debian_repository.id
                JOIN debian_repository_component ON debian_repository_component.release_id = debian_repository_release.id
                JOIN debian_repository_component_package ON debian_repository_component_package.component_id = debian_repository_component.id
                JOIN debian_repository_package ON debian_repository_package.id = debian_repository_component_package.package_id
            WHERE
                debian_repository.tenant_id = $1
                AND debian_repository.name = $2
                AND debian_repository_release.distribution = $3
                AND debian_repository_component.name = $4
                AND debian_repository_package.package = $5
                AND debian_repository_package.version = $6
                AND debian_repository_package.architecture = $7::debian_repository_architecture
        "#,
            tenant_id.0,
            repository,
            release,
            component,
            package,
            version,
            architecture as _
        )
        .fetch_optional(&mut **tx)
        .await
        .unwrap()
        .map(|row| {
            PublishedPackage {
                package: Package {
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
                filename: row.filename,
            }
        })
    }

    pub async fn query_from_packages_index<'a>(
        tx: &mut Transaction<'a, Postgres>,
        tenant_id: &TenantID,
        repository: &str,
        release: &str,
        component: &str,
        architecture: &str,
    ) -> Vec<Self> {
        // Note that we don't use `query_as!` here because the macros (which
        // have compile-time query checking) don't actually work with `FromRow`
        // instances and annotations like `flatten`. Otherwise, we would be able
        // to skip the `.map` after the query. For details, see:
        //
        // - https://github.com/launchbadge/sqlx/issues/2188
        // - https://github.com/launchbadge/sqlx/issues/514
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
                debian_repository
                JOIN debian_repository_release ON debian_repository_release.repository_id = debian_repository.id
                JOIN debian_repository_component ON debian_repository_component.release_id = debian_repository_release.id
                JOIN debian_repository_component_package ON debian_repository_component_package.component_id = debian_repository_component.id
                JOIN debian_repository_package ON debian_repository_package.id = debian_repository_component_package.package_id
            WHERE
                debian_repository.tenant_id = $1
                AND debian_repository.name = $2
                AND debian_repository_release.distribution = $3
                AND debian_repository_component.name = $4
                AND debian_repository_package.architecture = $5::debian_repository_architecture
        "#, tenant_id.0, repository, release, component, architecture as _)
        .map(|row| {
            PublishedPackage {
                package: Package {
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
                filename: row.filename,
            }
        })
        .fetch_all(&mut **tx)
        .await
        .unwrap()
    }
}

#[derive(Clone, Debug)]
pub struct PublishedPackageByMeta(pub PublishedPackage);

impl Eq for PublishedPackageByMeta {}

impl PartialEq for PublishedPackageByMeta {
    fn eq(&self, other: &Self) -> bool {
        package_eq_by_meta(&self.0.package, &other.0.package)
    }
}
