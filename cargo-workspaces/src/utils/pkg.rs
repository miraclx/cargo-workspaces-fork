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
    path::{Path, PathBuf},
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

#[derive(Eq, Clone, Debug, PartialEq, Serialize)]
pub struct PkgGroups {
    pub default: Vec<Pkg>,
    pub excluded: Vec<Pkg>,
    #[serde(flatten)]
    pub named_groups: HashMap<String, Vec<Pkg>>,
}

impl PkgGroups {
    fn is_empty(&self) -> bool {
        self.default.is_empty() && self.named_groups.is_empty()
    }

    fn all_pkgs(&self) -> impl Iterator<Item = &Pkg> {
        self.default
            .iter()
            .chain(self.named_groups.values().flatten())
            .chain(self.excluded.iter())
    }
}

macro_rules! iter {
    (($($pair:tt)+)$(, $($rest:tt)+)?) => {
        Some(($($pair)+)).into_iter()$(.chain(iter!($($rest)+)))?
    };
    (($($pair:tt)+) for ($($var:ident),+) in $expr:expr $(, $($rest:tt)+)?) => {
        $expr.iter().map(|($($var),+)| ($($pair)+))$(.chain(iter!($($rest)+)))?
    };
}

impl Listable for PkgGroups {
    fn list(&self, list: ListOpt) -> Result {
        if list.json {
            return self.json();
        }

        if self.is_empty() && self.excluded.is_empty() {
            return Ok(());
        }

        let (first, second, third) =
            self.all_pkgs()
                .fold((0, 0, 0), |(first, second, third), x| {
                    (
                        max(first, x.name.len()),
                        max(second, x.version.to_string().len() + 1),
                        max(third, max(1, x.path.as_os_str().len())),
                    )
                });

        for (group_name, pkgs) in iter![
            (None, &self.default),
            (Some(style(format!("[{}]", k)).green().to_string()), v) for (k, v) in self.named_groups,
            (Some(style("[excluded]").yellow().to_string()), &self.excluded)
        ] {
            if pkgs.is_empty() {
                continue;
            }
            if let Some(group_name) = group_name {
                TERM_OUT.write_line(&group_name)?;
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

pub fn get_pkg_groups(
    metadata: &Metadata,
    workspace_config: &WorkspaceConfig,
    all: bool,
) -> Result<PkgGroups> {
    let mut pkg_groups = PkgGroups {
        default: vec![],
        excluded: vec![],
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

            if let Some(ref exclude_spec) = workspace_config.exclude {
                if exclude_spec
                    .members
                    .iter()
                    .any(|x| x.matches_path(pkg.path.as_path()))
                {
                    pkg_groups.excluded.push(pkg);
                    continue;
                }
            }

            if let Some(ref package_groups) = workspace_config.group {
                if let Some(group) = package_groups.iter().find(|group| {
                    group
                        .members
                        .iter()
                        .any(|x| x.matches_path(pkg.path.as_path()))
                }) {
                    pkg_groups
                        .named_groups
                        .entry(group.name.clone())
                        .or_default()
                        .push(pkg);
                    continue;
                }
            }

            pkg_groups.default.push(pkg);
        } else {
            Error::PackageNotFound {
                id: id.repr.clone(),
            }
            .print()?;
        }
    }

    if pkg_groups.is_empty() {
        return Err(Error::EmptyWorkspace);
    }

    pkg_groups.default.sort();
    pkg_groups.excluded.sort();
    pkg_groups
        .named_groups
        .values_mut()
        .for_each(|pkgs| pkgs.sort());
    Ok(pkg_groups)
}
