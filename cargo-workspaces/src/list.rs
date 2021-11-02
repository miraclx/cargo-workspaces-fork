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

        let workspace_groups = get_group_packages(&metadata, &config, self.list.all)?;

        workspace_groups
            .iter()
            .filter(|(group_name, _)| {
                self.list.groups.is_empty() || self.list.groups.contains(group_name)
            })
            .collect::<Vec<_>>()
            .list(self.list)
    }
}
