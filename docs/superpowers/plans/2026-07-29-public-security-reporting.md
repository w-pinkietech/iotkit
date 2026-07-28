# Public Security Reporting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the obsolete private-repository instructions in `SECURITY.md` with an English/Japanese policy that directs vulnerability reports to IoTKit's verified GitHub private reporting route.

**Architecture:** This is a documentation-only change to the repository-wide security policy. The implementation first verifies the external GitHub state on which the wording depends, then replaces both language sections together and runs exact content, documentation, and whitespace checks before delivery.

**Tech Stack:** Markdown, PowerShell, GitHub CLI, Git

## Global Constraints

- The primary private reporting URL is exactly `https://github.com/w-pinkietech/iotkit/security/advisories/new`.
- English and Japanese instructions must remain equivalent.
- Vulnerability details and sensitive values must never be placed in an Issue or Pull Request.
- The change must not define supported versions, response-time promises, disclosure timelines, severity policy, or upgrade support.
- The change must not update dependencies, product source, runtime behavior, contracts, databases, releases, GitHub settings, or release artifacts.
- If the repository is not public, Private Vulnerability Reporting is not enabled, or the reporting action is unavailable, stop without publishing the policy change.
- The Pull Request must not contain vulnerability details or dependency advisory evidence.

---

### Task 1: Publish the verified bilingual security reporting policy

**Files:**
- Modify: `SECURITY.md`

**Interfaces:**
- Consumes: GitHub repository visibility, Private Vulnerability Reporting state, and the browser-visible reporting action for `w-pinkietech/iotkit`
- Produces: A bilingual top-level policy whose primary action is the verified repository-specific private reporting URL

- [ ] **Step 1: Verify the external reporting prerequisites without changing them**

Run:

```powershell
gh repo view w-pinkietech/iotkit --json visibility --jq '.visibility'
gh api repos/w-pinkietech/iotkit/private-vulnerability-reporting --jq '.enabled'
```

Expected:

```text
PUBLIC
true
```

Open `https://github.com/w-pinkietech/iotkit/security/advisories/new` in the signed-in in-app browser.

Expected: GitHub displays the private vulnerability report form or its `Report a vulnerability` action for `w-pinkietech/iotkit`. If GitHub displays a missing, disabled, or unauthorized route instead, stop this task and report the failed prerequisite.

- [ ] **Step 2: Run the acceptance check and confirm the current policy fails it**

Run:

```powershell
$policyText = Get-Content -Raw 'SECURITY.md'
if ($policyText -notmatch [regex]::Escape('https://github.com/w-pinkietech/iotkit/security/advisories/new')) { throw 'private reporting URL is missing' }
if ($policyText -match 'currently a private development repository|現在、このrepositoryは非公開') { throw 'obsolete private-repository guidance remains' }
if ($policyText -match 'Public release is blocked|公開releaseを行いません') { throw 'obsolete release-blocking guidance remains' }
```

Expected: non-zero exit with `private reporting URL is missing`.

- [ ] **Step 3: Replace `SECURITY.md` with the verified English and Japanese policy**

Use this exact content:

```markdown
# Security reporting

If you believe you have found a vulnerability in IoTKit, use GitHub's
[private vulnerability reporting form](https://github.com/w-pinkietech/iotkit/security/advisories/new).
GitHub explains this process in its
[private vulnerability reporting guidance](https://docs.github.com/code-security/security-advisories/guidance-on-reporting-and-writing/privately-reporting-a-security-vulnerability/).

Do not report suspected vulnerabilities, credentials, keys, tokens, customer
information, network identifiers, device identifiers, raw MQTT data, databases,
configuration, or sensitive screenshots in a GitHub Issue or Pull Request. Do
not send sensitive values through an unrelated public or private channel.

If sensitive data was posted accidentally:

1. revoke or rotate the exposed credential immediately;
2. do not rely on editing or deleting the post to remove it from history;
3. contact the repository maintainer privately with the URL, without copying the
   secret or vulnerability details again;
4. preserve only redacted evidence needed to investigate the product behavior.

## 日本語

IoTKitの脆弱性を発見した可能性がある場合は、GitHubの
[非公開脆弱性報告フォーム](https://github.com/w-pinkietech/iotkit/security/advisories/new)
を使用してください。この仕組みについては、GitHubの
[非公開脆弱性報告ガイド](https://docs.github.com/code-security/security-advisories/guidance-on-reporting-and-writing/privately-reporting-a-security-vulnerability/)
も参照できます。

脆弱性の疑い、credential、鍵、token、顧客情報、network識別情報、device識別情報、
生MQTT data、DB、設定、機密を含むscreenshotをGitHub IssueやPull Requestへ書かないで
ください。無関係な公開・非公開の連絡経路で機密値を送らないでください。

機密情報を誤って投稿した場合は、次の順序で対処します。

1. 公開したcredentialを直ちに失効またはrotationする。
2. 投稿の編集・削除だけで履歴から消えたと判断しない。
3. 秘密や脆弱性の詳細を再掲せず、投稿URLだけをrepository maintainerへ非公開で伝える。
4. 製品動作の調査に必要な秘匿化済み証拠だけを残す。
```

- [ ] **Step 4: Run the acceptance check and confirm the new policy passes it**

Run:

```powershell
$policyText = Get-Content -Raw 'SECURITY.md'
if ($policyText -notmatch [regex]::Escape('https://github.com/w-pinkietech/iotkit/security/advisories/new')) { throw 'private reporting URL is missing' }
if ($policyText -match 'currently a private development repository|現在、このrepositoryは非公開') { throw 'obsolete private-repository guidance remains' }
if ($policyText -match 'Public release is blocked|公開releaseを行いません') { throw 'obsolete release-blocking guidance remains' }
```

Expected: exit code 0 with no output.

- [ ] **Step 5: Inspect the bilingual diff for policy equivalence and scope**

Run:

```powershell
git diff -- SECURITY.md
```

Expected:

- both sections use the same repository-specific private reporting route;
- both prohibit disclosure through Issues, Pull Requests, and unrelated channels;
- both retain equivalent accidental-disclosure steps;
- no supported-version, response-time, disclosure-timeline, severity, dependency-advisory, or product-change claim is introduced.

- [ ] **Step 6: Run focused repository verification**

Run:

```powershell
node scripts/check-okf-docs.mjs
git diff --check
git status --short
```

Expected:

- `check-okf-docs.mjs`: `OKF docs check passed.`
- `git diff --check`: exit code 0 with no output
- `git status --short`: only `SECURITY.md` is modified, apart from already committed design and plan history

- [ ] **Step 7: Run the battle-tested review selector required by repository policy**

Run:

```powershell
node scripts/battle-tested-review.mjs select --base origin/master
```

Expected: Record the selector result and review every selected `BT-NNN` entry. Zero selections are acceptable only after independently confirming that this documentation-only change does not violate the security invariant or the approved design.

- [ ] **Step 8: Commit the policy update**

Run:

```powershell
git add -- SECURITY.md
git commit -m "docs: publish private security reporting route"
```

Expected: one commit containing only `SECURITY.md`.

- [ ] **Step 9: Push the branch and open a draft Pull Request**

Push `agent/issue-111-security-reporting`, then open a draft Pull Request targeting `master`. The PR body must summarize the stale-policy correction, list the verification commands and reporting-route checks, and include `Closes #111`. It must not mention vulnerability details or dependency advisory evidence.

Expected: a draft Pull Request for `agent/issue-111-security-reporting` exists on `w-pinkietech/iotkit` and remains unmerged for human review.
