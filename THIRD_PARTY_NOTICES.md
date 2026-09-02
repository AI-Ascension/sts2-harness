# Third-Party Notices

This foundation contains no game binaries, host assemblies, model weights, datasets, provider SDKs,
or copied implementation source.

The target-local repository policy tool uses the Cargo dependency `toml` as declared in the workspace
manifest and locked in `Cargo.lock`. Its upstream license and transitive notices are resolved from
the pinned Cargo package metadata and must be rechecked by release tooling before distribution.
Dependencies are not vendored into this repository.

Future dependencies, imported fixtures, generated schemas, provider adapters, and model artifacts
must record their source, exact version or digest, license, redistribution permission, and retention
status before they become release inputs. Unknown or incompatible provenance is a release blocker.
