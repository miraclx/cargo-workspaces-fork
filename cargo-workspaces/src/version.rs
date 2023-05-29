use crate::utils::{info, read_config, Result, VersionOpt};
use cargo_metadata::Metadata;
use clap::Parser;

/// Bump version of crates
#[derive(Debug, Parser)]
pub struct Version {
    #[clap(flatten)]
    version: VersionOpt,
}

impl Version {
    pub fn run(self, metadata: Metadata) -> Result {
        let config = read_config(&metadata.workspace_metadata)?;

        let branch = self
            .version
            .git
            .validate(&metadata.workspace_root, &config)?;

        if let Some((new_version, new_versions)) = self.version.do_versioning(&metadata, &config)? {
            let mut tags = vec![];
            for (_, (pkg, ver)) in &new_versions {
                if let Some(tag) = self.version.git.individual_tag(
                    &metadata.workspace_root,
                    &pkg.name,
                    pkg.private,
                    &ver.to_string(),
                    &config,
                )? {
                    tags.push(tag)
                }
            }

            if let Some(new_version) = new_version {
                if let Some(tag) = self.version.git.global_tag(
                    &metadata.workspace_root,
                    &new_version,
                    &new_versions,
                )? {
                    tags.push(tag)
                }
            }

            self.version
                .git
                .push(&metadata.workspace_root, &branch, &tags)?;
        }

        info!("success", "ok");

        Ok(())
    }
}
