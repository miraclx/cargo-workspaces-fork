use crate::utils;

use glob::Pattern;
use serde::{de, Deserialize};
use serde_json::{from_value, Value};

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
pub struct PackageGroup {
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
    pub group: Option<Vec<PackageGroup>>,
    pub allow_branch: Option<String>,
    pub no_individual_tags: Option<bool>,
}

fn deserialize_members<'de, D>(deserializer: D) -> Result<Vec<Pattern>, D::Error>
where
    D: de::Deserializer<'de>,
{
    Vec::<String>::deserialize(deserializer)?
        .into_iter()
        .map(|s| Pattern::new(&s))
        .collect::<Result<_, _>>()
        .map_err(de::Error::custom)
}
