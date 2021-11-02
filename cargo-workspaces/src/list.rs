use crate::utils::{get_group_packages, read_config, ListOpt, Listable, Result, WorkspaceConfig};
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

        let workspace_groups = get_group_packages(
            &metadata,
            &config,
            self.list.all,
            if self.list.groups.is_empty() {
                None
            } else {
                Some(&self.list.groups[..])
            },
        )?;

        workspace_groups.list(self.list)
    }
}
