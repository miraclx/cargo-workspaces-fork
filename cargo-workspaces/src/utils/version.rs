use crate::utils::{
    cargo, change_versions, info, read_config, ChangeData, ChangeOpt, Error, GitOpt, Pkg, Result,
    WorkspaceConfig, INTERNAL_ERR,
};

use cargo_metadata::Metadata;
use clap::{ArgEnum, ArgSettings, Parser};
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
use oclif::{
    console::{style, Style},
    term::{TERM_ERR, TERM_OUT},
};
use semver::{Identifier, Version, VersionReq};

use std::{
    collections::{BTreeMap as Map, HashMap},
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
pub struct VersionOpt {
    /// Increment all versions by the given explicit
    /// semver keyword while skipping the prompts for them
    #[clap(arg_enum)]
    pub bump: Option<Bump>,

    /// Specify custom version value when 'bump' is set to 'custom'
    #[clap(required_if_eq("bump", "custom"))]
    pub custom: Option<Version>,

    /// Specify prerelease identifier
    #[clap(
        long,
        value_name = "identifier",
        setting(ArgSettings::ForbidEmptyValues)
    )]
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
}

impl VersionOpt {
    pub fn do_versioning(&self, metadata: &Metadata) -> Result<Map<String, Version>> {
        let config: WorkspaceConfig = read_config(&metadata.workspace_metadata)?;
        let branch = self.git.validate(&metadata.workspace_root, &config)?;

        let change_data = ChangeData::new(metadata, &self.change)?;

        if self.change.force.is_none() && change_data.count == "0" && !change_data.dirty {
            TERM_OUT.write_line("Current HEAD is already released, skipping versioning")?;
            return Ok(Map::new());
        }

        let (mut changed_p, mut unchanged_p) =
            self.change
                .get_changed_pkgs(metadata, &change_data.since, self.all)?;

        if changed_p.is_empty() {
            TERM_OUT.write_line("No changes detected, skipping versioning")?;
            return Ok(Map::new());
        }

        let mut new_version = None;
        let mut new_versions = vec![];

        while !changed_p.is_empty() {
            self.get_new_versions(metadata, changed_p, &mut new_version, &mut new_versions)?;

            let pkgs = unchanged_p.into_iter().partition::<Vec<_>, _>(|p| {
                let pkg = metadata
                    .packages
                    .iter()
                    .find(|x| x.name == p.name)
                    .expect(INTERNAL_ERR);

                pkg.dependencies.iter().any(|x| {
                    if let Some(version) = new_versions.iter().find(|y| x.name == y.0).map(|y| &y.1)
                    {
                        !x.req.matches(version)
                            || matches!(x.req.to_string().as_str(), "*" | ">=0.0.0")
                    } else {
                        false
                    }
                })
            });

            changed_p = pkgs.0;
            unchanged_p = pkgs.1;
        }

        let mut unversioned_deps = HashMap::new();

        for (p, _, _) in new_versions.iter() {
            let pkg = metadata
                .packages
                .iter()
                .find(|x| &x.name == p)
                .expect(INTERNAL_ERR);

            for dep in pkg.dependencies.iter() {
                if let Some((_, v, _)) = new_versions.iter().find(|(x, _, _)| x == &dep.name) {
                    if matches!(dep.req.to_string().as_str(), "*" | ">=0.0.0") {
                        unversioned_deps
                            .entry(pkg.id.repr.as_str())
                            .or_insert_with(|| (pkg.name.as_str(), vec![]))
                            .1
                            .push((dep.name.as_str(), &dep.req, v));
                    }
                }
            }
        }

        let autoversion = self.alert_unversioned(unversioned_deps)?;

        let new_versions = self.confirm_versions(new_versions)?;

        for p in &metadata.packages {
            if new_versions.get(&p.name).is_none()
                && p.dependencies
                    .iter()
                    .all(|x| new_versions.get(&x.name).is_none())
            {
                continue;
            }

            fs::write(
                &p.manifest_path,
                format!(
                    "{}\n",
                    change_versions(
                        fs::read_to_string(&p.manifest_path)?,
                        &p.name,
                        &new_versions,
                        self.exact,
                        autoversion,
                    )?
                ),
            )?;
        }

        for pkg in new_versions.keys() {
            let output = cargo(&metadata.workspace_root, &["update", "-p", pkg], &[])?;

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
        pkgs: Vec<Pkg>,
        new_version: &mut Option<Version>,
        new_versions: &mut Vec<(String, Version, Version)>,
    ) -> Result {
        let (independent_pkgs, same_pkgs) = pkgs
            .into_iter()
            .partition::<Vec<_>, _>(|p| p.config.independent.unwrap_or(false));

        if !same_pkgs.is_empty() {
            let cur_version = same_pkgs
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
                .expect(INTERNAL_ERR);

            if new_version.is_none() {
                info!("current common version", cur_version);

                *new_version = Some(self.ask_version(cur_version, None)?);
            }

            for p in &same_pkgs {
                new_versions.push((
                    p.name.to_string(),
                    new_version.as_ref().expect(INTERNAL_ERR).clone(),
                    cur_version.clone(),
                ));
            }
        }

        for p in &independent_pkgs {
            let new_version = self.ask_version(&p.version, Some(&p.name))?;
            new_versions.push((p.name.to_string(), new_version, p.version.clone()));
        }

        Ok(())
    }

    fn alert_unversioned(
        &self,
        pkgs: HashMap<&str, (&str, Vec<(&str, &VersionReq, &Version)>)>,
    ) -> Result<bool> {
        if pkgs.is_empty() {
            return Ok(false);
        }
        loop {
            match Select::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "You have {} packages with unversioned dependencies",
                    pkgs.len()
                ))
                .items(&["Review Dependencies", "Auto-version", "Skip"])
                .default(0)
                .clear(true)
                .interact_on(&TERM_ERR)?
            {
                2 => return Ok(false),
                1 => {
                    if Confirm::with_theme(&ColorfulTheme::default())
                        .with_prompt(
                            "Are you sure you want this tool to auto-inject these versions?",
                        )
                        .default(false)
                        .interact_on(&TERM_ERR)?
                    {
                        return Ok(true);
                    }
                }
                _ => {
                    let mut items = vec![];
                    for (name, deps) in pkgs.values() {
                        items.push(format!(" │ {}", style(name).green()));
                        for (dep, ver, new_ver) in deps {
                            items.push(format!(
                                " │ \u{21b3} {}: {} => {}",
                                style(dep).cyan(),
                                style(ver).yellow(),
                                style(new_ver).green()
                            ));
                        }
                    }
                    Select::new()
                        .with_prompt("Packages with unversioned dependencies")
                        .items(&items)
                        .default(0)
                        .clear(true)
                        .report(false)
                        .max_length(10)
                        .interact_on_opt(&TERM_ERR)?;
                }
            }
        }
    }

    fn confirm_versions(
        &self,
        versions: Vec<(String, Version, Version)>,
    ) -> Result<Map<String, Version>> {
        let mut new_versions = Map::new();
        let style = Style::new().for_stderr();

        TERM_ERR.write_line("\nChanges:")?;

        for v in versions {
            TERM_ERR.write_line(&format!(
                " - {}: {} => {}",
                style.clone().yellow().apply_to(&v.0),
                v.2,
                style.clone().cyan().apply_to(&v.1),
            ))?;
            new_versions.insert(v.0, v.1);
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

        Ok(new_versions)
    }

    fn ask_version(&self, cur_version: &Version, pkg_name: Option<&str>) -> Result<Version> {
        let mut items = version_items(cur_version, &self.pre_id);

        items.push(("Custom Prerelease".to_string(), None));
        items.push(("Custom Version".to_string(), None));

        let prompt = if let Some(name) = pkg_name {
            format!("for {} ", name)
        } else {
            "".to_string()
        };

        let theme = ColorfulTheme::default();

        let selected = if let Some(bump) = &self.bump {
            bump.selected()
        } else {
            Select::with_theme(&theme)
                .with_prompt(&format!(
                    "Select a new version {}(currently {})",
                    prompt, cur_version
                ))
                .items(&items.iter().map(|x| &x.0).collect::<Vec<_>>())
                .default(0)
                .interact_on(&TERM_ERR)?
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
