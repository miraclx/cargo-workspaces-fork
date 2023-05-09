use crate::utils::{GroupName, Result};

use clap::Parser;
use oclif::term::TERM_OUT;
use serde::Serialize;

pub trait Listable: Serialize {
    fn json(&self) -> Result {
        TERM_OUT.write_line(&serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    fn list(&self, list: ListOpt) -> Result;
}

#[derive(Debug, Parser)]
pub struct ListOpt {
    /// Show extended information
    #[clap(short, long)]
    pub long: bool,

    /// Show private crates that are normally hidden
    #[clap(short, long)]
    pub all: bool,

    /// Show information as a JSON array
    #[clap(long, conflicts_with = "long")]
    pub json: bool,

    /// Comma separated list of crate groups to deal with
    #[clap(
        long,
        multiple_occurrences = true,
        use_value_delimiter = true,
        number_of_values = 1
    )]
    pub groups: Vec<GroupName>,
}
