use crate::utils::{
    get_group_packages, read_config, rename_packages, validate_value_containing_name, Error,
    GroupName, WorkspaceConfig,
};
use cargo_metadata::Metadata;
use clap::{ArgSettings, Parser};
use glob::{Pattern, PatternError};
use std::{collections::BTreeMap as Map, fs};

/// Rename crates in the project
#[derive(Debug, Parser)]
pub struct Rename {
    /// Rename private creates too
    #[clap(short, long)]
    pub all: bool,

    /// Ignore the crates matched by glob
    #[clap(long, value_name = "pattern")]
    pub ignore: Option<String>,

    /// The value that should be used as new name (should contain `%n`)
    #[clap(
        validator = validate_value_containing_name,
        setting(ArgSettings::ForbidEmptyValues)
    )]
    pub to: String,

    /// Specify which package groups to rename
    #[clap(
        long,
        multiple_occurrences = true,
        use_delimiter = true,
        number_of_values = 1
    )]
    pub groups: Vec<GroupName>,
}

impl Rename {
    pub fn run(self, metadata: Metadata) -> Result<(), Error> {
        let config: WorkspaceConfig = read_config(&metadata.workspace_metadata)?;

        let workspace_groups = get_group_packages(
            &metadata,
            &config,
            self.all,
            if self.groups.is_empty() {
                None
            } else {
                Some(&self.groups[..])
            },
            false,
        )?;

        let ignore = self
            .ignore
            .clone()
            .map(|x| Pattern::new(&x))
            .map_or::<Result<_, PatternError>, _>(Ok(None), |x| Ok(x.ok()))?;

        let mut rename_map = Map::new();

        for (_, pkg) in workspace_groups.into_iter() {
            if let Some(pattern) = &ignore {
                if pattern.matches(&pkg.name) {
                    continue;
                }
            }

            let new_name = self.to.replace("%n", &pkg.name);

            rename_map.insert(pkg.name, new_name);
        }

        for pkg in &metadata.packages {
            if rename_map.contains_key(&pkg.name)
                || pkg
                    .dependencies
                    .iter()
                    .map(|p| &p.name)
                    .any(|p| rename_map.contains_key(p))
            {
                fs::write(
                    &pkg.manifest_path,
                    format!(
                        "{}\n",
                        rename_packages(
                            fs::read_to_string(&pkg.manifest_path)?,
                            &pkg.name,
                            &rename_map,
                        )?
                    ),
                )?;
            }
        }

        Ok(())
    }
}
