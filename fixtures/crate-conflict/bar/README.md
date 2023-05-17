# Crate Conflict

This crate is a test for the capacity for cargo-workspaces to version a workspace with the following features:

- Presence of a public crate and local crate with the same name

## Logs

```console
$ cd fixtures/crate-conflict
$ cargo ws version --no-git
✔ Select a new version for the workspace (currently 0.1.0) · Major (1.0.0)

Changes:
 (current common version: 0.1.0)
 - bar: 0.1.0 => 1.0.0
 - foo: 0.1.0 => 1.0.0
 - syn: 0.1.0 => 1.0.0

✔ Are you sure you want to create these versions? · yes
    Updating crates.io index
    Updating bar v0.1.1 (/cargo-workspaces/fixtures/crate-conflict/bar) -> v1.0.0
    Updating foo v0.1.1 (/cargo-workspaces/fixtures/crate-conflict/foo) -> v1.0.0
    Updating syn v2.0.15 (/cargo-workspaces/fixtures/crate-conflict/syn) -> v1.0.0
info success ok
```
