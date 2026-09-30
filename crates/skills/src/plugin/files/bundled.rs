/*
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing,
 * software distributed under the License is distributed on an
 * "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
 * KIND, either express or implied.  See the License for the
 * specific language governing permissions and limitations
 * under the License.
 */

//! Shipped skills are product-managed complete trees; preferences remain user-owned.
use super::{Failure, MutationRejection, Publisher, SourceCatalog, bundled_tree};
use maka_plugins::filesystem::ReadDirectory;
use maka_runtime::artifact::content_digest;
use std::collections::BTreeSet;
use tokio_util::sync::CancellationToken;

pub(in crate::plugin) async fn reconcile(
    publisher: &Publisher,
    published: &ReadDirectory,
    cancellation: &CancellationToken,
) -> Result<(), Failure> {
    let sources = crate::source_catalog(published, None, cancellation)
        .await
        .map_err(|error| super::Error::Source(error.to_string()))?;
    reconcile_sources(publisher, &sources, cancellation).await
}

async fn reconcile_sources(
    publisher: &Publisher,
    sources: &SourceCatalog,
    cancellation: &CancellationToken,
) -> Result<(), Failure> {
    let ids: BTreeSet<_> = sources
        .bundled
        .iter()
        .map(|source| source.id)
        .chain(sources.publication.origins.keys().filter_map(|reference| {
            sources.bundled_origin(reference)?;
            reference.strip_prefix("workspace:legacy:")
        }))
        .collect();
    for id in ids {
        let reference = format!("workspace:legacy:{id}");
        let origin = sources.bundled_origin(&reference);
        if origin.is_none() && sources.publication.occupied.contains(&id.to_lowercase()) {
            continue; // A same-name custom installation is not ours to replace.
        }
        let expected = publisher.capture(id, cancellation).await?;
        if let Some(expected) = &expected {
            let Some(origin) = origin else {
                continue;
            };
            // Do not adopt a custom replacement made since catalog capture.
            if expected.get("skill.lock.json").map(content_digest) != origin.lock_sha256 {
                return Err(Failure::Rejected(MutationRejection::SourceChanged));
            }
        }
        let next = sources
            .bundled
            .iter()
            .find(|source| source.id == id)
            .map(bundled_tree)
            .transpose()?;
        if expected.as_ref().map(|tree| tree.digest()).transpose()?
            == next.as_ref().map(|tree| tree.digest()).transpose()?
        {
            continue;
        }
        // A fresh tree replaces the whole directory, including removed resources.
        // Retired bundled IDs are deleted through the same recoverable publication.
        publisher
            .publish(id, expected.as_ref(), next.as_ref(), cancellation)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use maka_plugins::{composition::Scope, fiber::Fiber, filesystem::ReadRoot};

    async fn fixture() -> (tempfile::TempDir, Fiber, Publisher, ReadDirectory) {
        let root = tempfile::tempdir().unwrap();
        let owner = Fiber::new("maka.skills", "skills", Scope::Profile).unwrap();
        owner.begin_loading().unwrap();
        let data =
            cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
        let cancellation = CancellationToken::new();
        let publisher = Publisher::open(&data, &cancellation).unwrap();
        let published = ReadRoot::open(root.path())
            .await
            .unwrap()
            .bind(owner.context(), cancellation);
        (root, owner, publisher, published)
    }

    #[tokio::test]
    async fn bundled_upgrade_replaces_complete_trees_and_removes_retired_skills() {
        let (root, _owner, publisher, published) = fixture().await;
        let cancellation = CancellationToken::new();
        let mut old = crate::source_catalog(&published, None, &cancellation)
            .await
            .unwrap();
        let authoring = old
            .bundled
            .iter_mut()
            .find(|source| source.id == "maka-plugin-authoring")
            .unwrap();
        authoring.content =
            "---\nname: maka-plugin-authoring\ndescription: old bundle\n---\nOld instructions";
        authoring.files = &[("references/obsolete.md", b"obsolete contract")];
        // A cancelled initial publication is recoverable and cannot suppress retry.
        let interrupted = CancellationToken::new();
        interrupted.cancel();
        assert!(
            reconcile_sources(&publisher, &old, &interrupted)
                .await
                .is_err()
        );
        publisher.recover().await.unwrap();
        reconcile_sources(&publisher, &old, &cancellation)
            .await
            .unwrap();
        let mut retired = crate::publication::Tree::empty();
        super::super::artifacts(
            &mut retired,
            "maka-retired",
            "maka-retired",
            super::super::InstallSource::Bundled,
            b"---\nname: maka-retired\ndescription: old builtin\n---\nRetired".to_vec(),
        )
        .unwrap();
        publisher
            .publish("maka-retired", None, Some(&retired), &cancellation)
            .await
            .unwrap();
        let directory = root.path().join("skills/maka-plugin-authoring");
        assert!(directory.join("references/obsolete.md").exists());
        reconcile(&publisher, &published, &cancellation)
            .await
            .unwrap();
        assert!(!directory.join("references/obsolete.md").exists());
        assert!(!root.path().join("skills/maka-retired").exists());
        let sdk = directory.join("references/sdk/host.ts");
        assert_eq!(
            std::fs::read(&sdk).unwrap(),
            include_bytes!("../../../../../packages/plugin-sdk/src/host.ts")
        );
        // Resource-only drift is replaced too, not just a changed SKILL.md hash.
        std::fs::write(&sdk, "outdated SDK").unwrap();
        std::fs::write(directory.join("obsolete.txt"), "stale resource").unwrap();
        reconcile(&publisher, &published, &cancellation)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(&sdk).unwrap(),
            include_bytes!("../../../../../packages/plugin-sdk/src/host.ts")
        );
        assert!(!directory.join("obsolete.txt").exists());
        let unchanged = std::fs::metadata(&sdk).unwrap().modified().unwrap();
        reconcile(&publisher, &published, &cancellation)
            .await
            .unwrap();
        assert_eq!(
            std::fs::metadata(&sdk).unwrap().modified().unwrap(),
            unchanged
        );
        assert_eq!(
            std::fs::read_dir(root.path().join("transactions"))
                .unwrap()
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn reconciliation_preserves_custom_collisions_and_rechecks_captured_ownership() {
        let (root, _owner, publisher, published) = fixture().await;
        let cancellation = CancellationToken::new();
        let cua = root.path().join("skills/maka-cua");
        std::fs::create_dir_all(&cua).unwrap();
        std::fs::write(
            cua.join("SKILL.md"),
            "custom instructions without bundle metadata",
        )
        .unwrap();
        reconcile(&publisher, &published, &cancellation)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(cua.join("SKILL.md")).unwrap(),
            "custom instructions without bundle metadata"
        );
        let sources = crate::source_catalog(&published, None, &cancellation)
            .await
            .unwrap();
        let directory = root.path().join("skills/maka-plugin-authoring");
        std::fs::remove_file(directory.join("skill.lock.json")).unwrap();
        std::fs::write(
            directory.join("SKILL.md"),
            "custom replacement after capture",
        )
        .unwrap();
        assert!(matches!(
            reconcile_sources(&publisher, &sources, &cancellation).await,
            Err(Failure::Rejected(MutationRejection::SourceChanged))
        ));
        assert_eq!(
            std::fs::read_to_string(directory.join("SKILL.md")).unwrap(),
            "custom replacement after capture"
        );
        // A fresh catalog also treats that replacement as custom.
        reconcile(&publisher, &published, &cancellation)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.join("SKILL.md")).unwrap(),
            "custom replacement after capture"
        );
    }
}
