The `data.json` file contains a list of crates that are known to be incompatible
with wasix. The structure is defined in the `IncompatibleCrate` type in the
code.

## Backwards Compatibility

Be careful when changing the structure of the data as previous version of
cargo-wasix will use the same data. Any changes made should be backwards
compatible.
