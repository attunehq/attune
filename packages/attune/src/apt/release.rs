use std::{collections::BTreeSet, fmt::Write as _, io::Write as _};

use sqlx::{FromRow, Postgres, Transaction};
use tabwriter::{Alignment, TabWriter};
use time::{OffsetDateTime, format_description::well_known::Rfc2822};

use crate::{api::TenantID, apt::PackagesIndexMeta};

#[derive(FromRow, Debug)]
pub struct ReleaseMeta {
    pub description: Option<String>,
    pub origin: Option<String>,
    pub label: Option<String>,
    pub version: Option<String>,
    pub suite: String,
    pub codename: String,
}

impl ReleaseMeta {
    pub async fn query_from_release<'a>(
        tx: &mut Transaction<'a, Postgres>,
        tenant_id: &TenantID,
        repository: &str,
        distribution: &str,
    ) -> Option<Self> {
        sqlx::query_as!(Self, r#"
            SELECT
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
            repository,
            distribution,
        )
        .fetch_optional(&mut **tx)
        .await
        .unwrap()
    }
}

#[derive(Debug)]
pub struct ReleaseFile {
    pub meta: ReleaseMeta,
    pub contents: String,
}

impl ReleaseFile {
    pub fn from_indexes(
        release: ReleaseMeta,
        release_ts: OffsetDateTime,
        packages_indexes: &Vec<PackagesIndexMeta>,
    ) -> Self {
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
        for p in packages_indexes {
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
        let mut release_file = vec![
            ("Origin", release.origin.clone()),
            ("Label", release.label.clone()),
            ("Version", release.version.clone()),
            ("Suite", Some(release.suite.clone())),
            ("Codename", Some(release.codename.clone())),
            ("Date", Some(date)),
            ("Architectures", Some(archs.to_string())),
            ("Components", Some(comps.to_string())),
            ("Description", release.description.clone()),
        ]
        .into_iter()
        .fold(String::new(), |mut acc, (k, v)| {
            if let Some(v) = v {
                write!(acc, "{}: {}\n", k, v).unwrap();
            }
            acc
        });

        // Write index fingerprints.
        release_file += "MD5Sum:\n";
        let mut md5writer = TabWriter::new(vec![])
            .alignment(Alignment::Right)
            .padding(1);
        for index in packages_indexes {
            // TODO(#94): Handle compressed indexes.
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
        for index in packages_indexes {
            // TODO(#94): Handle compressed indexes.
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
            meta: release,
            contents: release_file,
        }
    }
}
