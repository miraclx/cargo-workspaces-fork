use crate::utils::{debug, get_debug, info, Error, Result, INTERNAL_ERR};

use camino::Utf8Path;
use crates_index::Index;
use lazy_static::lazy_static;
use oclif::term::TERM_ERR;
use regex::{Captures, Regex};
use semver::{Version, VersionReq};

use std::{
    cell::RefCell,
    collections::{BTreeMap as Map, HashSet},
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    rc::Rc,
    thread::sleep,
    time::{Duration, Instant},
};

const CRLF: &str = "\r\n";
const LF: &str = "\n";

lazy_static! {
    static ref NAME: Regex =
        Regex::new(r#"^(\s*['"]?name['"]?\s*=\s*['"])([0-9A-Za-z-_]+)(['"].*)$"#).expect(INTERNAL_ERR);
    static ref VERSION: Regex =
        Regex::new(r#"^(\s*['"]?version['"]?\s*=\s*['"])([^'"]+)(['"].*)$"#)
            .expect(INTERNAL_ERR);
    static ref PACKAGE: Regex =
        Regex::new(r#"^(\s*['"]?package['"]?\s*=\s*['"])([0-9A-Za-z-_]+)(['"].*)$"#).expect(INTERNAL_ERR);
    static ref PACKAGE_TABLE: Regex =
        Regex::new(r#"^\[(workspace\.)?package]"#).expect(INTERNAL_ERR);
    static ref DEP_TABLE: Regex =
        Regex::new(r#"^\[(target\.'?([^']+)'?\.|workspace\.)?dependencies]"#).expect(INTERNAL_ERR);
    static ref DEP_ENTRY: Regex =
        Regex::new(r#"^\[(?:workspace\.)?dependencies\.([0-9A-Za-z-_]+)]"#).expect(INTERNAL_ERR);
    static ref BUILD_DEP_TABLE: Regex =
        Regex::new(r#"^\[(target\.'?([^']+)'?\.)?build-dependencies]"#).expect(INTERNAL_ERR);
    static ref BUILD_DEP_ENTRY: Regex =
        Regex::new(r#"^\[build-dependencies\.([0-9A-Za-z-_]+)]"#).expect(INTERNAL_ERR);
    static ref DEV_DEP_TABLE: Regex =
        Regex::new(r#"^\[(target\.'?([^']+)'?\.)?dev-dependencies]"#).expect(INTERNAL_ERR);
    static ref DEV_DEP_ENTRY: Regex =
        Regex::new(r#"^\[dev-dependencies\.([0-9A-Za-z-_]+)]"#).expect(INTERNAL_ERR);
    static ref DEP_DIRECT_VERSION: Regex =
        Regex::new(r#"^(\s*['"]?([0-9A-Za-z-_]+)['"]?\s*=\s*['"])([^'"]+)(['"].*)$"#)
            .expect(INTERNAL_ERR);
    static ref DEP_DIRECT_INHERITED: Regex =
        Regex::new(r#"^\s*['"]?([0-9A-Za-z-_]+)['"]?\s*\.\s*['"]?workspace['"]?\s*=\s*true\s*.*$"#)
            .expect(INTERNAL_ERR);
    static ref DEP_OBJ_VERSION: Regex =
        Regex::new(r#"^(\s*['"]?([0-9A-Za-z-_]+)['"]?\s*=\s*\{.*['"]?version['"]?\s*=\s*['"])([^'"]+)(['"].*}.*)$"#)
            .expect(INTERNAL_ERR);
    static ref DEP_OBJ_INHERITED: Regex =
        Regex::new(r#"^\s*['"]?([0-9A-Za-z-_]+)['"]?\s*=\s*\{.*['"]?workspace['"]?\s*=\s*true\s*.*}.*$"#)
            .expect(INTERNAL_ERR);
    static ref DEP_OBJ_RENAME_VERSION: Regex =
        Regex::new(r#"^(\s*['"]?([0-9A-Za-z-_]+)['"]?\s*=\s*\{.*['"]?version['"]?\s*=\s*['"])([^'"]+)(['"].*['"]?package['"]?\s*=\s*['"]([0-9A-Za-z-_]+)['"].*}.*)$"#)
            .expect(INTERNAL_ERR);
    static ref DEP_OBJ_RENAME_BEFORE_VERSION: Regex =
        Regex::new(r#"^(\s*['"]?[0-9A-Za-z-_]+['"]?\s*=\s*\{.*['"]?package['"]?\s*=\s*['"]([0-9A-Za-z-_]+)['"].*['"]?version['"]?\s*=\s*['"])([^'"]+)(['"].*}.*)$"#)
            .expect(INTERNAL_ERR);
    static ref DEP_DIRECT_NAME: Regex =
        Regex::new(r#"^(\s*['"]?([0-9A-Za-z-_]+)['"]?\s*=\s*)(['"][^'"]+['"])(.*)$"#)
            .expect(INTERNAL_ERR);
    static ref DEP_OBJ_NAME: Regex =
        Regex::new(r#"^(\s*['"]?([0-9A-Za-z-_]+)['"]?\s*=\s*\{(.*[^\s])?)(\s*}.*)$"#)
            .expect(INTERNAL_ERR);
    static ref DEP_OBJ_RENAME_NAME: Regex =
        Regex::new(r#"^(\s*['"]?[0-9A-Za-z-_]+['"]?\s*=\s*\{.*['"]?package['"]?\s*=\s*['"])([0-9A-Za-z-_]+)(['"].*}.*)$"#)
            .expect(INTERNAL_ERR);
    static ref WORKSPACE_KEY: Regex =
        Regex::new(r#"['"]?workspace['"]?\s*=\s*true"#).expect(INTERNAL_ERR);
}

pub fn cargo<'a>(
    root: &Utf8Path,
    args: &[&'a str],
    env: &[(&'a str, &'a str)],
) -> Result<(String, String)> {
    debug!("cargo", args.join(" "));

    let mut args = args.to_vec();

    if TERM_ERR.features().colors_supported() {
        args.push("--color");
        args.push("always");
    }

    if get_debug() {
        args.push("-v");
    }

    let args_text = args.iter().map(|x| x.to_string()).collect::<Vec<_>>();

    let mut stderr_lines = vec![];

    let mut child = Command::new("cargo")
        .current_dir(root)
        .args(&args)
        .envs(env.iter().copied())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| Error::Cargo {
            err,
            args: args_text.clone(),
        })?;

    {
        let stderr = child.stderr.as_mut().expect(INTERNAL_ERR);

        for line in BufReader::new(stderr).lines() {
            let line = line?;

            eprintln!("{}", line);
            stderr_lines.push(line);
        }
    }

    let output = child.wait_with_output().map_err(|err| Error::Cargo {
        err,
        args: args_text,
    })?;

    let output_stdout = String::from_utf8(output.stdout)?;
    let output_stderr = stderr_lines.join("\n");

    debug!("cargo stderr", output_stderr);
    debug!("cargo stdout", output_stdout);

    Ok((
        output_stdout.trim().to_owned(),
        output_stderr.trim().to_owned(),
    ))
}

pub fn cargo_config_get(root: &Utf8Path, name: &str) -> Result<String> {
    // You know how we sometimes have to make the best of an unfortunate
    // situation? This is one of those situations.
    //
    // In order to support private registries, we need to know the URL of their
    // index. That's stored in a `.cargo/config.toml` file, which could be in
    // the `root` directory, or someplace else, like `~/.cargo/config.toml`.
    //
    // In order to match cargo's lookup strategy, the best option is to use
    // `cargo config get`. However, that's unstable. Since we don't want
    // cargo-workspaces to require nightly, we can use two combined escape
    // hatches:
    //
    // 1. Set the `RUSTC_BOOTSTRAP` environment variable to `1`
    // 2. Pass `-Z unstable-options` to cargo
    //
    // This works because stable rustc versions contain exactly the same code as
    // nightly versions, but all the nightly features are gated. This allows
    // stable rustc versions to recognize nightly features and tell you: "no, you
    // need a nightly for this". But rustc should be able to compile rustc, and
    // the rustc codebase uses nightly features, so `RUSTC_BOOTSTRAP` removes that
    // gating.
    //
    // This is generally frowned upon (it's only supposed to be used to
    // bootstrap rustc), but here it's _just_ to get access to `cargo config`,
    // we're not actually building crates with
    // rustc-stable-masquerading-as-nightly.

    debug!("cargo config get", name);

    let args = vec!["-Z", "unstable-options", "config", "get", name];
    let env = &[("RUSTC_BOOTSTRAP", "1")];

    let (stdout, _) = cargo(root, &args, env)?;

    // `cargo config get` returns TOML output, like so:
    //
    //      $ RUSTC_BOOTSTRAP=1 cargo -Z unstable-options config get registries.foobar.index
    //      registries.foobar.index = "https://dl.cloudsmith.io/basic/some-org/foobar/cargo/index.git"
    //
    // The right thing to do is probably to pull in a TOML crate, but since the
    // output is so predictable, and in the interest of keeping dependencies low,
    // we just do some text wrangling instead:

    // tokens is ["registries.foobar.index", "\"some-url\""]
    let tokens = stdout
        .split(" = ")
        .map(|x| x.to_string())
        .collect::<Vec<_>>();

    // value is "\"some-url\""
    let value = tokens.get(1).ok_or(Error::BadConfigGetOutput(stdout))?;

    // we return "some-url"
    Ok(value
        .trim()
        .trim_start_matches('"')
        .trim_end_matches('"')
        .into())
}

#[derive(Debug)]
enum Context {
    Beginning,
    Package,
    Dependencies,
    DependencyEntry(String, Option<(usize, String)>, bool),
    DontCare,
}

fn edit_version(
    caps: Captures,
    new_lines: &mut Vec<String>,
    versions: &Map<String, Version>,
    exact: bool,
    version_index: usize,
) -> Result<()> {
    if let Some(new_version) = versions.get(&caps[version_index]) {
        if exact {
            new_lines.push(format!("{}={}{}", &caps[1], new_version, &caps[4]));
        } else if !VersionReq::parse(&caps[3])?.matches(new_version) {
            new_lines.push(format!("{}{}{}", &caps[1], new_version, &caps[4]));
        }
    }

    Ok(())
}

fn rename_dep(
    caps: Captures,
    new_lines: &mut Vec<String>,
    renames: &Map<String, String>,
    name_index: usize,
) -> Result<()> {
    if let Some(new_name) = renames.get(&caps[name_index]) {
        new_lines.push(format!("{}{}{}", &caps[1], new_name, &caps[3]));
    }

    Ok(())
}

fn parse<P, D, DE, DP>(
    manifest: String,
    dev_deps: bool,
    package_f: P,
    mut dependencies_f: D,
    dependency_entries_f: DE,
    mut dependency_pkg_f: DP,
) -> Result<String>
where
    P: Fn(&str, &mut Vec<String>) -> Result,
    D: FnMut(&str, &mut Vec<String>) -> Result,
    DE: Fn(&str, &mut Option<String>) -> (bool, Option<String>),
    DP: FnMut(&str, Option<(usize, String)>, &mut Vec<String>, bool) -> Result,
{
    let mut context = Context::Beginning;
    let mut new_lines = vec![];

    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if let Context::DependencyEntry(ref dep, ref mut dep_meta, inherits) = context {
                dependency_pkg_f(dep, dep_meta.take(), &mut new_lines, inherits)?;
            }
        }
        let count = new_lines.len();

        #[allow(clippy::if_same_then_else)]
        if let Some(_) = PACKAGE_TABLE.captures(trimmed) {
            context = Context::Package;
        } else if let Some(_) = DEP_TABLE.captures(trimmed) {
            context = Context::Dependencies;
        } else if let Some(_) = BUILD_DEP_TABLE.captures(trimmed) {
            context = Context::Dependencies;
        } else if let Some(_) = DEV_DEP_TABLE.captures(trimmed) {
            // TODO: let-chain
            if dev_deps {
                context = Context::Dependencies;
            } else {
                context = Context::DontCare;
            }
        } else if let Some(caps) = DEP_ENTRY.captures(trimmed) {
            context = Context::DependencyEntry(caps[1].to_string(), None, false);
        } else if let Some(caps) = BUILD_DEP_ENTRY.captures(trimmed) {
            context = Context::DependencyEntry(caps[1].to_string(), None, false);
        } else if let Some(caps) = DEV_DEP_ENTRY.captures(trimmed) {
            // TODO: let-chain
            if dev_deps {
                context = Context::DependencyEntry(caps[1].to_string(), None, false);
            } else {
                context = Context::DontCare;
            }
        } else if trimmed.starts_with('[') {
            context = Context::DontCare;
        } else {
            // TODO: Support `package.version` like stuff (with quotes) at beginning
            match &mut context {
                Context::Package => package_f(line, &mut new_lines)?,
                Context::Dependencies => dependencies_f(line, &mut new_lines)?,
                Context::DependencyEntry(dep, dep_meta, inherits) => {
                    let mut line_meta = None;

                    let (_inherits, new_dep) = dependency_entries_f(line, &mut line_meta);
                    *inherits |= _inherits;
                    if let Some(new_dep) = new_dep {
                        *dep = new_dep;
                    }
                    if let Some(meta) = line_meta {
                        dep_meta.replace((new_lines.len(), meta));
                    }
                }
                _ => {}
            }
        }

        if new_lines.len() == count {
            new_lines.push(line.to_string());
        }
    }

    if let Context::DependencyEntry(ref dep, dep_meta, inherits) = context {
        dependency_pkg_f(dep, dep_meta, &mut new_lines, inherits)?;
    }

    Ok(new_lines.join(if manifest.contains(CRLF) { CRLF } else { LF }))
}

pub fn rename_packages(
    manifest: String,
    pkg_name: &str,
    renames: &Map<String, String>,
) -> Result<String> {
    parse(
        manifest,
        true,
        |line, new_lines| {
            if let Some(to) = renames.get(pkg_name) {
                if let Some(caps) = NAME.captures(line) {
                    new_lines.push(format!("{}{}{}", &caps[1], to, &caps[3]));
                }
            }

            Ok(())
        },
        |line, new_lines| {
            if let Some(caps) = DEP_DIRECT_NAME.captures(line) {
                if let Some(new_name) = renames.get(&caps[2]) {
                    new_lines.push(format!(
                        "{}{{ version = {}, package = \"{}\" }}{}",
                        &caps[1], &caps[3], new_name, &caps[4]
                    ));
                }
            } else if let Some(caps) = DEP_OBJ_RENAME_NAME.captures(line) {
                rename_dep(caps, new_lines, renames, 2)?;
            } else if let Some(caps) = DEP_OBJ_NAME.captures(line) {
                if let Some(new_name) = renames.get(&caps[2]) {
                    if WORKSPACE_KEY.captures(&caps[3]).is_none() {
                        new_lines.push(format!(
                            "{}, package = \"{}\"{}",
                            &caps[1], new_name, &caps[4]
                        ));
                    }
                }
            }

            Ok(())
        },
        |line, package_line| {
            if PACKAGE.is_match(line) {
                package_line.replace(line.to_string());
            }

            (false, None)
        },
        |dep, package_line, new_lines, _| {
            match package_line {
                Some((i, line)) => {
                    if let (Some(line), Some(caps)) =
                        (new_lines.get_mut(i), PACKAGE.captures(&line))
                    {
                        if let Some(new_name) = renames.get(&caps[2]) {
                            *line = format!("{}{}{}", &caps[1], new_name, &caps[3]);
                        }
                    }
                }
                None => {
                    if let Some(new_name) = renames.get(dep) {
                        new_lines.push(format!("package = \"{}\"", new_name));
                    }
                }
            }

            Ok(())
        },
    )
}

pub fn change_versions(
    manifest: String,
    pkg_name: &str,
    versions: &Map<String, Version>,
    exact: bool,
    inherited: &mut HashSet<String>,
) -> Result<String> {
    let inherited = Rc::new(RefCell::new(inherited));
    parse(
        manifest,
        false,
        |line, new_lines| {
            if let Some(new_version) = versions.get(pkg_name) {
                if let Some(caps) = VERSION.captures(line) {
                    new_lines.push(format!("{}{}{}", &caps[1], new_version, &caps[3]));
                }
            }

            Ok(())
        },
        |line, new_lines| {
            if let Some(caps) = DEP_DIRECT_INHERITED.captures(line) {
                inherited.borrow_mut().insert(caps[1].to_string());
            } else if let Some(caps) = DEP_OBJ_INHERITED.captures(line) {
                inherited.borrow_mut().insert(caps[1].to_string());
            } else if let Some(caps) = DEP_DIRECT_VERSION.captures(line) {
                edit_version(caps, new_lines, versions, exact, 2)?;
            } else if let Some(caps) = DEP_OBJ_RENAME_VERSION.captures(line) {
                edit_version(caps, new_lines, versions, exact, 5)?;
            } else if let Some(caps) = DEP_OBJ_RENAME_BEFORE_VERSION.captures(line) {
                edit_version(caps, new_lines, versions, exact, 2)?;
            } else if let Some(caps) = DEP_OBJ_VERSION.captures(line) {
                edit_version(caps, new_lines, versions, exact, 2)?;
            } else if let Some(caps) = DEP_OBJ_NAME.captures(line) {
                if let Some(new_version) = versions.get(&caps[2]) {
                    if exact {
                        new_lines.push(format!(
                            "{}, version = \"={}\"{}",
                            &caps[1], new_version, &caps[4]
                        ));
                    } else {
                        new_lines.push(format!(
                            "{}, version = \"{}\"{}",
                            &caps[1], new_version, &caps[4]
                        ));
                    }
                }
            }

            Ok(())
        },
        |line, version_line| {
            if let Some(_) = WORKSPACE_KEY.captures(line) {
                return (true, None);
            } else if let Some(caps) = PACKAGE.captures(line) {
                return (false, Some(caps[2].to_string()));
            } else if VERSION.is_match(line) {
                version_line.replace(line.to_string());
            }

            (false, None)
        },
        |dep, version_line, new_lines, inherits| {
            if inherits {
                inherited.borrow_mut().insert(dep.to_string());
            } else if let Some((i, line)) = version_line {
                if let (Some(line), Some(caps), Some(new_version)) = (
                    new_lines.get_mut(i),
                    VERSION.captures(&line),
                    versions.get(dep),
                ) {
                    if exact {
                        *line = format!("{}={}{}", &caps[1], new_version, &caps[3]);
                    } else if !VersionReq::parse(&caps[2])?.matches(new_version) {
                        *line = format!("{}{}{}", &caps[1], new_version, &caps[3]);
                    }
                }
            } else {
                if let Some(new_version) = versions.get(dep) {
                    new_lines.push(format!("version = \"{}\"", new_version));
                }
            }

            Ok(())
        },
    )
}

pub trait VersionSpec {
    fn is_unversioned(other: &Self) -> bool;
}

impl VersionSpec for Version {
    fn is_unversioned(other: &Self) -> bool {
        matches!(
            other,
            Version {
                major: 0,
                minor: 0,
                patch: 0,
                ..
            }
            if other.pre.is_empty() && other.build.is_empty()
        )
    }
}

impl VersionSpec for VersionReq {
    fn is_unversioned(other: &Self) -> bool {
        other == &VersionReq::parse(">=0.0.0").unwrap() || other == &VersionReq::any()
    }
}

pub fn is_unversioned(v: &impl VersionSpec) -> bool {
    VersionSpec::is_unversioned(v)
}

pub fn is_published(index: &mut Index, name: &str, version: &str) -> Result<bool> {
    // See if we already have the crate (and version) in cache
    if let Some(crate_data) = index.crate_(name) {
        if crate_data.versions().iter().any(|v| v.version() == version) {
            return Ok(true);
        }
    }

    // We don't? Okay, update the cache then
    index.update()?;

    // Try again with updated index:
    if let Some(crate_data) = index.crate_(name) {
        if crate_data.versions().iter().any(|v| v.version() == version) {
            return Ok(true);
        }
    }

    // I guess we didn't have it
    Ok(false)
}

pub fn check_index(index: &mut Index, name: &str, version: &str) -> Result<()> {
    let now = Instant::now();
    let sleep_time = Duration::from_secs(2);
    let timeout = Duration::from_secs(300);
    let mut logged = false;

    loop {
        if is_published(index, name, version)? {
            break;
        } else if timeout < now.elapsed() {
            return Err(Error::PublishTimeout);
        }

        if !logged {
            info!("waiting", "...");
            logged = true;
        }

        sleep(sleep_time);
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use indoc::indoc;

    #[test]
    fn test_version() {
        let m = indoc! {r#"
            [package]
            version = "0.1.0"
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "this", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [package]
                version = "0.3.0""#
            }
        );
    }

    #[test]
    fn test_version_comments() {
        let m = indoc! {r#"
            [package]
            version="0.1.0" # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "this", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [package]
                version="0.3.0" # hello"#
            }
        );
    }

    #[test]
    fn test_version_quotes() {
        let m = indoc! {r#"
            [package]
            "version"	=	"0.1.0"
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "this", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [package]
                "version"	=	"0.3.0""#
            }
        );
    }

    #[test]
    fn test_version_single_quotes() {
        let m = indoc! {r#"
            [package]
            'version'='0.1.0'# hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "this", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [package]
                'version'='0.3.0'# hello"#
            }
        );
    }

    #[test]
    fn test_version_workspace() {
        let m = indoc! {r#"
            [workspace.package]
            version = "0.0.1" # hello
        "#};

        let mut v = Map::new();
        v.insert("<workspace>".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "<workspace>", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [workspace.package]
                version = "0.3.0" # hello"#
            }
        );
    }

    #[test]
    fn test_version_dependencies() {
        let m = indoc! {r#"
            [dependencies]
            this = "0.0.1" # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "another", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [dependencies]
                this = "0.3.0" # hello"#
            }
        );
    }

    #[test]
    fn test_missing_version_dependencies_object() {
        let m = indoc! {r#"
            [dependencies]
            this = { path = "../" } # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "another", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [dependencies]
                this = { path = "../", version = "0.3.0" } # hello"#
            }
        );
    }

    #[test]
    fn test_missing_version_dependencies_object_renamed() {
        let m = indoc! {r#"
            [dependencies]
            this = { path = "../", package = "ra_this" } # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "another", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [dependencies]
                this = { path = "../", package = "ra_this", version = "0.3.0" } # hello"#
            }
        );
    }

    #[test]
    fn test_version_dependencies_object() {
        let m = indoc! {r#"
            [dependencies]
            this = { path = "../", version = "0.0.1" } # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "another", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [dependencies]
                this = { path = "../", version = "0.3.0" } # hello"#
            }
        );
    }

    #[test]
    fn test_version_dependencies_object_renamed() {
        let m = indoc! {r#"
            [dependencies]
            this2 = { path = "../", version = "0.0.1", package = "this" } # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "another", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [dependencies]
                this2 = { path = "../", version = "0.3.0", package = "this" } # hello"#
            }
        );
    }

    #[test]
    fn test_version_dependencies_object_renamed_before_version() {
        let m = indoc! {r#"
            [dependencies]
            this2 = { path = "../", package = "this", version = "0.0.1" } # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "another", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [dependencies]
                this2 = { path = "../", package = "this", version = "0.3.0" } # hello"#
            }
        );
    }

    #[test]
    fn test_version_dependency_table() {
        let m = indoc! {r#"
            [dependencies.this]
            path = "../"
            version = "0.0.1" # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "another", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [dependencies.this]
                path = "../"
                version = "0.3.0" # hello"#
            }
        );
    }

    #[test]
    fn test_version_dependency_table_ignore_workspace() {
        let m = indoc! {r#"
            [dependencies.this]
            path = "../"
            workspace = true

            [dependencies.other]
            path = "../"
            workspace = true

            [dev-dependencies.dev-this]
            path = "../"
            workspace = true

            [dev-dependencies.dev-other]
            path = "../"
            workspace = true
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        let mut inherited = HashSet::new();

        assert_eq!(
            change_versions(m.into(), "another", &v, false, &mut inherited).unwrap(),
            indoc! {r#"
                [dependencies.this]
                path = "../"
                workspace = true

                [dependencies.other]
                path = "../"
                workspace = true

                [dev-dependencies.dev-this]
                path = "../"
                workspace = true

                [dev-dependencies.dev-other]
                path = "../"
                workspace = true"#
            }
        );

        assert_eq!(inherited.len(), 2);
        assert!(inherited.contains("this"));
        assert!(inherited.contains("other"));
    }

    #[test]
    fn test_version_dependency_table_missing_version() {
        let m = indoc! {r#"
            [dependencies.this]
            path = "../" # hello
            [package]
            name = "test"
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "this", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [dependencies.this]
                path = "../" # hello
                version = "0.3.0"
                [package]
                name = "test""#}
        );
    }

    #[test]
    fn test_dependency_table_renamed() {
        let m = indoc! {r#"
            [dependencies.this2]
            path = "../"
            version = "0.0.1" # hello"
            package = "this"
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "this", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [dependencies.this2]
                path = "../"
                version = "0.3.0" # hello"
                package = "this""#
            }
        );
    }

    #[test]
    fn test_version_dependency_table_renamed_before_version() {
        let m = indoc! {r#"
            [dependencies.this2]
            path = "../"
            package = "this"
            version = "0.0.1" # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "another", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [dependencies.this2]
                path = "../"
                package = "this"
                version = "0.3.0" # hello"#
            }
        );
    }

    #[test]
    fn test_version_target_dependencies() {
        let m = indoc! {r#"
            [target.x86_64-pc-windows-gnu.dependencies]
            this = "0.0.1" # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "another", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [target.x86_64-pc-windows-gnu.dependencies]
                this = "0.3.0" # hello"#
            }
        );
    }

    #[test]
    fn test_version_target_cfg_dependencies() {
        let m = indoc! {r#"
            [target.'cfg(not(any(target_arch = "wasm32", target_os = "emscripten")))'.dependencies]
            this = "0.0.1" # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "another", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [target.'cfg(not(any(target_arch = "wasm32", target_os = "emscripten")))'.dependencies]
                this = "0.3.0" # hello"#
            }
        );
    }

    #[test]
    fn test_version_ignore_workspace() {
        let m = indoc! {r#"
            [dependencies]
            this = { workspace = true } # hello
            other = { workspace= true } # hello

            [dev-dependencies]
            dev-this = { workspace = true } # hello
            dev-other = { workspace= true } # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        let mut inherited = HashSet::new();

        assert_eq!(
            change_versions(m.into(), "another", &v, false, &mut inherited).unwrap(),
            indoc! {r#"
                [dependencies]
                this = { workspace = true } # hello
                other = { workspace= true } # hello

                [dev-dependencies]
                dev-this = { workspace = true } # hello
                dev-other = { workspace= true } # hello"#
            }
        );

        assert_eq!(inherited.len(), 2);
        assert!(inherited.contains("this"));
        assert!(inherited.contains("other"));
    }

    #[test]
    fn test_version_workspace_dependencies() {
        let m = indoc! {r#"
            [workspace.dependencies]
            this = "0.0.1" # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "another", &v, false, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [workspace.dependencies]
                this = "0.3.0" # hello"#
            }
        );
    }

    #[test]
    fn test_version_ignore_dotted_workspace() {
        let m = indoc! {r#"
            [dependencies]
            this.workspace = true # hello
            other.workspace=true# hello

            [dev-dependencies]
            dev-this.workspace = true # hello
            dev-other.workspace=true# hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        let mut inherited = HashSet::new();

        assert_eq!(
            change_versions(m.into(), "another", &v, false, &mut inherited).unwrap(),
            indoc! {r#"
                [dependencies]
                this.workspace = true # hello
                other.workspace=true# hello

                [dev-dependencies]
                dev-this.workspace = true # hello
                dev-other.workspace=true# hello"#
            }
        );

        assert_eq!(inherited.len(), 2);
        assert!(inherited.contains("this"));
        assert!(inherited.contains("other"));
    }

    #[test]
    fn test_exact() {
        let m = indoc! {r#"
            [dependencies]
            this = { path = "../", version = "0.0.1" } # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "another", &v, true, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [dependencies]
                this = { path = "../", version = "=0.3.0" } # hello"#
            }
        );
    }

    #[test]
    fn test_exact_version_missing() {
        let m = indoc! {r#"
            [dependencies]
            this = { path = "../" } # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), Version::parse("0.3.0").unwrap());

        assert_eq!(
            change_versions(m.into(), "this", &v, true, &mut HashSet::new()).unwrap(),
            indoc! {r#"
                [dependencies]
                this = { path = "../", version = "=0.3.0" } # hello"#
            }
        );
    }

    #[test]
    fn test_name() {
        let m = indoc! {r#"
            [package]
            name = "this"
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), "ra_this".to_string());

        assert_eq!(
            rename_packages(m.into(), "this", &v).unwrap(),
            indoc! {r#"
                [package]
                name = "ra_this""#
            }
        );
    }

    #[test]
    fn test_name_dependencies() {
        let m = indoc! {r#"
            [dependencies]
            this = "0.0.1" # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), "ra_this".to_string());

        assert_eq!(
            rename_packages(m.into(), "another", &v).unwrap(),
            indoc! {r#"
                [dependencies]
                this = { version = "0.0.1", package = "ra_this" } # hello"#
            }
        );
    }

    #[test]
    fn test_name_dependencies_object() {
        let m = indoc! {r#"
            [dependencies]
            this = { path = "../", version = "0.0.1" } # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), "ra_this".to_string());

        assert_eq!(
            rename_packages(m.into(), "another", &v).unwrap(),
            indoc! {r#"
                [dependencies]
                this = { path = "../", version = "0.0.1", package = "ra_this" } # hello"#
            }
        );
    }

    #[test]
    fn test_name_dependencies_object_renamed() {
        let m = indoc! {r#"
            [dependencies]
            this2 = { path = "../", version = "0.0.1", package = "this" } # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), "ra_this".to_string());

        assert_eq!(
            rename_packages(m.into(), "another", &v).unwrap(),
            indoc! {r#"
                [dependencies]
                this2 = { path = "../", version = "0.0.1", package = "ra_this" } # hello"#
            }
        );
    }

    #[test]
    fn test_name_dependencies_object_renamed_before_version() {
        let m = indoc! {r#"
            [dependencies]
            this2 = { path = "../", package = "this", version = "0.0.1" } # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), "ra_this".to_string());

        assert_eq!(
            rename_packages(m.into(), "another", &v).unwrap(),
            indoc! {r#"
                [dependencies]
                this2 = { path = "../", package = "ra_this", version = "0.0.1" } # hello"#
            }
        );
    }

    #[test]
    fn test_name_dependency_table() {
        let m = indoc! {r#"
            [dependencies.this]
            path = "../"
            version = "0.0.1" # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), "ra_this".to_string());

        assert_eq!(
            rename_packages(m.into(), "another", &v).unwrap(),
            indoc! {r#"
                [dependencies.this]
                path = "../"
                version = "0.0.1" # hello
                package = "ra_this""#
            }
        );
    }

    #[test]
    fn test_name_dependency_table_renamed() {
        let m = indoc! {r#"
            [dependencies.this2]
            path = "../"
            version = "0.0.1" # hello"
            package = "this"
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), "ra_this".to_string());

        assert_eq!(
            rename_packages(m.into(), "another", &v).unwrap(),
            indoc! {r#"
                [dependencies.this2]
                path = "../"
                version = "0.0.1" # hello"
                package = "ra_this""#
            }
        );
    }

    #[test]
    fn test_name_dependency_table_renamed_before_version() {
        let m = indoc! {r#"
            [dependencies.this2]
            path = "../"
            package = "this"
            version = "0.0.1" # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), "ra_this".to_string());

        assert_eq!(
            rename_packages(m.into(), "another", &v).unwrap(),
            indoc! {r#"
                [dependencies.this2]
                path = "../"
                package = "ra_this"
                version = "0.0.1" # hello"#
            }
        );
    }

    #[test]
    fn test_name_target_dependencies() {
        let m = indoc! {r#"
            [target.x86_64-pc-windows-gnu.dependencies]
            this = "0.0.1" # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), "ra_this".to_string());

        assert_eq!(
            rename_packages(m.into(), "another", &v).unwrap(),
            indoc! {r#"
                [target.x86_64-pc-windows-gnu.dependencies]
                this = { version = "0.0.1", package = "ra_this" } # hello"#
            }
        );
    }

    #[test]
    fn test_name_target_cfg_dependencies() {
        let m = indoc! {r#"
            [target.'cfg(not(any(target_arch = "wasm32", target_os = "emscripten")))'.dependencies]
            this = "0.0.1" # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), "ra_this".to_string());

        assert_eq!(
            rename_packages(m.into(), "another", &v).unwrap(),
            indoc! {r#"
                [target.'cfg(not(any(target_arch = "wasm32", target_os = "emscripten")))'.dependencies]
                this = { version = "0.0.1", package = "ra_this" } # hello"#
            }
        );
    }

    #[test]
    fn test_name_ignore_workspace() {
        let m = indoc! {r#"
            [dependencies]
            this = { workspace = true } # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), "ra_this".to_string());

        assert_eq!(
            rename_packages(m.into(), "another", &v).unwrap(),
            indoc! {r#"
                [dependencies]
                this = { workspace = true } # hello"#
            }
        );
    }

    #[test]
    fn test_name_ignore_workspace_with_keys() {
        let m = indoc! {r#"
            [dependencies]
            this = { workspace = true, optional = true } # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), "ra_this".to_string());

        assert_eq!(
            rename_packages(m.into(), "another", &v).unwrap(),
            indoc! {r#"
                [dependencies]
                this = { workspace = true, optional = true } # hello"#
            }
        );
    }

    #[test]
    fn test_name_ignore_dotted_workspace() {
        let m = indoc! {r#"
            [dependencies]
            this.workspace = true # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), "ra_this".to_string());

        assert_eq!(
            rename_packages(m.into(), "another", &v).unwrap(),
            indoc! {r#"
                [dependencies]
                this.workspace = true # hello"#
            }
        );
    }

    #[test]
    fn test_name_workspace_dependencies() {
        let m = indoc! {r#"
            [workspace.dependencies]
            this = "0.0.1" # hello
        "#};

        let mut v = Map::new();
        v.insert("this".to_string(), "ra_this".to_string());

        assert_eq!(
            rename_packages(m.into(), "another", &v).unwrap(),
            indoc! {r#"
                [workspace.dependencies]
                this = { version = "0.0.1", package = "ra_this" } # hello"#
            }
        );
    }
}
