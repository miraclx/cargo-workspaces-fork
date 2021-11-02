use crate::utils::{
    read_config, Error, ListOpt, Listable, PackageConfig, Result, WorkspaceConfig, INTERNAL_ERR,
};

use cargo_metadata::{Metadata, PackageId};
use oclif::{console::style, term::TERM_OUT, CliError};
use semver::Version;
use serde::Serialize;

use std::{
    cmp::max,
    collections::HashMap,
    iter::repeat,
    path::{Path, PathBuf},
    str::FromStr,
};

#[derive(Serialize, Debug, Clone, Ord, Eq, PartialOrd, PartialEq)]
pub struct Pkg {
    #[serde(skip)]
    pub id: PackageId,
    pub name: String,
    pub version: Version,
    pub location: PathBuf,
    #[serde(skip)]
    pub path: PathBuf,
    pub private: bool,
    #[serde(skip)]
    pub config: PackageConfig,
}

impl Listable for Vec<Pkg> {
    fn list(&self, list: ListOpt) -> Result {
        if list.json {
            return self.json();
        }

        if self.is_empty() {
            return Ok(());
        }

        let first = self.iter().map(|x| x.name.len()).max().expect(INTERNAL_ERR);
        let second = self
            .iter()
            .map(|x| x.version.to_string().len() + 1)
            .max()
            .expect(INTERNAL_ERR);
        let third = self
            .iter()
            .map(|x| max(1, x.path.as_os_str().len()))
            .max()
            .expect(INTERNAL_ERR);

        for pkg in self {
            TERM_OUT.write_str(&pkg.name)?;
            let mut width = first - pkg.name.len();

            if list.long {
                let path = if pkg.path.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    pkg.path.as_path()
                };

                TERM_OUT.write_str(&format!(
                    "{:f$} {}{:s$} {}",
                    "",
                    style(format!("v{}", pkg.version)).green(),
                    "",
                    style(path.display()).black().bright(),
                    f = width,
                    s = second - pkg.version.to_string().len() - 1,
                ))?;

                width = third - pkg.path.as_os_str().len();
            }

            if list.all && pkg.private {
                TERM_OUT.write_str(&format!(
                    "{:w$} ({})",
                    "",
                    style("PRIVATE").red(),
                    w = width
                ))?;
            }

            TERM_OUT.write_line("")?;
        }

        Ok(())
    }
}

pub fn get_pkgs(metadata: &Metadata, all: bool) -> Result<Vec<Pkg>> {
    let mut pkgs = vec![];

    for id in &metadata.workspace_members {
        if let Some(pkg) = metadata.packages.iter().find(|x| x.id == *id) {
            let private =
                pkg.publish.is_some() && pkg.publish.as_ref().expect(INTERNAL_ERR).is_empty();

            if !all && private {
                continue;
            }

            let loc = pkg.manifest_path.strip_prefix(&metadata.workspace_root);

            if loc.is_err() {
                return Err(Error::PackageNotInWorkspace {
                    id: pkg.id.repr.clone(),
                    ws: metadata.workspace_root.to_string(),
                });
            }

            let loc = loc.expect(INTERNAL_ERR);
            let loc = if loc.is_file() {
                loc.parent().expect(INTERNAL_ERR)
            } else {
                loc
            };

            pkgs.push(Pkg {
                id: pkg.id.clone(),
                name: pkg.name.clone(),
                version: pkg.version.clone(),
                location: metadata.workspace_root.join(loc).into(),
                path: loc.into(),
                private,
                config: read_config(&pkg.metadata)?,
            });
        } else {
            Error::PackageNotFound {
                id: id.repr.clone(),
            }
            .print()?;
        }
    }

    if pkgs.is_empty() {
        return Err(Error::EmptyWorkspace);
    }

    pkgs.sort();
    Ok(pkgs)
}

macro_rules! ser_unit_variant {
    ($variant:ident) => {
        pub mod $variant {
            pub fn ser<S>(s: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                s.serialize_str(stringify!($variant))
            }
        }
    };
}

mod ser_grp {
    ser_unit_variant!(default);
    ser_unit_variant!(excluded);
}

#[derive(Eq, Hash, Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum GroupName {
    #[serde(serialize_with = "ser_grp::default::ser")]
    Default,
    #[serde(serialize_with = "ser_grp::excluded::ser")]
    Excluded,
    Custom(String),
}

impl GroupName {
    pub fn pretty_fmt(&self) -> Option<String> {
        match self {
            GroupName::Default => None,
            GroupName::Excluded => Some(style(format!("[excluded]")).bold().yellow().to_string()),
            GroupName::Custom(group_name) => Some(
                style(format!("[{}]", group_name))
                    .bold()
                    .color256(37)
                    .to_string(),
            ),
        }
    }

    pub fn validate(s: &str) -> std::result::Result<(), String> {
        if s.contains(":") {
            return Err(format!("invalid character `:` in group name: {}", s));
        }
        Ok(())
    }
}

impl FromStr for GroupName {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::validate(s).map(|_| match s {
            "default" => GroupName::Default,
            "excluded" => GroupName::Excluded,
            custom => GroupName::Custom(custom.to_string()),
        })
    }
}

#[derive(Eq, Clone, Debug, PartialEq, Serialize)]
pub struct WorkspaceGroups {
    #[serde(flatten)]
    pub named_groups: HashMap<GroupName, Vec<Pkg>>,
}

impl WorkspaceGroups {
    pub fn is_empty(&self) -> bool {
        self.named_groups.is_empty()
    }

    pub fn iter_groups(&self) -> impl Iterator<Item = (&GroupName, &Vec<Pkg>)> {
        let default = self
            .named_groups
            .get_key_value(&GroupName::Default)
            .into_iter();
        let excluded = self
            .named_groups
            .get_key_value(&GroupName::Excluded)
            .into_iter();

        let rest = self
            .named_groups
            .iter()
            .filter(|(group, _)| !matches!(group, GroupName::Default | GroupName::Excluded));

        default.chain(rest).chain(excluded)
    }

    pub fn iter_pkg(&self) -> impl Iterator<Item = (&GroupName, &Pkg)> {
        self.iter_groups()
            .map(|(group, pkgs)| repeat(group).zip(pkgs.iter()))
            .flatten()
    }
}

impl Listable for WorkspaceGroups {
    fn list(&self, list: ListOpt) -> Result {
        if list.json {
            return self.json();
        }

        if self.is_empty() {
            return Ok(());
        }

        let (first, second, third) =
            self.iter_pkg()
                .fold((0, 0, 0), |(first, second, third), (_, x)| {
                    (
                        max(first, x.name.len()),
                        max(second, x.version.to_string().len() + 1),
                        max(third, max(1, x.path.as_os_str().len())),
                    )
                });

        for (group_name, pkgs) in self.iter_groups() {
            if pkgs.is_empty() {
                continue;
            }
            if let Some(group_name) = group_name.pretty_fmt() {
                TERM_OUT.write_line(&group_name.to_string())?;
            }
            for pkg in pkgs {
                TERM_OUT.write_str(&pkg.name)?;
                let mut width = first - pkg.name.len();

                if list.long {
                    let path = if pkg.path.as_os_str().is_empty() {
                        Path::new(".")
                    } else {
                        pkg.path.as_path()
                    };

                    TERM_OUT.write_str(&format!(
                        "{:f$} {}{:s$} {}",
                        "",
                        style(format!("v{}", pkg.version)).green(),
                        "",
                        style(path.display()).black().bright(),
                        f = width,
                        s = second - pkg.version.to_string().len() - 1,
                    ))?;

                    width = third - pkg.path.as_os_str().len();
                }

                if list.all && pkg.private {
                    TERM_OUT.write_str(&format!(
                        "{:w$} ({})",
                        "",
                        style("PRIVATE").red(),
                        w = width
                    ))?;
                }

                TERM_OUT.write_line("")?;
            }
        }

        Ok(())
    }
}

pub fn get_group_packages(
    metadata: &Metadata,
    workspace_config: &WorkspaceConfig,
    all: bool,
    filter: Option<&[GroupName]>,
    with_excluded: bool,
) -> Result<WorkspaceGroups> {
    let mut non_empty = false;
    let mut pkg_groups = WorkspaceGroups {
        named_groups: HashMap::new(),
    };

    for id in &metadata.workspace_members {
        if let Some(pkg) = metadata.packages.iter().find(|x| x.id == *id) {
            let private =
                pkg.publish.is_some() && pkg.publish.as_ref().expect(INTERNAL_ERR).is_empty();

            if !all && private {
                continue;
            }

            let loc = pkg.manifest_path.strip_prefix(&metadata.workspace_root);

            if loc.is_err() {
                return Err(Error::PackageNotInWorkspace {
                    id: pkg.id.repr.clone(),
                    ws: metadata.workspace_root.to_string(),
                });
            }

            let loc = loc.expect(INTERNAL_ERR);
            let loc = if loc.is_file() {
                loc.parent().expect(INTERNAL_ERR)
            } else {
                loc
            };

            let pkg = Pkg {
                id: pkg.id.clone(),
                name: pkg.name.clone(),
                version: pkg.version.clone(),
                location: metadata.workspace_root.join(loc).into(),
                path: loc.into(),
                private,
                config: read_config(&pkg.metadata)?,
            };

            let group_name = loop {
                if let Some(ref exclude_spec) = workspace_config.exclude {
                    if exclude_spec
                        .members
                        .iter()
                        .any(|x| x.matches_path(pkg.path.as_path()))
                    {
                        break GroupName::Excluded;
                    }
                };

                if let Some(ref package_groups) = workspace_config.group {
                    if let Some(group) = package_groups.iter().find(|group| {
                        group
                            .members
                            .iter()
                            .any(|x| x.matches_path(pkg.path.as_path()))
                    }) {
                        break GroupName::Custom(group.name.clone());
                    }
                }

                break GroupName::Default;
            };

            if filter.map_or(true, |filter| filter.contains(&group_name)) {
                non_empty |= !matches!(group_name, GroupName::Excluded);

                pkg_groups
                    .named_groups
                    .entry(group_name)
                    .or_default()
                    .push(pkg);
            }
        } else {
            Error::PackageNotFound {
                id: id.repr.clone(),
            }
            .print()?;
        }
    }

    if !(with_excluded || non_empty) {
        return Err(Error::EmptyWorkspace);
    }

    pkg_groups
        .named_groups
        .values_mut()
        .for_each(|pkgs| pkgs.sort());
    Ok(pkg_groups)
}
