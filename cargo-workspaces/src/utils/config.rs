use crate::utils;

use glob::Pattern;
use serde::{de, Deserialize};
use serde_json::{from_value, Value};

use std::fmt;

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

#[derive(Deserialize, Debug, Clone, Ord, Eq, PartialOrd, PartialEq)]
pub struct WorkspaceGroupSpec {
    #[serde(deserialize_with = "validate_group_name")]
    pub name: String,
    #[serde(deserialize_with = "deserialize_members")]
    pub members: Vec<Pattern>,
}

#[derive(Deserialize, Debug, Clone, Ord, Eq, PartialOrd, PartialEq)]
#[serde(transparent)]
pub struct ExcludeSpec {
    #[serde(deserialize_with = "deserialize_members")]
    pub members: Vec<Pattern>,
}

#[derive(Deserialize, Default, Debug, Clone, Ord, Eq, PartialOrd, PartialEq)]
pub struct WorkspaceConfig {
    pub exclude: Option<ExcludeSpec>,
    pub group: Option<Vec<WorkspaceGroupSpec>>,
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

fn deserialize_members<'de, D>(deserializer: D) -> Result<Vec<Pattern>, D::Error>
where
    D: de::Deserializer<'de>,
{
    struct MembersVisitor;

    impl<'de> de::Visitor<'de> for MembersVisitor {
        type Value = Vec<Pattern>;

        fn expecting(&self, fmt: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
            fmt.write_str("a list of glob patterns matching paths to workspace members")
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut vec = Vec::with_capacity(seq.size_hint().unwrap_or(0));

            while let Some(elem) = seq.next_element::<String>()? {
                vec.push(Pattern::new(&elem).map_err(de::Error::custom)?);
            }

            Ok(vec)
        }
    }
    deserializer.deserialize_seq(MembersVisitor)
}
