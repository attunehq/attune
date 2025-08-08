use std::iter::once;

use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use tracing::instrument;

use crate::{
    api::{ErrorResponse, TenantID},
    apt::{Package, PackagesIndex, PackagesIndexMeta, PublishedPackage, ReleaseFile, ReleaseMeta},
};

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
    changed_package: PublishedPackage,
    orphaned_pool_filename: bool,
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
    // Load the repository. If it does not exist, return an error.
    sqlx::query!(
        "SELECT id FROM debian_repository WHERE tenant_id = $1 AND name = $2",
        tenant_id.0,
        change.repository
    )
    .fetch_optional(&mut **tx)
    .await
    .unwrap()
    .ok_or(ErrorResponse::not_found("repository"))?;

    // Load the Release metadata. If the Release has never been created
    // before, use default values.
    let release = ReleaseMeta::query_from_release(
        &mut *tx,
        tenant_id,
        &change.repository,
        &change.distribution,
    )
    .await
    .unwrap_or(ReleaseMeta {
        description: None,
        origin: None,
        label: None,
        version: None,
        suite: change.distribution.clone(),
        codename: change.distribution.clone(),
    });

    // Load the package to be added. If it does not exist, return an error.
    let changed_package = match &change.action {
        PackageChangeAction::Add { package_sha256sum } => {
            let package = Package::query_from_sha256sum(&mut *tx, tenant_id, package_sha256sum)
                .await
                .ok_or(ErrorResponse::not_found("package"))?;
            PublishedPackage::from_package(package, &change.component)
        }
        PackageChangeAction::Remove {
            name,
            version,
            architecture,
        } => PublishedPackage::query_from_meta(
            &mut *tx,
            tenant_id,
            &change.repository,
            &change.distribution,
            &change.component,
            name,
            version,
            architecture,
        )
        .await
        .ok_or(ErrorResponse::not_found("package"))?,
    };

    // Load the Packages index that will be changed.
    //
    // Note that `packages_index_packages` might be empty if this is the first
    // package to be added to this (distribution, component, architecture)
    // tuple. But that's okay, because it will just end up constructing an empty
    // PackagesIndex.
    let packages_index_packages = PublishedPackage::query_from_packages_index(
        &mut *tx,
        tenant_id,
        &change.repository,
        &change.distribution,
        &change.component,
        &changed_package.package.architecture,
    )
    .await;
    let mut changed_packages_index = PackagesIndex::from_packages(
        &change.component,
        &changed_package.package.architecture,
        packages_index_packages,
    );

    // Modify the changed Packages index.
    match &change.action {
        PackageChangeAction::Add { .. } => {
            changed_packages_index.add_package(changed_package.package.clone());
        }
        PackageChangeAction::Remove { .. } => {
            changed_packages_index.remove_package(changed_package.clone());
        }
    }

    // Load all Packages indexes in the Release file.
    let packages_indexes = PackagesIndexMeta::query_from_release(
        &mut *tx,
        tenant_id,
        &change.repository,
        &change.distribution,
    )
    .await;

    // Update the set of Packages indexes in the Release file.
    let packages_indexes =
        update_release_package_indexes(packages_indexes, &changed_packages_index);

    // Construct the new Release file.
    let release_file = ReleaseFile::from_indexes(release, release_ts, &packages_indexes);

    // Determine whether there exist other component-packages with the same
    // filename. In the case of removals, this is used to clean up orphaned pool
    // files.
    //
    // Note that it is NOT sufficient to examine whether the resulting index has
    // been deleted, because each index is specific to (distribution, component,
    // architecture), and pool objects are shared between distributions!
    let remaining_component_packages = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!: i64"
        FROM
            debian_repository_package
            JOIN debian_repository_component_package ON debian_repository_package.id = debian_repository_component_package.package_id
        WHERE
            debian_repository_package.tenant_id = $1
            AND debian_repository_package.package = $2
            AND debian_repository_package.version = $3
            AND debian_repository_package.architecture = $4::debian_repository_architecture
            AND debian_repository_component_package.filename = $5
        "#,
        tenant_id.0,
        &changed_package.package.name,
        &changed_package.package.version,
        &changed_package.package.architecture as _,
        &changed_package.filename,
    )
    .fetch_one(&mut **tx)
    .await
    .unwrap();

    Ok(PackageChangeResult {
        release_file,
        changed_packages_index,
        changed_package,
        orphaned_pool_filename: remaining_component_packages.count == 0,
    })
}

// Update the set of `Packages` indexes in the Release file. This function is
// refactored out for purity so we can unit test it.
fn update_release_package_indexes(
    packages_indexes: Vec<PackagesIndexMeta>,
    changed_packages_index: &PackagesIndex,
) -> Vec<PackagesIndexMeta> {
    // TODO: Should we add assertions here for preconditions? For example, no
    // element in `packages_indexes` should be an index for the same component
    // and architecture as another, and none of them should be empty. We could
    // also do the same for post-conditions.

    // There are three cases to handle here:
    //
    // 1. If the index didn't previously exist, it should be added to the
    //    Release file.
    // 2. If the index previously existed, it should be updated in the Release
    //    file.
    // 3. If the index previously existed, but is now empty (i.e. this change
    //    removed all packages in it), it should be removed from the Release
    //    file.
    //
    // To do this, we first remove any existing Packages index for the same
    // component and architecture (notice that this is a no-op if the index
    // doesn't yet exist). Then, we add our new index if it's non-empty.
    let packages_indexes = packages_indexes.into_iter().filter(|pi| {
        !(pi.component == changed_packages_index.meta.component
            && pi.architecture == changed_packages_index.meta.architecture)
    });

    // Add the new `Packages` index if it's non-empty.
    if changed_packages_index.contents.is_empty() {
        packages_indexes.collect()
    } else {
        packages_indexes
            .chain(once(changed_packages_index.meta.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Packages with different architectures should be separated into their own
    /// indexes.
    ///
    /// This is a regression test for #105.
    #[sqlx::test(migrator = "crate::MIGRATOR", fixtures("setup_multi_arch"))]
    async fn packages_separated_by_architecture(pool: sqlx::PgPool) {
        let mut tx = pool.begin().await.unwrap();
        let tenant_id = crate::api::TenantID(1);
        let release_ts = OffsetDateTime::now_utc();

        let amd64_change = PackageChange {
            repository: String::from("test-multi-arch"),
            distribution: String::from("stable"),
            component: String::from("main"),
            action: PackageChangeAction::Add {
                package_sha256sum: String::from("amd64sha256sum"),
            },
        };
        let amd64_result =
            generate_release_file_with_change(&mut tx, &tenant_id, &amd64_change, release_ts)
                .await
                .expect("Failed to generate release file for amd64");
        assert!(
            amd64_result
                .changed_packages_index
                .contents
                .contains("Architecture: amd64"),
            "amd64 index should contain amd64 package"
        );
        assert!(
            !amd64_result
                .changed_packages_index
                .contents
                .contains("Architecture: arm64"),
            "amd64 index should NOT contain arm64 package"
        );
        assert_eq!(
            amd64_result.changed_packages_index.meta.architecture, "amd64",
            "Index should be for amd64 architecture"
        );

        let arm64_change = PackageChange {
            repository: String::from("test-multi-arch"),
            distribution: String::from("stable"),
            component: String::from("main"),
            action: PackageChangeAction::Add {
                package_sha256sum: String::from("arm64sha256sum"),
            },
        };
        let arm64_result =
            generate_release_file_with_change(&mut tx, &tenant_id, &arm64_change, release_ts)
                .await
                .expect("Failed to generate release file for arm64");
        assert!(
            arm64_result
                .changed_packages_index
                .contents
                .contains("Architecture: arm64"),
            "arm64 index should contain arm64 package"
        );
        assert!(
            !arm64_result
                .changed_packages_index
                .contents
                .contains("Architecture: amd64"),
            "arm64 index should NOT contain amd64 package"
        );
        assert_eq!(
            arm64_result.changed_packages_index.meta.architecture, "arm64",
            "Index should be for arm64 architecture"
        );

        tx.rollback().await.unwrap();
    }

    /// Removing all packages from an architecture results in an empty index.
    #[sqlx::test(migrator = "crate::MIGRATOR", fixtures("setup_multi_arch"))]
    async fn remove_all_packages_for_architecture(pool: sqlx::PgPool) {
        let mut tx = pool.begin().await.unwrap();
        let tenant_id = crate::api::TenantID(1);
        let release_ts = OffsetDateTime::now_utc();

        let remove_amd64_change = PackageChange {
            repository: String::from("test-multi-arch"),
            distribution: String::from("stable"),
            component: String::from("main"),
            action: PackageChangeAction::Remove {
                name: String::from("test-package"),
                version: String::from("1.0.0"),
                architecture: String::from("amd64"),
            },
        };
        let remove_result = generate_release_file_with_change(
            &mut tx,
            &tenant_id,
            &remove_amd64_change,
            release_ts,
        )
        .await
        .expect("Failed to generate release file for removal");
        assert!(
            remove_result.changed_packages_index.contents.is_empty(),
            "amd64 index should be empty after removing all amd64 packages"
        );
        assert_eq!(
            remove_result.changed_packages_index.meta.architecture, "amd64",
            "Index should still be for amd64 architecture"
        );
        assert_eq!(
            remove_result.changed_packages_index.meta.size, 0,
            "Index size should be 0"
        );

        // Verify that arm64 packages are unaffected by checking the release file
        // The release file should still list the arm64 index
        assert!(
            remove_result.release_file.contents.contains("arm64"),
            "Release file should still reference arm64 architecture"
        );

        tx.rollback().await.unwrap();
    }

    /// The release file should list all architecture indexes.
    #[sqlx::test(migrator = "crate::MIGRATOR", fixtures("setup_multi_arch"))]
    async fn release_file_lists_all_architectures(pool: sqlx::PgPool) {
        let mut tx = pool.begin().await.unwrap();
        let tenant_id = crate::api::TenantID(1);
        let release_ts = OffsetDateTime::now_utc();

        // Make a change to trigger release file generation
        let change = PackageChange {
            repository: String::from("test-multi-arch"),
            distribution: String::from("stable"),
            component: String::from("main"),
            action: PackageChangeAction::Add {
                package_sha256sum: String::from("amd64sha256sum"),
            },
        };
        let result = generate_release_file_with_change(&mut tx, &tenant_id, &change, release_ts)
            .await
            .expect("Failed to generate release file");
        assert!(
            result
                .release_file
                .contents
                .contains("Architectures: amd64 arm64"),
            "Release file should list both architectures"
        );
        assert!(
            result
                .release_file
                .contents
                .contains("main/binary-amd64/Packages"),
            "Release file should reference amd64 Packages index"
        );
        assert!(
            result
                .release_file
                .contents
                .contains("main/binary-arm64/Packages"),
            "Release file should reference arm64 Packages index"
        );

        tx.rollback().await.unwrap();
    }
}
