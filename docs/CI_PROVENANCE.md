# Private validation workflow policy

No hosted workflow has been executed or authorized by this local work. The owner must first verify remote repository visibility, collaborators, runner trust and artifact access. A Git origin does not establish any of those properties.

Every job requires BOTH:

- `github.event.repository.private == true`
- Repository variable `KORTEX_PRIVATE_VALIDATION_APPROVED` explicitly set to `true` by the owner.

Do not set that variable automatically. It permits the configured private hosted validation and private diagnostic artifacts, not public publication or third-party use. The packaging tool still unconditionally refuses non-development/public archives, regardless of legacy release approval variables. Checkout credentials are not persisted after checkout; workflow token permissions are read-only.

## Pinned external actions

These commit IDs were resolved locally from official public GitHub reference metadata. The network requests concerned only the named public upstream actions; no Kortex source or artifact was uploaded. `target/actions-provenance.json` records the retrieval timestamp and reference URLs. Commit pinning is not a signature verification, independent source audit, hosted execution result or legal/provenance approval.

| Action | Reviewed reference | Pinned commit |
|---|---|---|
| actions/checkout | v6 | d23441a48e516b6c34aea4fa41551a30e30af803 |
| dtolnay/rust-toolchain | stable | b725eda525362e7cf34af9a7243cac1e3f01a7ec |
| actions/setup-python | v6 | ece7cb06caefa5fff74198d8649806c4678c61a1 |
| actions/upload-artifact | v6 | b7c566a772e6b6bfb58ed0dc250532a479d7789f |
| taiki-e/install-action | v2 | cfdb7ed4ccc7c7dce37759eae89dc5674b469b39 |
| actions/setup-node | v6 | 249970729cb0ef3589644e2896645e5dc5ba9c38 |

Installed validation tools are pinned to Rust 1.99.1, cargo-deny 0.20.2 and cargo-llvm-cov 0.9.1. The explorer uses Node 24 for building/testing, not as a requirement for the native CLI. Cargo tests/clippy/audit include all first-party features and use the lockfile. Frontend dependency installs disable lifecycle scripts; browser installation is an explicit step in the owner-gated explorer job.

## What configuration does not prove

- Linux/Windows/macOS runner success and architecture/install coverage still require actual authorized runs.
- The current local production-wave dependency audit has failed; a configured audit step is not evidence of remediation.
- Benchmark orchestration unit tests use mocked commands and are not application performance evidence. The tiny real CLI self-test is also not reference-scale acceptance.
- Explorer build/browser jobs do not establish production readiness until the real API-backed tests pass and their outputs are reviewed.
- Coverage numbers from the previous accepted development baseline are not measurements of the current production wave.

`packaging/test_private_policy.py` guards the private-job approval conditions, external action commit pins, restrictive first-party license metadata, disabled publishing and unconditional public-packaging refusal. These static regressions do not inspect or change actual GitHub access controls.
