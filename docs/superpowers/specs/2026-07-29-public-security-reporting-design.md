# Public Security Reporting Design

Date: 2026-07-29
Issue: [#111](https://github.com/w-pinkietech/iotkit/issues/111)
Status: approved

## 1. Purpose

IoTKit is a public repository with a published `v0.2.0` source release. GitHub
Private Vulnerability Reporting is enabled, but `SECURITY.md` still describes
the repository as private and says that public release is blocked until the
reporting route is enabled.

This change makes the English and Japanese security guidance match the current
repository state and gives reporters one verified private route. It does not
publish or discuss any vulnerability.

## 2. Scope

Update the two language sections in the existing top-level `SECURITY.md`.
Both sections provide the same policy:

- report suspected vulnerabilities through
  `https://github.com/w-pinkietech/iotkit/security/advisories/new`;
- do not put vulnerability details, credentials, keys, tokens, customer
  information, network or device identifiers, raw MQTT data, databases,
  configuration, or sensitive screenshots in an Issue or Pull Request;
- do not send sensitive values through an unrelated public or private channel;
- if sensitive data was posted accidentally, revoke or rotate it, avoid
  repeating it, contact the maintainer with only the URL, and preserve only
  redacted evidence.

The policy may link to GitHub's official
[private vulnerability reporting guidance](https://docs.github.com/code-security/security-advisories/guidance-on-reporting-and-writing/privately-reporting-a-security-vulnerability/)
for platform behavior. The repository-specific reporting URL remains the
primary action.

## 3. Boundaries

This issue does not define supported product versions, response-time promises,
coordinated disclosure timelines, severity policy, or upgrade support. Those
public-distribution commitments remain in #95.

This issue does not update dependencies or describe dependency advisories.
Dependency findings remain a separate non-public follow-up until the repository
policy provides the intended confidential route.

No product source, runtime behavior, wire contract, database, release version,
GitHub security setting, or release artifact changes.

## 4. Authority and failure handling

The policy states only capabilities verified at implementation time:

1. the repository is public;
2. GitHub reports Private Vulnerability Reporting as enabled;
3. the repository advisory page exposes the private reporting action.

If any check fails, implementation stops rather than publishing a broken or
misleading URL. The change does not enable, disable, or otherwise mutate GitHub
security settings.

The English and Japanese sections must remain equivalent. Neither language may
offer a fallback that asks a reporter to disclose vulnerability details in a
public Issue.

## 5. Verification

Verification is documentation-focused:

- query the GitHub repository and confirm it is public;
- query the Private Vulnerability Reporting setting and confirm it is enabled;
- open the repository security-advisory reporting route and confirm the
  reporting action is available;
- inspect both language sections for equivalent reporting and accidental
  disclosure instructions;
- confirm the removed private-repository and pre-public-release statements no
  longer appear;
- run `git diff --check`;
- run the repository documentation and structure checks selected for the
  changed paths.

No Rust product test is required because the change is limited to security
policy text and external-link verification.

## 6. Delivery

Work uses branch `agent/issue-111-security-reporting` and closes #111 through a
draft Pull Request. The PR must not contain vulnerability details or dependency
advisory evidence.
