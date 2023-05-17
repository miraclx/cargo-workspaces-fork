use crate::utils::{
    cargo, change_versions, is_unversioned, read_config, ChangeData, ChangeOpt, Error, GitOpt,
    GroupName, ManifestDiscriminant, Pkg, Result, WorkspaceConfig, INTERNAL_ERR,
};

use cargo_metadata::Metadata;
use clap::{ArgEnum, Parser};
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
use oclif::{
    console::style,
    term::{TERM_ERR, TERM_OUT},
};
use semver::{Identifier, Version, VersionReq};

use std::{
    collections::{BTreeMap as Map, HashMap, HashSet},
    fs,
    process::exit,
};

#[derive(Debug, Clone, ArgEnum)]
pub enum Bump {
    Major,
    Minor,
    Patch,
    Premajor,
    Preminor,
    Prepatch,
    Prerelease,
    Custom,
}

impl Bump {
    pub fn selected(&self) -> usize {
        match self {
            Bump::Major => 2,
            Bump::Minor => 1,
            Bump::Patch => 0,
            Bump::Premajor => 5,
            Bump::Preminor => 4,
            Bump::Prepatch => 3,
            Bump::Prerelease => 6,
            Bump::Custom => 7,
        }
    }
}

#[derive(Debug, Parser)]
#[clap(next_help_heading = "VERSION OPTIONS")]
pub struct VersionOpt {
    /// Increment all versions by the given explicit
    /// semver keyword while skipping the prompts for them
    #[clap(arg_enum, help_heading = "VERSION ARGS")]
    pub bump: Option<Bump>,

    /// Specify custom version value when 'bump' is set to 'custom'
    #[clap(required_if_eq("bump", "custom"), help_heading = "VERSION ARGS")]
    pub custom: Option<Version>,

    /// Specify prerelease identifier
    #[clap(long, value_name = "identifier", forbid_empty_values(true))]
    pub pre_id: Option<String>,

    #[clap(flatten)]
    pub change: ChangeOpt,

    #[clap(flatten)]
    pub git: GitOpt,

    /// Also do versioning for private crates (will not be published)
    #[clap(short, long)]
    pub all: bool,

    /// Specify inter dependency version numbers exactly with `=`
    #[clap(long)]
    pub exact: bool,

    /// Skip confirmation prompt
    #[clap(short, long)]
    pub yes: bool,

    /// Comma separated list of crate groups to version
    #[clap(
        long,
        multiple_occurrences = true,
        use_value_delimiter = true,
        number_of_values = 1
    )]
    pub groups: Vec<GroupName>,

    /// Do not use a pager for previewing package groups in interactive mode
    #[clap(long)]
    pub no_pager: bool,
}

impl VersionOpt {
    pub fn do_versioning(&self, metadata: &Metadata) -> Result<Map<String, (Pkg, Version)>> {
        let config: WorkspaceConfig = read_config(&metadata.workspace_metadata)?;
        let branch = self.git.validate(&metadata.workspace_root, &config)?;

        let last_tag = if !self.git.no_git {
            let change_data = ChangeData::new(metadata, &self.change)?;

            if self.change.force.is_none() && change_data.count == "0" && !change_data.dirty {
                TERM_OUT.write_line("Current HEAD is already released, skipping versioning")?;
                return Ok(Map::new());
            }

            change_data.since
        } else {
            None
        };

        let (mut changed_p, mut unchanged_p) = self.change.get_changed_pkgs(
            metadata,
            &config,
            &last_tag,
            &self.groups[..],
            self.all,
        )?;

        if changed_p.is_empty() {
            TERM_OUT.write_line("No changes detected, skipping versioning")?;
            return Ok(Map::new());
        }

        let mut bumped_pkgs = HashMap::new();

        while !changed_p.is_empty() {
            self.get_new_versions(metadata, changed_p, &mut bumped_pkgs)?;

            let pkgs = unchanged_p.into_iter().partition::<Vec<_>, _>(|(_, p)| {
                let pkg = metadata
                    .packages
                    .iter()
                    .find(|x| x.name == p.name)
                    .expect(INTERNAL_ERR);

                pkg.dependencies.iter().any(|x| {
                    bumped_pkgs.values().any(|(_, _, new_versions)| {
                        if let Some(version) = new_versions
                            .iter()
                            .find(|(p, _, _)| x.name == p.name)
                            .map(|y| &y.1)
                        {
                            !x.req.matches(version) || is_unversioned(&x.req)
                        } else {
                            false
                        }
                    })
                })
            });

            changed_p = pkgs.0;
            unchanged_p = pkgs.1;
        }

        if bumped_pkgs.is_empty() {
            TERM_OUT.write_line(
                "Changes detected but the versions weren't bumped, skipping versioning",
            )?;
            return Ok(Map::new());
        }

        let mut unversioned_deps = HashMap::new();

        let new_versions = bumped_pkgs
            .iter()
            .flat_map(|(_, (_, _, nv))| {
                nv.iter()
                    .map(|(pkg, ver, _)| (pkg.name.clone(), ver.clone()))
            })
            .collect::<Vec<_>>();

        for (pkg_name, new_version) in &new_versions {
            let pkg = metadata
                .packages
                .iter()
                .find(|cargo_pkg| pkg_name == &cargo_pkg.name)
                .expect(INTERNAL_ERR);

            for dep in pkg.dependencies.iter() {
                if let Some((_, pkg_ver)) = new_versions
                    .iter()
                    .find(|(pkg_name, _)| pkg_name == &dep.name)
                {
                    if is_unversioned(&dep.req) && !is_unversioned(new_version) {
                        unversioned_deps
                            .entry(pkg.id.repr.as_str())
                            .or_insert_with(|| (pkg.name.as_str(), vec![]))
                            .1
                            .push((dep.name.as_str(), &dep.req, pkg_ver));
                    }
                }
            }
        }

        self.alert_unversioned(unversioned_deps)?;

        let (new_version, new_versions) = self.confirm_versions(bumped_pkgs)?;

        let mut new_versions_root = Map::new();

        let workspace_root = metadata.workspace_root.join("Cargo.toml");
        let mut workspace_key = "<workspace>".to_string();

        for p in &metadata.packages {
            let deps = p
                .dependencies
                .iter()
                .filter_map(|dep| {
                    // todo! make sure the dep path is part of the workspace
                    dep.path.as_ref().and(new_versions.get(&dep.name).map(|_| {
                        (
                            dep.rename.as_ref().unwrap_or(&dep.name).clone(),
                            dep.name.clone(),
                        )
                    }))
                })
                .collect::<HashMap<_, _>>();

            if new_versions.get(&p.name).is_none()
                && deps
                    .iter()
                    .all(|(_key, pkg_name)| new_versions.get(pkg_name).is_none())
            {
                continue;
            }

            let mut new_versions_sub = deps
                .into_iter()
                .map(|(key, pkg_name)| {
                    (
                        key,
                        new_versions.get(&pkg_name).expect(INTERNAL_ERR).1.clone(),
                    )
                })
                .collect::<Map<_, _>>();

            if let Some((_, version)) = new_versions.get(&p.name) {
                new_versions_sub.insert(p.name.clone(), version.clone());
                new_versions_root.insert(p.name.clone(), version.clone());

                if p.manifest_path == workspace_root {
                    workspace_key = p.name.clone();
                }
            }

            let mut inherited_pkgs = HashSet::new();

            fs::write(
                &p.manifest_path,
                format!(
                    "{}\n",
                    change_versions(
                        fs::read_to_string(&p.manifest_path)?,
                        &p.name,
                        &new_versions_sub,
                        ManifestDiscriminant::Package,
                        self.exact,
                        &mut inherited_pkgs,
                    )?
                ),
            )?;

            new_versions_root.extend(inherited_pkgs.into_iter().filter_map(|pkg_name| {
                new_versions_sub
                    .get(&pkg_name)
                    .map(|version| (pkg_name, version.clone()))
            }));
        }

        if let Some(version) = &new_version {
            new_versions_root.insert(workspace_key.clone(), version.clone());
        }

        fs::write(
            &workspace_root,
            format!(
                "{}\n",
                change_versions(
                    fs::read_to_string(&workspace_root)?,
                    &workspace_key,
                    &new_versions_root,
                    ManifestDiscriminant::Workspace,
                    self.exact,
                    &mut HashSet::new(),
                )?
            ),
        )?;

        for (pkg_name, (p, _)) in &new_versions {
            let output = cargo(
                &metadata.workspace_root,
                &[
                    "update",
                    "-p",
                    &format!(
                        "file://{}#{}",
                        p.manifest_path.parent().expect(INTERNAL_ERR),
                        pkg_name
                    ),
                ],
                &[],
            )?;

            if output.1.contains("error:") {
                return Err(Error::Update);
            }
        }

        self.git.commit(
            &metadata.workspace_root,
            &new_version,
            &new_versions,
            branch,
            &config,
        )?;

        Ok(new_versions)
    }

    fn get_new_versions(
        &self,
        metadata: &Metadata,
        pkgs: Vec<((GroupName, Option<Version>), Pkg)>,
        bumped_pkgs: &mut HashMap<
            GroupName,
            (
                Option<Version>,
                Option<Version>,
                Vec<(Pkg, Version, Version)>,
            ),
        >,
    ) -> Result {
        let pkgs = pkgs
            .into_iter()
            .filter(|((group, _), _)| !matches!(group, GroupName::Excluded));

        let mut changed_pkg_groups = pkgs.fold(
            HashMap::new(),
            |mut groups, ((group_name, group_ver), pkg)| {
                groups
                    .entry(group_name)
                    .or_insert_with(|| (group_ver, vec![]))
                    .1
                    .push(pkg);
                groups
            },
        );

        let default_group = changed_pkg_groups.remove_entry(&GroupName::Default);

        for (group_name, (group_ver, pkgs)) in default_group.into_iter().chain(changed_pkg_groups) {
            let (common_version, new_group_version, new_versions) = loop {
                match bumped_pkgs.get_mut(&group_name) {
                    Some(pkg) => break pkg,
                    None => {
                        bumped_pkgs.insert(group_name.clone(), Default::default());
                    }
                }
            };
            let (independent_pkgs, same_pkgs) = pkgs
                .into_iter()
                .partition::<Vec<_>, _>(|p| p.config.independent.unwrap_or(false));

            if !same_pkgs.is_empty() {
                let group_version = match group_ver {
                    Some(ver) => ver,
                    None => {
                        let mut group_version = same_pkgs
                            .iter()
                            .map(|p| {
                                &metadata
                                    .packages
                                    .iter()
                                    .find(|x| x.id == p.id)
                                    .expect(INTERNAL_ERR)
                                    .version
                            })
                            .max()
                            .expect(INTERNAL_ERR)
                            .clone();
                        if common_version.is_none() {
                            let custom_group_version = self.ask_version(
                                &group_version,
                                &group_name,
                                Some(&same_pkgs[..]),
                                None,
                            )?;
                            *common_version = Some(group_version);
                            group_version = custom_group_version;
                        }
                        group_version
                    }
                };

                if let None = new_group_version {
                    *new_group_version = Some(group_version.clone());
                }

                for p in same_pkgs {
                    let old_version = p.version.clone();
                    if old_version != group_version {
                        new_versions.push((p, group_version.clone(), old_version));
                    }
                }
            }

            for p in independent_pkgs {
                let old_version = p.version.clone();
                let new_version =
                    self.ask_version(&old_version, &group_name, None, Some(&p.name))?;
                if old_version != new_version {
                    new_versions.push((p, new_version, old_version));
                }
            }

            if new_versions.is_empty() {
                bumped_pkgs.remove(&group_name);
            }
        }

        Ok(())
    }

    fn alert_unversioned(
        &self,
        pkgs: HashMap<&str, (&str, Vec<(&str, &VersionReq, &Version)>)>,
    ) -> Result {
        if pkgs.is_empty() || self.yes {
            return Ok(());
        }
        loop {
            match Select::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "You have {} packages with unversioned dependencies",
                    pkgs.len()
                ))
                .items(&["Review Dependencies", "Auto-version", "Abort"])
                .default(0)
                .clear(true)
                .interact_on_opt(&TERM_ERR)?
            {
                Some(2) | None => exit(0),
                Some(1) => {
                    if Confirm::with_theme(&ColorfulTheme::default())
                        .with_prompt(
                            "Are you sure you want this tool to auto-inject these versions?",
                        )
                        .default(false)
                        .interact_on(&TERM_ERR)?
                    {
                        return Ok(());
                    }
                }
                _ => {
                    let mut items = vec![];
                    for (name, deps) in pkgs.values() {
                        items.push(format!(" │ {}", style(name).green()));
                        for (dep, _, new_ver) in deps {
                            items.push(format!(" │  \u{21b3} {}:", style(dep).cyan()));
                            items.push(format!(
                                " │     \u{21b3} +{}",
                                style(format!("version = \"{}\"", new_ver)).green()
                            ));
                        }
                    }
                    Select::new()
                        .with_prompt("Packages with unversioned dependencies")
                        .items(&items)
                        .default(0)
                        .clear(true)
                        .report(false)
                        .max_length(15)
                        .interact_on_opt(&TERM_ERR)?;
                }
            }
        }
    }

    fn confirm_versions(
        &self,
        mut bumped_pkgs: HashMap<
            GroupName,
            (
                Option<Version>,
                Option<Version>,
                Vec<(Pkg, Version, Version)>,
            ),
        >,
    ) -> Result<(Option<Version>, Map<String, (Pkg, Version)>)> {
        let mut new_versions = Map::new();

        TERM_ERR.write_line("\nChanges:")?;

        let default_group = bumped_pkgs.remove_entry(&GroupName::Default);
        let new_version = default_group
            .as_ref()
            .and_then(|(_, (_, group_version, _))| group_version.clone());

        for (group, (grp_common_version, _, versions)) in
            default_group.into_iter().chain(bumped_pkgs)
        {
            if versions.is_empty() {
                continue;
            }
            if let Some(group_name) = group.pretty_fmt() {
                TERM_ERR.write_str(&format!(" {}", group_name))?;
            }
            if let Some(version) = grp_common_version {
                TERM_ERR.write_line(&format!(
                    " (current common version: {})",
                    style(version.to_string()).yellow().for_stderr()
                ))?;
            } else {
                TERM_ERR.write_line("")?;
            }
            for (p, new_version, cur_version) in versions {
                TERM_ERR.write_line(&format!(
                    " - {}: {} => {}",
                    style(&p.name).yellow().for_stderr(),
                    cur_version,
                    style(&new_version).yellow().for_stderr()
                ))?;
                new_versions.insert(p.name.clone(), (p, new_version));
            }
        }

        TERM_ERR.write_line("")?;
        TERM_ERR.flush()?;

        let create = self.yes
            || Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt("Are you sure you want to create these versions?")
                .default(false)
                .interact_on(&TERM_ERR)?;

        if !create {
            exit(0);
        }

        Ok((new_version, new_versions))
    }

    fn ask_version(
        &self,
        cur_version: &Version,
        group: &GroupName,
        mut group_pkgs: Option<&[Pkg]>,
        pkg_name: Option<&str>,
    ) -> Result<Version> {
        let mut items = version_items(cur_version, &self.pre_id);

        items.push(("Custom Prerelease".to_string(), None));
        items.push(("Custom Version".to_string(), None));

        let prompt = match (group, pkg_name) {
            (GroupName::Custom(group_name), Some(name)) => {
                format!("for {} in the group `{}` ", name, group_name)
            }
            (GroupName::Custom(group_name), None) => {
                format!("for the group `{}` ", group_name)
            }
            (_, Some(name)) => format!("for {} ", name),
            (_, None) => "for the workspace ".to_string(),
        };

        let theme = ColorfulTheme::default();

        let selected = loop {
            let mut selected = if let Some(bump) = &self.bump {
                bump.selected()
            } else {
                let items = items.iter().map(|x| x.0.as_str());

                let items: Vec<_> = if let Some(_) = group_pkgs {
                    Some("List Packages Affected")
                        .into_iter()
                        .chain(items)
                        .collect()
                } else {
                    items.collect()
                };

                Select::with_theme(&theme)
                    .with_prompt(&format!(
                        "Select a new version {}(currently {})",
                        prompt, cur_version
                    ))
                    .items(&items)
                    .default(0)
                    .interact_on(&TERM_ERR)?
            };

            if let Some(group_pkgs) = if self.no_pager {
                // take, so we only get the list option once
                group_pkgs.take()
            } else {
                group_pkgs
            } {
                if selected == 0 {
                    if self.no_pager {
                        for (i, p) in group_pkgs.iter().enumerate() {
                            TERM_ERR.write_line(&format!(
                                " {:>s$} │ {}: {}",
                                i + 1,
                                style(&p.name).yellow().for_stderr(),
                                p.version,
                                s = (group_pkgs.len() as f32).log10() as usize + 1,
                            ))?;
                        }
                    } else {
                        let group_pkgs = group_pkgs
                            .iter()
                            .enumerate()
                            .map(|(i, p)| {
                                format!(
                                    " {:>s$} │ {}: {}",
                                    i + 1,
                                    style(&p.name).yellow().for_stderr(),
                                    p.version,
                                    s = (group_pkgs.len() as f32).log10() as usize + 1,
                                )
                            })
                            .collect::<Vec<_>>();
                        Select::new()
                            .with_prompt(format!(
                                "{} packages affected in this group",
                                group_pkgs.len()
                            ))
                            .items(&group_pkgs)
                            .default(0)
                            .clear(true)
                            .report(false)
                            .max_length(10)
                            .interact_on_opt(&TERM_ERR)?;
                    }
                    continue;
                }
                selected -= 1;
            }

            break selected;
        };

        let new_version = if selected == 6 {
            let custom = custom_pre(cur_version);

            let preid = if let Some(preid) = &self.pre_id {
                preid.clone()
            } else {
                Input::with_theme(&theme)
                    .with_prompt(&format!(
                        "Enter a prerelease identifier (default: '{}', yielding {})",
                        custom.0, custom.1
                    ))
                    .default(custom.0.to_string())
                    .interact_on(&TERM_ERR)?
            };

            inc_preid(cur_version, Identifier::AlphaNumeric(preid))
        } else if selected == 7 {
            if let Some(version) = &self.custom {
                version.clone()
            } else {
                Input::with_theme(&theme)
                    .with_prompt("Enter a custom version")
                    .interact_on(&TERM_ERR)?
            }
        } else {
            items
                .get(selected)
                .expect(INTERNAL_ERR)
                .clone()
                .1
                .expect(INTERNAL_ERR)
        };

        Ok(new_version)
    }
}

fn inc_pre(pre: &[Identifier], preid: &Option<String>) -> Vec<Identifier> {
    match pre.get(0) {
        Some(Identifier::AlphaNumeric(id)) => {
            vec![Identifier::AlphaNumeric(id.clone()), Identifier::Numeric(0)]
        }
        Some(Identifier::Numeric(_)) => vec![Identifier::Numeric(0)],
        None => vec![
            Identifier::AlphaNumeric(
                preid
                    .as_ref()
                    .map_or_else(|| "alpha".to_string(), |x| x.clone()),
            ),
            Identifier::Numeric(0),
        ],
    }
}

fn inc_preid(cur_version: &Version, preid: Identifier) -> Version {
    let mut version = cur_version.clone();

    if cur_version.pre.is_empty() {
        version.increment_patch();
        version.pre = vec![preid, Identifier::Numeric(0)];
    } else {
        match cur_version.pre.get(0).expect(INTERNAL_ERR) {
            Identifier::AlphaNumeric(id) => {
                version.pre = vec![preid.clone()];

                if preid.to_string() == *id {
                    match cur_version.pre.get(1) {
                        Some(Identifier::Numeric(n)) => {
                            version.pre.push(Identifier::Numeric(n + 1))
                        }
                        _ => version.pre.push(Identifier::Numeric(0)),
                    };
                } else {
                    version.pre.push(Identifier::Numeric(0));
                }
            }
            Identifier::Numeric(n) => {
                if preid.to_string() == n.to_string() {
                    version.pre = cur_version.pre.clone();

                    if let Some(Identifier::Numeric(n)) = version
                        .pre
                        .iter_mut()
                        .rfind(|x| matches!(x, Identifier::Numeric(_)))
                    {
                        *n += 1;
                    }
                } else {
                    version.pre = vec![preid, Identifier::Numeric(0)];
                }
            }
        }
    }

    version
}

fn custom_pre(cur_version: &Version) -> (Identifier, Version) {
    let id = if let Some(id) = cur_version.pre.get(0) {
        id.clone()
    } else {
        Identifier::AlphaNumeric("alpha".to_string())
    };

    (id.clone(), inc_preid(cur_version, id))
}

fn inc_patch(mut cur_version: Version) -> Version {
    if !cur_version.pre.is_empty() {
        cur_version.pre.clear();
    } else {
        cur_version.increment_patch();
    }

    cur_version
}

fn inc_minor(mut cur_version: Version) -> Version {
    if !cur_version.pre.is_empty() && cur_version.patch == 0 {
        cur_version.pre.clear();
    } else {
        cur_version.increment_minor();
    }

    cur_version
}

fn inc_major(mut cur_version: Version) -> Version {
    if !cur_version.pre.is_empty() && cur_version.patch == 0 && cur_version.minor == 0 {
        cur_version.pre.clear();
    } else {
        cur_version.increment_major();
    }

    cur_version
}

fn version_items(cur_version: &Version, preid: &Option<String>) -> Vec<(String, Option<Version>)> {
    let mut items = vec![];

    let v = inc_patch(cur_version.clone());
    items.push((format!("Patch ({})", &v), Some(v)));

    let v = inc_minor(cur_version.clone());
    items.push((format!("Minor ({})", &v), Some(v)));

    let v = inc_major(cur_version.clone());
    items.push((format!("Major ({})", &v), Some(v)));

    let mut v = cur_version.clone();
    v.increment_patch();
    v.pre = inc_pre(&cur_version.pre, preid);
    items.push((format!("Prepatch ({})", &v), Some(v)));

    let mut v = cur_version.clone();
    v.increment_minor();
    v.pre = inc_pre(&cur_version.pre, preid);
    items.push((format!("Preminor ({})", &v), Some(v)));

    let mut v = cur_version.clone();
    v.increment_major();
    v.pre = inc_pre(&cur_version.pre, preid);
    items.push((format!("Premajor ({})", &v), Some(v)));

    items
}

#[cfg(test)]
mod test_super {
    use super::*;

    #[test]
    fn test_inc_patch() {
        let v = inc_patch(Version::parse("0.7.2").unwrap());
        assert_eq!(v.to_string(), "0.7.3");
    }

    #[test]
    fn test_inc_patch_on_prepatch() {
        let v = inc_patch(Version::parse("0.7.2-rc.0").unwrap());
        assert_eq!(v.to_string(), "0.7.2");
    }

    #[test]
    fn test_inc_patch_on_preminor() {
        let v = inc_patch(Version::parse("0.7.0-rc.0").unwrap());
        assert_eq!(v.to_string(), "0.7.0");
    }

    #[test]
    fn test_inc_patch_on_premajor() {
        let v = inc_patch(Version::parse("1.0.0-rc.0").unwrap());
        assert_eq!(v.to_string(), "1.0.0");
    }

    #[test]
    fn test_inc_minor() {
        let v = inc_minor(Version::parse("0.7.2").unwrap());
        assert_eq!(v.to_string(), "0.8.0");
    }

    #[test]
    fn test_inc_minor_on_prepatch() {
        let v = inc_minor(Version::parse("0.7.2-rc.0").unwrap());
        assert_eq!(v.to_string(), "0.8.0");
    }

    #[test]
    fn test_inc_minor_on_preminor() {
        let v = inc_minor(Version::parse("0.7.0-rc.0").unwrap());
        assert_eq!(v.to_string(), "0.7.0");
    }

    #[test]
    fn test_inc_minor_on_premajor() {
        let v = inc_minor(Version::parse("1.0.0-rc.0").unwrap());
        assert_eq!(v.to_string(), "1.0.0");
    }

    #[test]
    fn test_inc_major() {
        let v = inc_major(Version::parse("0.7.2").unwrap());
        assert_eq!(v.to_string(), "1.0.0");
    }

    #[test]
    fn test_inc_major_on_prepatch() {
        let v = inc_major(Version::parse("0.7.2-rc.0").unwrap());
        assert_eq!(v.to_string(), "1.0.0");
    }

    #[test]
    fn test_inc_major_on_preminor() {
        let v = inc_major(Version::parse("0.7.0-rc.0").unwrap());
        assert_eq!(v.to_string(), "1.0.0");
    }

    #[test]
    fn test_inc_major_on_premajor_with_patch() {
        let v = inc_major(Version::parse("1.0.1-rc.0").unwrap());
        assert_eq!(v.to_string(), "2.0.0");
    }

    #[test]
    fn test_inc_major_on_premajor() {
        let v = inc_major(Version::parse("1.0.0-rc.0").unwrap());
        assert_eq!(v.to_string(), "1.0.0");
    }

    #[test]
    fn test_inc_preid() {
        let v = inc_preid(
            &Version::parse("3.0.0").unwrap(),
            Identifier::AlphaNumeric("beta".to_string()),
        );
        assert_eq!(v.to_string(), "3.0.1-beta.0");
    }

    #[test]
    fn test_inc_preid_on_alpha() {
        let v = inc_preid(
            &Version::parse("3.0.0-alpha.19").unwrap(),
            Identifier::AlphaNumeric("beta".to_string()),
        );
        assert_eq!(v.to_string(), "3.0.0-beta.0");
    }

    #[test]
    fn test_inc_preid_on_num() {
        let v = inc_preid(
            &Version::parse("3.0.0-11.19").unwrap(),
            Identifier::AlphaNumeric("beta".to_string()),
        );
        assert_eq!(v.to_string(), "3.0.0-beta.0");
    }

    #[test]
    fn test_custom_pre() {
        let v = custom_pre(&Version::parse("3.0.0").unwrap());
        assert_eq!(v.0, Identifier::AlphaNumeric("alpha".to_string()));
        assert_eq!(v.1.to_string(), "3.0.1-alpha.0");
    }

    #[test]
    fn test_custom_pre_on_single_alpha() {
        let v = custom_pre(&Version::parse("3.0.0-a").unwrap());
        assert_eq!(v.0, Identifier::AlphaNumeric("a".to_string()));
        assert_eq!(v.1.to_string(), "3.0.0-a.0");
    }

    #[test]
    fn test_custom_pre_on_single_alpha_with_second_num() {
        let v = custom_pre(&Version::parse("3.0.0-a.11").unwrap());
        assert_eq!(v.0, Identifier::AlphaNumeric("a".to_string()));
        assert_eq!(v.1.to_string(), "3.0.0-a.12");
    }

    #[test]
    fn test_custom_pre_on_second_alpha() {
        let v = custom_pre(&Version::parse("3.0.0-a.b").unwrap());
        assert_eq!(v.0, Identifier::AlphaNumeric("a".to_string()));
        assert_eq!(v.1.to_string(), "3.0.0-a.0");
    }

    #[test]
    fn test_custom_pre_on_second_alpha_with_num() {
        let v = custom_pre(&Version::parse("3.0.0-a.b.1").unwrap());
        assert_eq!(v.0, Identifier::AlphaNumeric("a".to_string()));
        assert_eq!(v.1.to_string(), "3.0.0-a.0");
    }

    #[test]
    fn test_custom_pre_on_single_num() {
        let v = custom_pre(&Version::parse("3.0.0-11").unwrap());
        assert_eq!(v.0, Identifier::Numeric(11));
        assert_eq!(v.1.to_string(), "3.0.0-12");
    }

    #[test]
    fn test_custom_pre_on_single_num_with_second_alpha() {
        let v = custom_pre(&Version::parse("3.0.0-11.a").unwrap());
        assert_eq!(v.0, Identifier::Numeric(11));
        assert_eq!(v.1.to_string(), "3.0.0-12.a");
    }

    #[test]
    fn test_custom_pre_on_second_num() {
        let v = custom_pre(&Version::parse("3.0.0-11.20").unwrap());
        assert_eq!(v.0, Identifier::Numeric(11));
        assert_eq!(v.1.to_string(), "3.0.0-11.21");
    }

    #[test]
    fn test_custom_pre_on_multiple_num() {
        let v = custom_pre(&Version::parse("3.0.0-11.20.a.55.c").unwrap());
        assert_eq!(v.0, Identifier::Numeric(11));
        assert_eq!(v.1.to_string(), "3.0.0-11.20.a.56.c");
    }
}
