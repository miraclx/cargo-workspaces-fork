use crate::utils::{
    cargo, cargo_config_get, check_index, dag, info, is_published, read_config, Error, Result,
    VersionOpt, INTERNAL_ERR,
};
use cargo_metadata::Metadata;
use clap::Parser;
use crates_index::Index;
use indexmap::IndexSet as Set;

/// Publish crates in the project
#[derive(Debug, Parser)]
#[clap(next_help_heading = "PUBLISH OPTIONS")]
pub struct Publish {
    #[clap(flatten, next_help_heading = None)]
    version: VersionOpt,

    /// Publish crates from the current commit without versioning
    // TODO: conflicts_with = "version" (group)
    #[clap(long)]
    from_git: bool,

    /// Skip already published crate versions
    #[clap(long, hide = true)]
    skip_published: bool,

    /// Skip crate verification (not recommended)
    #[clap(long)]
    no_verify: bool,

    /// Allow dirty working directories to be published
    #[clap(long)]
    allow_dirty: bool,

    /// The token to use for publishing
    #[clap(long, forbid_empty_values(true))]
    token: Option<String>,

    /// The Cargo registry to use for publishing
    #[clap(long, forbid_empty_values(true))]
    registry: Option<String>,
}

impl Publish {
    pub fn run(self, metadata: Metadata) -> Result {
        let config = read_config(&metadata.workspace_metadata)?;

        let mut versions = None;
        let branch = self
            .version
            .git
            .validate(&metadata.workspace_root, &config)?;

        let pkgs = if !self.from_git {
            let mut new_versions = vec![];
            if let Some((new_version, _new_versions)) =
                self.version.do_versioning(&metadata, &config)?
            {
                for (_, (pkg, ver)) in &_new_versions {
                    new_versions.push((
                        metadata
                            .packages
                            .iter()
                            .find(|y| pkg.id == y.id)
                            .expect(INTERNAL_ERR),
                        ver.clone(),
                    ));
                }
                versions = Some((new_version, _new_versions));
            }
            new_versions
        } else {
            metadata
                .packages
                .iter()
                .map(|x| (x, x.version.clone()))
                .collect()
        };

        let (names, visited) = dag(&pkgs);

        // Filter out private packages
        let visited = visited
            .into_iter()
            .filter(|x| {
                if let Some((pkg, _)) = pkgs.iter().find(|(p, _)| p.manifest_path == **x) {
                    return pkg.publish.is_none()
                        || !pkg.publish.as_ref().expect(INTERNAL_ERR).is_empty();
                }

                false
            })
            .collect::<Set<_>>();

        let mut tags = vec![];
        for p in &visited {
            let (pkg, version) = names.get(p).expect(INTERNAL_ERR);
            let name = pkg.name.clone();
            let mut args = vec!["publish"];

            let name_ver = format!("{} v{}", name, version);

            let mut index =
                if let Some(publish) = pkg.publish.as_deref().and_then(|x| x.get(0)).as_deref() {
                    let registry_url = cargo_config_get(
                        &metadata.workspace_root,
                        &format!("registries.{}.index", publish),
                    )?;
                    Index::from_url(&format!("registry+{}", registry_url))?
                } else {
                    Index::new_cargo_default()?
                };

            let version = version.to_string();

            if is_published(&mut index, &name, &version)? {
                info!("already published", name_ver);
                continue;
            }

            if self.no_verify {
                args.push("--no-verify");
            }

            if self.allow_dirty {
                args.push("--allow-dirty");
            }

            if let Some(ref registry) = self.registry {
                args.push("--registry");
                args.push(registry);
            }

            if let Some(ref token) = self.token {
                args.push("--token");
                args.push(token);
            }

            args.push("--manifest-path");
            args.push(p.as_str());

            let (_, stderr) = cargo(&metadata.workspace_root, &args, &[])?;

            if !stderr.contains("Uploading") || stderr.contains("error:") {
                return Err(Error::Publish(name));
            }

            check_index(&mut index, &name, &version)?;

            info!("published", name_ver);

            if let Some(tag) = self.version.git.individual_tag(
                &metadata.workspace_root,
                &pkg.name,
                pkg.publish.as_ref().map_or(false, Vec::is_empty),
                &version,
                &config,
            )? {
                tags.push(tag)
            }
        }

        if let Some((Some(new_version), new_versions)) = versions {
            if let Some(tag) = self.version.git.global_tag(
                &metadata.workspace_root,
                &new_version,
                &new_versions,
            )? {
                tags.push(tag)
            }

            self.version
                .git
                .push(&metadata.workspace_root, &branch, &tags)?;
        }

        info!("success", "ok");
        Ok(())
    }
}
