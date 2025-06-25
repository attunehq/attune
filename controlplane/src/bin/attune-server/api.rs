use std::{
    collections::{HashMap, HashSet},
    io::Write,
};

use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
};
use debian_packaging::deb::reader::{BinaryPackageEntry, BinaryPackageReader, ControlTarFile};
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use sqlx::types::{JsonValue, time::OffsetDateTime};
use tabwriter::TabWriter;
use time::format_description::well_known::Rfc2822;
use tracing::{Instrument, instrument, debug_span};

#[derive(Clone, Debug)]
pub struct ServerState {
    pub db: sqlx::PgPool,
    pub s3: aws_sdk_s3::Client,

    pub s3_bucket_name: String,
    pub tenant_mode: TenantMode,
}

#[derive(Debug, Clone)]
pub enum TenantMode {
    Single,
    Multi,
}

#[derive(Serialize)]
pub struct Repository {
    id: i64,
    uri: String,
    distribution: String,
}

#[derive(Deserialize, Debug)]
pub struct CreateRepositoryRequest {
    uri: String,
    distribution: String,
    origin: Option<String>,
    label: Option<String>,
    version: Option<String>,
    suite: Option<String>,
    codename: Option<String>,
    description: Option<String>,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn create_repository(
    State(state): State<ServerState>,
    Json(payload): Json<CreateRepositoryRequest>,
) -> Json<Repository> {
    let s3_prefix = match state.tenant_mode {
        TenantMode::Single => "".to_string(),
        TenantMode::Multi => hex::encode(Sha256::digest(&payload.uri)),
    };
    let created = sqlx::query!(
        r#"
        INSERT INTO debian_repository (
            uri,
            distribution,
            origin,
            label,
            version,
            suite,
            codename,
            description,
            s3_bucket,
            s3_prefix
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING id
        "#,
        payload.uri,
        payload.distribution,
        payload.origin,
        payload.label,
        payload.version,
        payload.suite,
        payload.codename,
        payload.description,
        state.s3_bucket_name,
        s3_prefix,
    )
    .fetch_one(&state.db)
    .await
    .unwrap();
    Json(Repository {
        id: created.id,
        uri: payload.uri,
        distribution: payload.distribution,
    })
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn list_repositories(State(state): State<ServerState>) -> Json<Vec<Repository>> {
    let repositories = sqlx::query!("SELECT id, uri, distribution FROM debian_repository")
        .fetch_all(&state.db)
        .await
        .unwrap();
    Json(
        repositories
            .into_iter()
            .map(|r| Repository {
                id: r.id,
                uri: r.uri,
                distribution: r.distribution,
            })
            .collect(),
    )
}

#[derive(Debug, Serialize)]
pub struct RepositoryStatus {
    changes: Vec<RepositoryChange>,
}

#[derive(Debug, Serialize)]
pub struct RepositoryChange {
    package_id: i64,
    component: String,
    package: String,
    version: String,
    architecture: String,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
    change: String,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn repository_status(
    State(state): State<ServerState>,
    Path(repository_id): Path<u64>,
) -> Json<RepositoryStatus> {
    let changes = sqlx::query!(
        r#"
        SELECT
            debian_repository_package.id,
            debian_repository_package.repository_id,
            debian_repository_component.name AS component,
            debian_repository_package.package,
            debian_repository_package.version,
            debian_repository_architecture.name AS architecture,
            debian_repository_package.staging_status::TEXT AS "staging_status!: String",
            debian_repository_package.updated_at
        FROM debian_repository_package
        JOIN debian_repository_architecture ON debian_repository_architecture.id = debian_repository_package.architecture_id
        JOIN debian_repository_component ON debian_repository_component.id = debian_repository_package.component_id
        WHERE
            staging_status IS NOT NULL
            AND debian_repository_package.repository_id = $1
        "#,
        repository_id as i64,
    )
    .map(|row| RepositoryChange {
        package_id: row.id,
        component: row.component,
        package: row.package,
        version: row.version,
        architecture: row.architecture,
        updated_at: row.updated_at,
        change: row.staging_status
    })
    .fetch_all(&state.db)
    .await
    .unwrap();
    Json(RepositoryStatus { changes })
}

#[derive(Debug, Serialize)]
pub struct RepositoryIndexes {
    release: String,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn generate_indexes(
    State(state): State<ServerState>,
    Path(repository_id): Path<u64>,
) -> Json<RepositoryIndexes> {
    // TODO: Do some optimization to check for staleness to avoid regenerating
    // indexes every call if packages haven't changed.

    let mut tx = state.db.begin().await.unwrap();
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();

    let repo = sqlx::query!(
        r#"
        SELECT
            distribution,
            origin,
            label,
            version,
            suite,
            codename,
            description,
            s3_bucket,
            s3_prefix
        FROM debian_repository
        WHERE id = $1
        "#,
        repository_id as i64
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    // Generate package indexes, save to database, and upload to staging.
    let package_indexes = sqlx::query!(r#"
        SELECT DISTINCT
            debian_repository_package.component_id,
            debian_repository_package.architecture_id,
            debian_repository_component.name AS component,
            debian_repository_architecture.name AS architecture
        FROM debian_repository_package
        JOIN debian_repository_architecture ON debian_repository_architecture.id = debian_repository_package.architecture_id
        JOIN debian_repository_component ON debian_repository_component.id = debian_repository_package.component_id
        WHERE debian_repository_package.repository_id = $1
    "#, repository_id as i64
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    for package_index in &package_indexes {
        // For each package index file, generate the index.
        let pkgs = sqlx::query!(
            r#"
            SELECT
                paragraph,
                filename,
                size,
                md5sum,
                sha1sum,
                sha256sum
            FROM debian_repository_package
            WHERE
                repository_id = $1
                AND component_id = $2
                AND architecture_id = $3
        "#,
            repository_id as i64,
            package_index.component_id,
            package_index.architecture_id
        )
        .fetch_all(&mut *tx)
        .await
        .unwrap();

        let index = pkgs.into_iter().fold(String::new(), |acc_index, mut pkg| {
            let fields = pkg
                .paragraph
                .as_object_mut()
                .unwrap()
                .into_iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                .chain(
                    vec![
                        ("Filename".to_string(), pkg.filename),
                        ("Size".to_string(), pkg.size.to_string()),
                        ("MD5sum".to_string(), pkg.md5sum),
                        ("SHA1".to_string(), pkg.sha1sum),
                        ("SHA256".to_string(), pkg.sha256sum),
                    ]
                    .into_iter(),
                )
                .fold(String::new(), |acc_fields, (k, v)| {
                    acc_fields + &k + ": " + &v + "\n"
                });
            acc_index + &fields + "\n"
        });

        // Compute hashes.
        let sha256sum = hex::encode(Sha256::digest(&index));
        let sha1sum = hex::encode(Sha1::digest(&index));
        let md5sum = hex::encode(Md5::digest(&index));
        let size = index.len() as i64;

        // Save index to database.
        sqlx::query!(
            r#"
            INSERT INTO debian_repository_index_packages (
                repository_id,
                component_id,
                architecture_id,
                compression,
                size,
                contents,
                md5sum,
                sha1sum,
                sha256sum
            ) VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, $8)
            ON CONFLICT (repository_id, component_id, architecture_id)
            DO UPDATE SET
                repository_id = $1,
                component_id = $2,
                architecture_id = $3,
                compression = NULL,
                size = $4,
                contents = $5,
                md5sum = $6,
                sha1sum = $7,
                sha256sum = $8
            "#,
            repository_id as i64,
            package_index.component_id,
            package_index.architecture_id,
            size,
            index.as_bytes(),
            md5sum,
            sha1sum,
            sha256sum
        )
        .execute(&mut *tx)
        .await
        .unwrap();

        // Upload index to staging.
        let key = format!(
            "staging/dists/{}/{}/binary-{}/Packages",
            repo.distribution, package_index.component, package_index.architecture
        );
        state
            .s3
            .put_object()
            .bucket(&repo.s3_bucket)
            .key(if repo.s3_prefix.is_empty() {
                key
            } else {
                format!("{}/{}", repo.s3_prefix, key)
            })
            .body(axum::body::Bytes::from_owner(index).into())
            .send()
            .await
            .unwrap();
    }

    // Generate release index, save to database, and upload to staging.
    //
    // Note that the date format is RFC 2822. _Technically_, the Debian spec
    // says it should be the date format of `date -R -u`, which technically is
    // RFC 5322, but these formats are compatible. 5322 is a later revision of
    // 2822 that retains backwards compatibility.
    let date = OffsetDateTime::now_utc().format(&Rfc2822).unwrap();
    let mut arch_set = HashSet::new();
    let mut comp_set = HashSet::new();
    for p in package_indexes {
        arch_set.insert(p.architecture);
        comp_set.insert(p.component);
    }
    let archs = arch_set
        .into_iter()
        .fold(String::new(), |acc_archs, arch| acc_archs + " " + &arch);
    let comps = comp_set
        .into_iter()
        .fold(String::new(), |acc_comps, comp| acc_comps + " " + &comp);
    let release_fields: Vec<(&str, Option<String>)> = vec![
        ("Origin", repo.origin.clone()),
        ("Label", repo.label.clone()),
        ("Version", repo.version.clone()),
        ("Suite", Some(repo.suite.clone())),
        ("Codename", Some(repo.codename.clone())),
        ("Date", Some(date)),
        ("Architectures", Some(archs)),
        ("Components", Some(comps)),
        ("Description", repo.description.clone()),
    ];
    let mut release_index = String::new();
    for (k, v) in release_fields {
        if let Some(v) = v {
            release_index.push_str(&format!("{}: {}\n", k, v));
        }
    }

    let indexes = sqlx::query!(r#"
        SELECT
            debian_repository_component.name AS component,
            debian_repository_architecture.name AS architecture,
            debian_repository_index_packages.size,
            debian_repository_index_packages.md5sum,
            debian_repository_index_packages.sha1sum,
            debian_repository_index_packages.sha256sum
        FROM debian_repository_index_packages
        JOIN debian_repository_architecture ON debian_repository_architecture.id = debian_repository_index_packages.architecture_id
        JOIN debian_repository_component ON debian_repository_component.id = debian_repository_index_packages.component_id
        WHERE
            debian_repository_index_packages.compression IS NULL
            AND debian_repository_index_packages.repository_id = $1
        "#,
        repository_id as i64
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();

    release_index = release_index + "MD5Sum:\n";
    let mut md5writer = TabWriter::new(vec![]);
    for index in &indexes {
        // TODO: Handle compressed indexes.
        write!(
            &mut md5writer,
            " {}\t{} {}\n",
            index.md5sum,
            index.size,
            format!("{}/binary-{}/Packages", index.component, index.architecture)
        )
        .unwrap();
    }
    md5writer.flush().unwrap();
    release_index = release_index + &String::from_utf8(md5writer.into_inner().unwrap()).unwrap();

    release_index = release_index + "SHA256:\n";
    let mut sha256writer = TabWriter::new(vec![]);
    for index in &indexes {
        // TODO: Handle compressed indexes.
        write!(
            &mut sha256writer,
            " {}\t{} {}\n",
            index.sha256sum,
            index.size,
            format!("{}/binary-{}/Packages", index.component, index.architecture)
        )
        .unwrap();
    }
    sha256writer.flush().unwrap();
    release_index = release_index + &String::from_utf8(sha256writer.into_inner().unwrap()).unwrap();

    // Save index to database.
    sqlx::query!(
        r#"
        INSERT INTO debian_repository_index_release (
            repository_id,
            origin,
            label,
            version,
            suite,
            codename,
            description,
            contents
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (repository_id)
        DO UPDATE SET
            repository_id = $1,
            origin = $2,
            label = $3,
            version = $4,
            suite = $5,
            codename = $6,
            description = $7,
            contents = $8
        "#,
        repository_id as i64,
        repo.origin,
        repo.label,
        repo.version,
        repo.suite,
        repo.codename,
        repo.description,
        release_index
    )
    .execute(&mut *tx)
    .await
    .unwrap();

    // Upload index to staging.
    let key = format!("staging/dists/{}/Release", repo.distribution);

    state
        .s3
        .put_object()
        .bucket(&repo.s3_bucket)
        .key(if repo.s3_prefix.is_empty() {
            key
        } else {
            format!("{}/{}", repo.s3_prefix, key)
        })
        .body(axum::body::Bytes::from_owner(release_index.clone()).into())
        .send()
        .await
        .unwrap();

    tx.commit().await.unwrap();

    // Return generated release index.
    Json(RepositoryIndexes {
        release: release_index,
    })
}

#[derive(Debug, Deserialize)]
pub struct SyncRepositoryRequest {
    clearsigned: String,
    detached: String,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn sync_repository(
    State(state): State<ServerState>,
    Path(repository_id): Path<u64>,
    Json(payload): Json<SyncRepositoryRequest>,
) -> () {
    // TODO: Add locking to make sure this can't happen simultaneously.

    // TODO: Check that signatures and checksums are actually valid and
    // up-to-date?

    // Save signatures to database.
    let mut tx = state.db.begin().await.unwrap();
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let repo = sqlx::query!(
        "SELECT distribution, s3_bucket, s3_prefix FROM debian_repository WHERE id = $1",
        repository_id as i64
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    let release_index = sqlx::query!(
        r#"
        UPDATE debian_repository_index_release
        SET clearsigned = $1, detached = $2
        WHERE repository_id = $3
        RETURNING contents
    "#,
        payload.clearsigned,
        payload.detached,
        repository_id as i64,
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    // Copy new package files from staging to active.
    let added_packages = sqlx::query!(
        r#"
        SELECT filename
        FROM debian_repository_package
        WHERE
            staging_status = 'add'
            AND repository_id = $1
        "#,
        repository_id as i64
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    for added in added_packages {
        let source_key = if repo.s3_prefix.is_empty() {
            format!("{}/staging/{}", repo.s3_bucket, added.filename)
        } else {
            format!(
                "{}/{}/staging/{}",
                repo.s3_bucket, repo.s3_prefix, added.filename
            )
        };
        state
            .s3
            .copy_object()
            .copy_source(source_key)
            .bucket(&repo.s3_bucket)
            .key(if repo.s3_prefix.is_empty() {
                added.filename
            } else {
                format!("{}/{}", repo.s3_prefix, added.filename)
            })
            .send()
            .await
            .unwrap();
    }

    // Copy package indexes from staging to active.
    let package_indexes = sqlx::query!(
        r#"
        SELECT
            debian_repository_component.name AS component,
            debian_repository_architecture.name AS architecture
        FROM debian_repository_index_packages
        JOIN debian_repository_architecture ON debian_repository_architecture.id = debian_repository_index_packages.architecture_id
        JOIN debian_repository_component ON debian_repository_component.id = debian_repository_index_packages.component_id
        WHERE
            debian_repository_index_packages.compression IS NULL
            AND debian_repository_index_packages.repository_id = $1
        "#,
        repository_id as i64
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    for index in package_indexes {
        let source_key = if repo.s3_prefix.is_empty() {
            format!(
                "{}/staging/dists/{}/{}/binary-{}/Packages",
                repo.s3_bucket, repo.distribution, index.component, index.architecture
            )
        } else {
            format!(
                "{}/{}/staging/dists/{}/{}/binary-{}/Packages",
                repo.s3_bucket,
                repo.s3_prefix,
                repo.distribution,
                index.component,
                index.architecture
            )
        };
        let key = format!(
            "dists/{}/{}/binary-{}/Packages",
            repo.distribution, index.component, index.architecture
        );
        state
            .s3
            .copy_object()
            .copy_source(source_key)
            .bucket(&repo.s3_bucket)
            .key(if repo.s3_prefix.is_empty() {
                key
            } else {
                format!("{}/{}", repo.s3_prefix, key)
            })
            .send()
            .await
            .unwrap();
    }

    // Save release indexes.
    let key = format!("dists/{}/Release", repo.distribution);
    state
        .s3
        .put_object()
        .bucket(&repo.s3_bucket)
        .key(if repo.s3_prefix.is_empty() {
            key
        } else {
            format!("{}/{}", repo.s3_prefix, key)
        })
        .body(axum::body::Bytes::from_owner(release_index.contents).into())
        .send()
        .await
        .unwrap();

    let key = format!("dists/{}/Release.gpg", repo.distribution);
    state
        .s3
        .put_object()
        .bucket(&repo.s3_bucket)
        .key(if repo.s3_prefix.is_empty() {
            key
        } else {
            format!("{}/{}", repo.s3_prefix, key)
        })
        .body(axum::body::Bytes::from_owner(payload.detached).into())
        .send()
        .await
        .unwrap();

    let key = format!("dists/{}/InRelease", repo.distribution);
    state
        .s3
        .put_object()
        .bucket(&repo.s3_bucket)
        .key(if repo.s3_prefix.is_empty() {
            key
        } else {
            format!("{}/{}", repo.s3_prefix, key)
        })
        .body(axum::body::Bytes::from_owner(payload.clearsigned).into())
        .send()
        .await
        .unwrap();

    // TODO: Delete removed package files from active.

    // Update staging statuses for all packages.
    sqlx::query!(
        "UPDATE debian_repository_package SET staging_status = NULL WHERE repository_id = $1",
        repository_id as i64
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // TODO: Clean up staging.

    return;
}

#[derive(Deserialize, Serialize)]
pub struct Package {
    id: i64,
    package: String,
    version: String,
    architecture: String,
}

#[axum::debug_handler]
#[instrument(skip(state, multipart))]
pub async fn add_package(
    State(state): State<ServerState>,
    Path(repository_id): Path<u64>,
    Query(params): Query<HashMap<String, String>>,
    mut multipart: Multipart,
) -> Json<Package> {
    // Parse request.
    let component = params.get("component").unwrap().to_string();

    // Parse uploaded body file.
    let field = multipart
        .next_field()
        .await
        .unwrap()
        .expect("expected a file");
    let name = field.name().unwrap().to_string();
    if name != "file" {
        panic!("unexpected field name: expected \"file\", got {}", name);
    }

    // TODO: Is there a way to implement this function body with streaming
    // instead of holding the whole file in memory?

    // Parse Debian package for control fields.
    let value = field.bytes().await.unwrap();
    let mut reader = BinaryPackageReader::new(value.as_ref()).unwrap();
    let control_file = debug_span!("parse_debian_package").in_scope(|| {
        let header_entry = reader.next_entry().unwrap().unwrap();
        let BinaryPackageEntry::DebianBinary(_) = header_entry else {
            panic!("expected a Debian binary package")
        };
        let control_entry = reader.next_entry().unwrap().unwrap();
        let BinaryPackageEntry::Control(mut control_reader) = control_entry else {
            panic!("expected a control file")
        };
        let mut control_entries = control_reader.entries().unwrap();
        let control_file = loop {
            let (_, control_tar_file) = control_entries
                .next()
                .unwrap()
                .unwrap()
                .to_control_file()
                .unwrap();
            match control_tar_file {
                ControlTarFile::Control(control_file) => {
                    break control_file;
                }
                _ => {}
            }
        };
        // TODO: Parse file paths for building Contents index.
        let data_entry = reader.next_entry().unwrap().unwrap();
        let BinaryPackageEntry::Data(_) = data_entry else {
            panic!("expected a data file")
        };
        control_file
    });
    let architecture = control_file.architecture().unwrap();

    // Compute hashes.
    //
    // TODO: Can we make this faster?
    let (sha256sum, sha1sum, md5sum) = debug_span!("compute_hashes").in_scope(|| {
        let sha256sum = debug_span!("sha256sum").in_scope(|| hex::encode(Sha256::digest(&value)));
        let sha1sum = debug_span!("sha1sum").in_scope(|| hex::encode(Sha1::digest(&value)));
        let md5sum = debug_span!("md5sum").in_scope(|| hex::encode(Md5::digest(&value)));
        (sha256sum, sha1sum, md5sum)
    });
    let size = value.len() as i64;

    // Now that we're sure it's a valid Debian package, upload it into the pool.
    let package_name = control_file.package().unwrap();
    let package_name_start = package_name.chars().next().unwrap();
    let version = control_file.version().unwrap().to_string();
    let pool_filename = format!(
        "pool/{component}/{package_name_start}/{package_name}/{package_name}_{version}_{architecture}.deb"
    );

    let span = debug_span!("upload_to_pool");
    let key = format!("staging/{pool_filename}");
    async {
        let repo = sqlx::query!(
            "SELECT s3_bucket, s3_prefix FROM debian_repository WHERE id = $1",
            repository_id as i64
        )
        .fetch_one(&state.db)
        .await
        .unwrap();

        state
            .s3
            .put_object()
            .bucket(&repo.s3_bucket)
            .key(if repo.s3_prefix.is_empty() {
                key
            } else {
                format!("{}/{}", repo.s3_prefix, key)
            })
            .body(value.into())
            .send()
            .await
            .unwrap();
    }
    .instrument(span)
    .await;

    // Once the upload is complete, add a record to the database.
    let span = debug_span!("add_to_database");
    let package_row = async {
        let mut tx = state.db.begin().await.unwrap();
        sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").execute(&mut *tx).await.unwrap();
        let arch_row = sqlx::query!(
            "SELECT id FROM debian_repository_architecture WHERE name = $1 AND repository_id = $2",
            architecture,
            repository_id as i64
        )
        .fetch_optional(&mut *tx)
        .await
        .unwrap();
        let arch_id = match arch_row {
            Some(arch) => arch.id,
            None => {
                sqlx::query!(
                    "INSERT INTO debian_repository_architecture (name, repository_id) VALUES ($1, $2) RETURNING id",
                    architecture,
                    repository_id as i64
                )
                .fetch_one(&mut *tx)
                .await
                .unwrap()
                .id
            }
        };
        let component_row = sqlx::query!(
            "SELECT id FROM debian_repository_component WHERE name = $1 AND repository_id = $2",
            component,
            repository_id as i64
        )
        .fetch_optional(&mut *tx)
        .await
        .unwrap();
        let component_id = match component_row {
            Some(c) => c.id,
            None => {
                sqlx::query!(
                    "INSERT INTO debian_repository_component (name, repository_id) VALUES ($1, $2) RETURNING id",
                    component,
                    repository_id as i64
                )
                .fetch_one(&mut *tx)
                .await
                .unwrap()
                .id
            }
        };
        let paragraph = JsonValue::Object(
            control_file
                .as_str_hash_map()
                .into_iter()
                .map(|(k, v)| (k.to_string(), JsonValue::String(v.to_string())))
                .collect(),
        );
        let package_row = sqlx::query!(
            r#"
            INSERT INTO debian_repository_package (
                repository_id,
                architecture_id,
                component_id,

                staging_status,

                package,
                version,
                priority,
                section,
                installed_size,
                maintainer,
                description,
                homepage,

                paragraph,

                depends,
                recommends,
                conflicts,
                provides,
                replaces,

                filename,
                size,
                md5sum,
                sha1sum,
                sha256sum
            )
            VALUES ($1, $2, $3, $4::debian_repository_package_staging_status, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23)
            RETURNING id
            "#,
            repository_id as i64,
            arch_id,
            component_id,
            "add" as _,
            package_name,
            &version,
            control_file.priority(),
            control_file.section(),
            control_file.installed_size().map(|s| s.unwrap() as i64),
            control_file.maintainer().unwrap(),
            control_file.description().unwrap(),
            control_file.homepage(),
            paragraph,
            control_file.depends().map(|d| d.unwrap().to_string()),
            control_file.recommends().map(|d| d.unwrap().to_string()),
            control_file.field_dependency_list("conflicts").map(|d| d.unwrap().to_string()),
            control_file.field_dependency_list("provides").map(|d| d.unwrap().to_string()),
            control_file.field_dependency_list("replaces").map(|d| d.unwrap().to_string()),
            &pool_filename,
            size,
            md5sum,
            sha1sum,
            sha256sum,
        )
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        package_row
    }
    .instrument(span)
    .await;

    let None = multipart.next_field().await.unwrap() else {
        panic!("expected no more fields");
    };

    Json(Package {
        id: package_row.id,
        package: package_name.to_string(),
        version,
        architecture: architecture.to_string(),
    })
}

#[axum::debug_handler]
#[instrument]
pub async fn list_packages() -> Json<Vec<Package>> {
    todo!()
}

#[axum::debug_handler]
#[instrument]
pub async fn remove_package() -> Json<Package> {
    todo!()
}
