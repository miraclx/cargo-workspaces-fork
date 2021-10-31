use crate::utils::{get_pkg_groups, read_config, ListOpt, Listable, Result, WorkspaceConfig};
use cargo_metadata::Metadata;
use clap::Parser;

/// List crates in the project
#[derive(Debug, Parser)]
#[clap(alias = "ls")]
pub struct List {
    #[clap(flatten)]
    list: ListOpt,
}

impl List {
    pub fn run(self, metadata: Metadata) -> Result {
        let config: WorkspaceConfig = read_config(&metadata.workspace_metadata)?;

        let pkg_groups = get_pkg_groups(&metadata, &config, self.list.all)?;

        pkg_groups.list(self.list)
    }
}
