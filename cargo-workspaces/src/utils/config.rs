use crate::utils;

use semver::Version;
use serde::{de, Deserialize};
use serde_json::{from_value, Value};

use std::{fmt, path::Path};

#[derive(Deserialize, Default)]
struct MetadataWorkspaces<T> {
    pub workspaces: Option<T>,
}

// TODO: Validation of conflicting options (hard to tell conflicts if between cli and option)
pub fn read_config<T>(value: &Value) -> utils::Result<T>
where
    T: for<'de> Deserialize<'de> + Default,
{
    from_value::<Option<MetadataWorkspaces<T>>>(value.clone())
        .map_err(utils::Error::BadMetadata)
        .map(|v| v.unwrap_or_default().workspaces.unwrap_or_default())
}

#[derive(Deserialize, Default, Debug, Clone, Ord, Eq, PartialOrd, PartialEq)]
pub struct PackageConfig {
    pub independent: Option<bool>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceGroupSpec {
    #[serde(deserialize_with = "validate_group_name")]
    pub name: String,
    pub version: Option<Version>,
    #[serde(deserialize_with = "deserialize_members")]
    pub members: Vec<GroupMember>,
}

#[derive(Deserialize, Debug)]
#[serde(transparent, deny_unknown_fields)]
pub struct ExcludeSpec {
    #[serde(deserialize_with = "deserialize_members")]
    pub members: Vec<GroupMember>,
}

pub struct GroupMember {
    pub pattern: glob::Pattern,
    paths_fn: Box<dyn Fn() -> glob::Paths>,
}

impl fmt::Debug for GroupMember {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("GroupMember").field(&self.pattern).finish()
    }
}

impl GroupMember {
    pub fn matches(&self, path: &Path) -> bool {
        if let Ok(path) = path.canonicalize() {
            for entry in (self.paths_fn)() {
                if let Ok(entry) = entry {
                    if let Ok(entry) = entry.canonicalize() {
                        if entry == path {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

#[derive(Deserialize, Default, Debug)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    pub version: Option<Version>,
    pub exclude: Option<ExcludeSpec>,
    #[serde(rename = "group")]
    pub groups: Vec<WorkspaceGroupSpec>,
    pub allow_branch: Option<String>,
    pub no_individual_tags: Option<bool>,
}

fn validate_group_name<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: de::Deserializer<'de>,
{
    let group_name = String::deserialize(deserializer)?;
    utils::GroupName::validate(&group_name).map_err(de::Error::custom)?;
    if matches!(group_name.as_str(), "excluded" | "default") {
        return Err(de::Error::custom(format!(
            "invalid use of reserved group name: {}",
            group_name
        )));
    };
    Ok(group_name)
}

fn deserialize_members<'de, D>(deserializer: D) -> Result<Vec<GroupMember>, D::Error>
where
    D: de::Deserializer<'de>,
{
    struct MembersVisitor;

    impl<'de> de::Visitor<'de> for MembersVisitor {
        type Value = Vec<GroupMember>;

        fn expecting(&self, fmt: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
            fmt.write_str("a list of glob patterns matching paths to workspace members")
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut vec = Vec::with_capacity(seq.size_hint().unwrap_or(0));

            while let Some(elem) = seq.next_element::<String>()? {
                vec.push(GroupMember {
                    pattern: glob::Pattern::new(&elem).map_err(de::Error::custom)?,
                    paths_fn: Box::new(move || glob::glob(&elem).expect(utils::INTERNAL_ERR)),
                });
            }

            Ok(vec)
        }
    }
    deserializer.deserialize_seq(MembersVisitor)
}
