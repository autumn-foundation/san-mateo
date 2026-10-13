# ADR 0001: Clean-room code and the Nango catalog format

- Status: Accepted
- Date: 2026-10-13

## Context

Nango uses the Elastic License 2.0 (ELv2). ELv2 does not let a person give the
software to others as a hosted service. San Mateo uses Apache-2.0. Nango has a
catalog of 1,048 providers in one YAML file. That file is part of the ELv2
repository.

## Options

1. Port the Nango TypeScript code to Rust.
2. Copy `providers.yaml` into San Mateo.
3. Write new code. Read the YAML format. Write a new built-in catalog from
   public provider docs.

## Decision

Option 3.

## Reasons

- Options 1 and 2 copy ELv2 work into an Apache-2.0 crate.
- A file format and an HTTP API shape are interfaces. San Mateo reads the
  format so users can bring their own catalog file.
- The built-in catalog comes from public provider docs. It has a `docs` link
  for each provider.

## Results

- Good: no ELv2 code or data in the repository.
- Good: `Catalog::from_yaml_file` reads a Nango-format file that a user has.
- Bad: the built-in catalog is small (about 35 providers). Users add more in
  their own YAML file.
