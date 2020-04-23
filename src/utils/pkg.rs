use crate::utils::{Listable, Writer};
use serde::Serialize;
use std::{cmp::max, io::Result, path::PathBuf};

#[derive(Serialize, Ord, Eq, PartialOrd, PartialEq)]
pub struct Pkg {
    pub name: String,
    pub version: String,
    pub location: PathBuf,
    #[serde(skip)]
    pub path: String,
    pub private: bool,
}

impl Listable for Vec<Pkg> {
    fn list(&self, w: &mut Writer, json: bool, long: bool, all: bool) -> Result<()> {
        if json {
            return self.json(w);
        }

        let first = self.iter().map(|x| x.name.len()).max().unwrap();
        let second = self.iter().map(|x| x.version.len() + 1).max().unwrap();
        let third = self.iter().map(|x| max(1, x.path.len())).max().unwrap();

        for pkg in self {
            w.none(&pkg.name)?;
            let mut width = first - pkg.name.len();

            if long {
                w.none(&format!("{:w$} ", "", w = width))?;
                w.green(&format!("v{}", pkg.version))?;
                w.none(&format!("{:w$} ", "", w = second - pkg.version.len() - 1))?;

                if pkg.path.is_empty() {
                    w.br_black(".")?;
                } else {
                    w.br_black(&pkg.path)?;
                }

                width = third - pkg.path.len();
            }

            if all && pkg.private {
                w.none(&format!("{:w$} (", "", w = width))?;
                w.red("PRIVATE")?;
                w.none(")")?;
            }

            w.none("\n")?;
        }

        Ok(())
    }
}
