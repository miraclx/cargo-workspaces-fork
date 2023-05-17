# Self-inherited

This crate is a test for the capacity for cargo-workspaces to version a workspace with the following features:

- Workspace inheritance
- Root as a package
- Presence of a public crate and local crate with the same name

## Logs

```console
$ cd fixtures/self-inherited
$ cargo ws version --no-git
✔ Select a new version for the workspace (currently 0.1.0) · Patch (0.1.1)
✔ Select a new version for the group `foo-and-bar` (currently 0.1.0) · Minor (0.2.0)
✔ You have 4 packages with unversioned dependencies · Auto-version

Changes:
 (current common version: 0.1.0)
 - foobar: 0.1.0 => 0.1.1
 - foobard: 0.1.0 => 0.1.1
 - libcommon: 0.1.0 => 0.1.1
 - syn: 0.1.0 => 0.1.1
 [foo-and-bar] (current common version: 0.1.0)
 - bar: 0.1.0 => 0.2.0
 - foo: 0.1.0 => 0.2.0

✔ Are you sure you want to create these versions? · yes
    Updating bar v1.0.0 (/cargo-workspaces/fixtures/self-inherited/crates/bar) -> v0.2.0
    Updating foo v1.0.0 (/cargo-workspaces/fixtures/self-inherited/crates/foo) -> v0.2.0
    Updating foobar v0.2.0 (/cargo-workspaces/fixtures/self-inherited) -> v0.1.1
    Updating foobard v0.2.0 (/cargo-workspaces/fixtures/self-inherited/crates/foobard) -> v0.1.1
    Updating libcommon v0.2.0 (/cargo-workspaces/fixtures/self-inherited/crates/common) -> v0.1.1
    Updating syn v0.2.0 (/cargo-workspaces/fixtures/self-inherited/crates/syn) -> v0.1.1
    Updating crates.io index
    Updating crates.io index
info success ok
```
