use crate::utils::{info, Result, VersionOpt};
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
        if let Some((config, tags, _)) = self.version.do_versioning(&metadata)? {
            let branch = self
                .version
                .git
                .validate(&metadata.workspace_root, &config)?;

            self.version
                .git
                .push(&metadata.workspace_root, &branch, &tags)?;
        }

        info!("success", "ok");

        Ok(())
    }
}
