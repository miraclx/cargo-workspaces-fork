# Inheritance

This crate is a test for the capacity for cargo-workspaces to version a workspace with the following features:

- Workspace inheritance

## Logs

```console
$ cd fixtures/inheritance
$ cargo ws version --no-git
✔ Select a new version for the workspace (currently 0.1.0) · Minor (0.2.0)
✔ You have 3 packages with unversioned dependencies · Auto-version

Changes:
 (current common version: 0.1.0)
 - bar: 0.1.0 => 0.2.0
 - foo: 0.1.0 => 0.2.0
 - foobar: 0.1.0 => 0.2.0
 - libcommon: 0.1.0 => 0.2.0

✔ Are you sure you want to create these versions? · yes
    Updating bar v0.1.1 (/cargo-workspaces/fixtures/inheritance/crates/bar) -> v0.2.0
    Updating foo v0.1.1 (/cargo-workspaces/fixtures/inheritance/crates/foo) -> v0.2.0
    Updating foobar v0.1.1 (/cargo-workspaces/fixtures/inheritance/crates/foobar) -> v0.2.0
    Updating libcommon v0.1.1 (/cargo-workspaces/fixtures/inheritance/crates/common) -> v0.2.0
info success ok
```
