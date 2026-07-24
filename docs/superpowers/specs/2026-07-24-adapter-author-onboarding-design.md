# Adapter Author Onboarding Design

## Goal

Make the current Input and Output Adapter authoring paths accurate, copyable, and
protected against documentation drift without changing the compile-time
composition architecture.

## Output Adapter documentation

The English and Japanese normative Output Adapter v1 contracts will use the exact
public Rust names and signatures from `iotkit-output-adapter-api`: `Descriptor`,
`Mode`, `Observation`, `OutputAdapter`, `MqttPublication`, and
`AdapterError`. The documents will link the API crate, compile-tested example,
and shared testkit, and will retain the current compile-time registry and
Console behavior.

A lightweight Node source guard will reject the stale Go-shaped declarations
and error names that previously appeared in the current English contract. It
will also require the Rust trait and error vocabulary plus links to the example
and testkit.

## Input Adapter authoring path

The existing provider-neutral `ReferenceAdapter` in
`iotkit-input-adapter-testkit` will be promoted from an observation fixture to a
complete test-only author model. It will expose a descriptor, typed configuration
validation, and a `start` method that returns the real `RunningInputAdapter`.
The lifecycle test will exercise descriptor validation, rejected invalid
configuration, start, source-bound submission, activity, shutdown, and requested
completion without BravePI or RPi-local types.

No production adapter crate or runtime plugin surface will be added. The
reference remains outside the production catalog.

The English and Japanese adapter READMEs and normative Input Adapter contracts
will state the current integration work exactly:

- add the focused crate to the root workspace;
- add its dependency to `iotkit-edge-node`;
- extend the central, closed `RawInputAdapterInstance` schema;
- add one private factory and catalog entry in the Edge Node composition root;
- update layer classification and the architecture map;
- add package, testkit, catalog/config, and conformance tests;
- run the exact focused commands.

This issue documents the current central schema edit; it does not redesign input
configuration as opaque adapter-owned data.

## Verification

Verification covers the new Node guard, the input testkit, Output Adapter
example/testkit/registry tests, bilingual OKF validation against the branch
base, layer rules, and source-layout rules. Both paired OKF documents increment
their revisions together.
