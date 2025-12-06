# Rust In-Process Plugin Example

This directory contains a minimal example of an in-process plugin
written in Rust as a `cdylib` dynamic library.

## Note
This example is intended for demonstration and testing purposes only. Removing it will cause [`load_inprocess_plugin`](../core/tests/load_inprocess_plugin.rs) test to fail.

If you are looking to create your own in-process plugin, refer to the
[in-process plugin guide](../docs/INPROCESS_PLUGIN.md) in the `docs` folder for detailed instructions on the required ABI and usage.